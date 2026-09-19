# Douyin site notes

- Site id: `dy`; home URL: `https://www.douyin.com/`.
- Douyin web may throttle by keeping the page visually blank for 4-5 minutes.
  Commands therefore use long waits by default and report
  `blank_or_throttled` instead of treating a blank page as an immediate hard
  failure.
- Current first-step tool: `page_state`. Use it with `--debug-snapshot` to
  verify whether the homepage/search UI is visible before implementing or
  relying on deeper workflows.
- `search` starts from the homepage/top search box, enters the keyword,
  submits with Enter, then extracts cards from the search-result waterfall.
  Use `--num` to scroll for more cards; default is 10.
- Observed stable-ish selectors on 2026-06-11:
  `data-e2e="searchbar-input"`, `data-e2e="searchbar-button"`,
  `.search-result-card`, parent ids like `waterfall_item_<video_id>`,
  `.videoImage`, `.RBpYLmIg` for title text, `.lGzJpEad` for author, and
  `.GiEcbsyC span` for visible like/play count text.
- Search-result "综合" includes non-video modules such as live rooms and topic
  cards. The extractor filters obvious live cards and keeps cards with video
  signals; fields absent from the search card, such as comments/shares, are
  returned as empty strings.
- For a selected work, call `videoDetail` before `comments`. The generic host
  automatically scrolls the comment panel, expands collapsed replies,
  deduplicates, and returns the accumulated set up to `limit` (100 by default).
  Native `get_videos --num-comments` performs the same bounded loop for
  CLI/agent callers (up to 100 comments).
- `videoCards` automatically collects lazy-scroll cards up to `limit`.
  `videoCards`, `videoDetail`, accumulated comments, native search cards, and `get_videos`
  results are archived as desktop post cards and downloadable JSON artifacts.
  Cite a saved work with archive id `dy:<video_id>` when `note:` citations are
  requested by the host.
- When a tool explicitly returns `recovery.action:"browser_script"`, follow the
  system-level local browser self-repair protocol, save a verified override for
  that exact tool, retry it once, and continue the task. Do not use browser
  scripts to bypass login, captcha, security verification, rate limits,
  permissions, or a confirmed valid empty result.
