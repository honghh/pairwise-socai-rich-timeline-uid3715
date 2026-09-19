# TikTok research workflow

- Use `searchState` before accepting `videoCards`; a login, CAPTCHA, rate-limit,
  query mismatch, or unhydrated page is not an empty result.
- Open selected `/@<author>/video/<video_id>` pages and call `videoDetail` before
  comments. If the comment panel is closed, use `commentActivation`, then call
  `comments`; the generic host automatically scrolls, expands collapsed replies,
  deduplicates, and returns the accumulated set up to `limit` (100 by default).
  Native `get_videos --num-comments` performs the
  same bounded loop for CLI/agent callers (up to 100 comments).
- Preserve the canonical video URL, creator identity, media candidates, counts,
  and returned comments. Never like, follow, reply, comment, or send messages.
- `videoCards` automatically collects lazy-scroll cards up to `limit`.
  `videoCards`, `videoDetail`, accumulated comments, native search cards, and `get_videos`
  results are archived as desktop post cards and downloadable JSON artifacts.
  Cite a saved post with archive id `tiktok:<video_id>` when `note:` citations
  are requested by the host.
