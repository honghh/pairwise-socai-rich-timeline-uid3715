use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncReadExt;

use super::auth::{bearer, configured_base_url, http_client, load_credentials};

const TASK_POLL_INTERVAL: Duration = Duration::from_secs(1);
const AUDIO_UPLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_AUDIO_UPLOAD_BYTES: u64 = 128 * 1024 * 1024;
/// Consecutive poll failures tolerated before giving up on a submitted task.
const MAX_POLL_FAILURES: u32 = 3;

#[derive(Debug)]
struct CloudAsrAccessRejected(reqwest::StatusCode);

impl std::fmt::Display for CloudAsrAccessRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "cloud ASR access was rejected before upload ({})",
            self.0
        )
    }
}

impl std::error::Error for CloudAsrAccessRejected {}

/// True only when the server rejected authorization before any audio upload or
/// provider submission. Callers may safely retry these requests with local ASR
/// without risking a duplicate cloud transcription or charge.
pub fn cloud_asr_access_rejected(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<CloudAsrAccessRejected>().is_some())
}

#[derive(Debug, Deserialize)]
struct UploadUrlResponse {
    task_id: String,
    upload_url: String,
    headers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct TaskResponse {
    status: String,
    transcript: Option<String>,
    error: Option<String>,
    provider_latency_ms: i64,
}

#[derive(Debug, Clone)]
pub struct CloudAsrResult {
    pub transcript: String,
    pub provider_latency_ms: i64,
    pub total_latency_ms: u128,
}

pub async fn transcribe_audio_file(
    path: &Path,
    duration_s: i64,
    timeout: Duration,
    client_task_id: Option<&str>,
) -> Result<CloudAsrResult> {
    let started = Instant::now();
    let deadline = started
        .checked_add(timeout)
        .context("cloud ASR timeout is too large")?;
    let base_url = configured_base_url()
        .ok_or_else(|| anyhow::anyhow!("socai server URL is not configured"))?;
    let creds = load_credentials()
        .ok_or_else(|| anyhow::anyhow!("sign in before requesting cloud transcription"))?;
    if creds.user_id.trim().is_empty() || creds.device_token.trim().is_empty() {
        anyhow::bail!("sign in before requesting cloud transcription");
    }
    // The extension matters: the server passes the filename through to
    // DashScope, which detects the audio format from it.
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("audio.aac");
    let content_type = audio_content_type(path);
    let size_bytes = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("failed to read metadata for {}", path.display()))?
        .len();
    if size_bytes > MAX_AUDIO_UPLOAD_BYTES {
        anyhow::bail!(
            "audio upload is too large: {} bytes exceeds the 128 MiB limit",
            size_bytes
        );
    }
    let client = http_client()?;
    let upload_url_timeout = remaining_before(deadline, "requesting an upload URL")?;
    let upload_response = bearer(
        client
            .post(format!("{base_url}/v1/asr/upload-url"))
            .timeout(upload_url_timeout)
            .json(&json!({
                "filename": filename,
                "content_type": content_type,
                "size_bytes": size_bytes,
                "duration_s": duration_s.max(0),
                "client_task_id": client_task_id.unwrap_or(""),
            })),
        &creds.device_token,
    )
    .send()
    .await?;
    if matches!(upload_response.status().as_u16(), 401 | 402 | 403) {
        return Err(CloudAsrAccessRejected(upload_response.status()).into());
    }
    let upload: UploadUrlResponse = tokio::time::timeout(
        remaining_before(deadline, "reading the upload authorization")?,
        upload_response.error_for_status()?.json(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("cloud ASR timed out while reading upload authorization"))??;

    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("failed to open {} for upload", path.display()))?;
    let stream = futures::stream::try_unfold(file, |mut file| async move {
        let mut chunk = vec![0u8; 64 * 1024];
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        chunk.truncate(read);
        Ok(Some((chunk, file)))
    });
    let upload_timeout = AUDIO_UPLOAD_TIMEOUT.min(remaining_before(deadline, "uploading audio")?);
    let mut put = client
        .put(&upload.upload_url)
        .timeout(upload_timeout)
        .header(reqwest::header::CONTENT_LENGTH, size_bytes)
        .body(reqwest::Body::wrap_stream(stream));
    for (key, value) in &upload.headers {
        put = put.header(key, value);
    }
    put.send().await?.error_for_status()?;

    let submit_timeout = remaining_before(deadline, "submitting the transcription task")?;
    bearer(
        client
            .post(format!("{base_url}/v1/asr/tasks/{}/submit", upload.task_id))
            .timeout(submit_timeout),
        &creds.device_token,
    )
    .send()
    .await?
    .error_for_status()?;

    let mut poll_failures: u32 = 0;
    loop {
        let poll_timeout = remaining_before(deadline, "waiting for transcription")?;
        // The upload is already done at this point; tolerate a few transient
        // poll errors (5xx, network blips) instead of abandoning the task.
        match poll_task(
            &client,
            &base_url,
            &upload.task_id,
            &creds.device_token,
            poll_timeout,
        )
        .await
        {
            Ok(task) => {
                poll_failures = 0;
                match task.status.as_str() {
                    "succeeded" => {
                        return Ok(CloudAsrResult {
                            transcript: task.transcript.unwrap_or_default(),
                            provider_latency_ms: task.provider_latency_ms,
                            total_latency_ms: started.elapsed().as_millis(),
                        });
                    }
                    "failed" => {
                        let error = task.error.unwrap_or_else(|| "unknown error".into());
                        // Fun-ASR reports an audio track it decoded fine but
                        // heard no speech in (music/SFX-only videos) as a
                        // failure. Surface the meaning, not the provider blob.
                        if error.contains("ASR_RESPONSE_HAVE_NO_WORDS") {
                            anyhow::bail!(
                                "no speech detected in the video's audio track (it likely \
                                 contains only music or sound effects), so there is no \
                                 spoken content to transcribe"
                            );
                        }
                        anyhow::bail!("video transcription failed: {}", short_error(&error));
                    }
                    _ => {}
                }
            }
            Err(err) => {
                poll_failures += 1;
                if poll_failures >= MAX_POLL_FAILURES {
                    return Err(err.context("video transcription status polling failed"));
                }
            }
        }
        let remaining = remaining_before(deadline, "waiting for transcription")?;
        tokio::time::sleep(TASK_POLL_INTERVAL.min(remaining)).await;
    }
}

fn audio_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("ogg") | Some("opus") => "audio/ogg",
        _ => "audio/aac",
    }
}

async fn poll_task(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    token: &str,
    timeout: Duration,
) -> Result<TaskResponse> {
    Ok(bearer(
        client
            .get(format!("{base_url}/v1/asr/tasks/{task_id}"))
            .timeout(timeout),
        token,
    )
    .send()
    .await?
    .error_for_status()?
    .json()
    .await?)
}

fn remaining_before(deadline: Instant, operation: &str) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| anyhow::anyhow!("cloud ASR timed out while {operation}"))
}

/// Trim a provider error to something an agent can read. Raw DashScope task
/// dumps run to kilobytes of nested JSON with signed URLs; the leading part
/// carries the task id + failure code, which is all a transcript_error needs.
fn short_error(error: &str) -> String {
    const MAX: usize = 300;
    let trimmed = error.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(MAX).collect();
    format!("{head}… (truncated)")
}

#[cfg(test)]
mod tests {
    use super::audio_content_type;
    use std::path::Path;

    #[test]
    fn upload_content_type_follows_audio_extension() {
        assert_eq!(audio_content_type(Path::new("voice.wav")), "audio/wav");
        assert_eq!(audio_content_type(Path::new("clip.aac")), "audio/aac");
        assert_eq!(audio_content_type(Path::new("speech.MP3")), "audio/mpeg");
    }
}
