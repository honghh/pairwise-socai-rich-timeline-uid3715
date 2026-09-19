//! Shared helpers for skill-backed CLI/daemon workflows.
//!
//! Instagram and LinkedIn keep page extraction in their site-skill packages.
//! These helpers only handle navigation, waiting, pagination, and archiving
//! so `socai <site> <command>` can call the same browser tools as the agent.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::tool::ToolProgressSender;
use crate::agent::Tool;
use crate::cdp::PageSession;
use crate::sites::learning::{run_site_browser_tool, run_site_browser_tool_collecting};
use crate::sites::post_archive::{persist_site_tool_result, save_site_media};
use crate::sites::registry::BoxFuture;
use crate::sites::runner::{run_tool_command, ToolCommand};

pub const MAX_TOOL_ITEMS: i64 = 100;
pub const MAX_TOOL_WAIT_SECONDS: f64 = 330.0;
pub const DEFAULT_WAIT_SECONDS: f64 = 30.0;
pub const DEFAULT_RESULT_COUNT: i64 = 10;
pub const DEFAULT_COMMENT_COUNT: i64 = 8;

pub fn run_skill_command(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
    progress: Option<ToolProgressSender>,
    command: ToolCommand<'static>,
    tools: Vec<Arc<dyn Tool>>,
) -> BoxFuture<Value> {
    Box::pin(async move {
        run_tool_command(command, page, &tools, args, debug_snapshot, progress).await
    })
}

pub fn percent_encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

pub fn hostname(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default()
}

pub fn host_matches(url: &str, root: &str) -> bool {
    let host = hostname(url);
    host == root || host.ends_with(&format!(".{root}"))
}

pub async fn current_url(page: &PageSession) -> Result<String> {
    Ok(page
        .page_info()
        .await?
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

pub async fn navigate_https(page: &PageSession, url: &str) -> Result<()> {
    page.navigate_with_timeout(url, 60.0).await
}

pub async fn ensure_site_page(page: &PageSession, root: &str, home_url: &str) -> Result<()> {
    let url = current_url(page).await.unwrap_or_default();
    if host_matches(&url, root) {
        return Ok(());
    }
    navigate_https(page, home_url).await
}

pub async fn invoke_browser_tool(
    page: &PageSession,
    ctx: &crate::agent::ToolContext,
    site_id: &str,
    tool_name: &str,
    args: Option<&Value>,
    collect: bool,
) -> Result<Value> {
    let mut result = if collect {
        run_site_browser_tool_collecting(page, site_id, tool_name, args).await?
    } else {
        run_site_browser_tool(page, site_id, tool_name, args).await?
    };
    save_site_media(page, ctx, site_id, tool_name, &mut result).await;
    let page_url = current_url(page).await.ok();
    persist_site_tool_result(ctx, site_id, tool_name, &result, page_url.as_deref());
    Ok(result)
}

pub async fn wait_for_browser_tool(
    page: &PageSession,
    site_id: &str,
    tool_name: &str,
    args: Option<&Value>,
    timeout_seconds: f64,
) -> Result<Value> {
    let deadline =
        Instant::now() + Duration::from_secs_f64(timeout_seconds.clamp(1.0, MAX_TOOL_WAIT_SECONDS));
    let mut latest = json!({ "ok": false, "reason": "waiting" });
    while Instant::now() < deadline {
        latest = run_site_browser_tool(page, site_id, tool_name, args).await?;
        if is_terminal_state(&latest) {
            return Ok(latest);
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    if latest.get("reason").and_then(Value::as_str).is_none()
        && latest.get("error").and_then(Value::as_str).is_none()
    {
        if let Some(object) = latest.as_object_mut() {
            object.insert("reason".into(), json!("timeout"));
        }
    }
    Ok(latest)
}

pub fn gate_reason(state: &Value) -> Option<&'static str> {
    for key in ["status", "error", "reason"] {
        if let Some(value) = state.get(key).and_then(Value::as_str) {
            if let Some(reason) = classified_gate(value) {
                return Some(reason);
            }
        }
    }
    if state.get("login_required").and_then(Value::as_bool) == Some(true) {
        return Some("login_required");
    }
    if state.get("challenge_required").and_then(Value::as_bool) == Some(true) {
        return Some("challenge_required");
    }
    if state.get("rate_limited").and_then(Value::as_bool) == Some(true) {
        return Some("rate_limited");
    }
    None
}

pub fn failure_payload(reason: &str, extra: Value) -> Value {
    let mut payload = extra;
    if let Some(object) = payload.as_object_mut() {
        object.insert("ok".into(), json!(false));
        object.entry("reason").or_insert_with(|| json!(reason));
    }
    payload
}

fn classified_gate(value: &str) -> Option<&'static str> {
    match value {
        "login_required" => Some("login_required"),
        "challenge_required" => Some("challenge_required"),
        "rate_limited" => Some("rate_limited"),
        _ => None,
    }
}

fn is_terminal_state(value: &Value) -> bool {
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if gate_reason(value).is_some() {
        return true;
    }
    matches!(
        value
            .get("status")
            .and_then(Value::as_str)
            .or_else(|| value.get("error").and_then(Value::as_str)),
        Some("empty")
    )
}
