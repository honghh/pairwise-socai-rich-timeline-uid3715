//! Typed tool-event rendering for the task activity timeline.
//!
//! The shell emits typed tool_call / tool_result / tool_error events whose
//! `entities` carry structured payloads (search cards, notes, profiles,
//! comments, images, aggregated search bundles). This module turns those
//! entities into the same note/media UI the conversation already uses, and
//! owns the stable call↔result correlation key shared by the live row
//! appender (tasks.ts) and the full-render grouping (conversation.ts) so a
//! streamed run and a replayed history render the same rows.
//!
//! Anything unrecognized — unknown entity types, malformed fields — degrades
//! to an expandable JSON block; a bad entity must never break the timeline.

import { esc } from "../lib/html";
import { t } from "../lib/i18n";
import type { AgentTaskEventPayload, NoteData, NoteMedia, TimelineEntity } from "../main";
import { mergeNoteRegistry, renderNoteCards } from "./notes";

// Internal page-driving operations: shown, but de-emphasized — they describe
// navigation, not findings.
const QUIET_TOOLS = new Set([
  "page_state",
  "scroll_in_note",
  "open_note",
  "close_note",
  "list_search_tabs",
  "click_search_tab",
  "reset_search_filters",
  "apply_search_filters",
  "wait_for_rate_limit",
  "wait_for_login",
]);

export function isQuietTool(name: string | undefined): boolean {
  return QUIET_TOOLS.has(name ?? "");
}

/** Stable correlation key joining a tool call with its result. Live events
 *  carry provider ids; replayed runs without one get a derived id from the
 *  shell (run/step/in-step sequence/tool name). The fallback here mirrors
 *  that derivation for events that arrive with no id at all. */
export function toolEventKey(ev: AgentTaskEventPayload): string {
  const id = (ev.id ?? "").trim();
  if (id) return id;
  return `s${ev.step ?? 0}:c${ev.sequence_in_step ?? 0}:${ev.name ?? "tool"}`;
}

// ── entity → note registry ──────────────────────────────────────────

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function asRecord(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function str(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

// XHS displays counts as text ("1.2万", "3k"); the registry wants numbers.
function parseCount(raw: unknown): number | undefined {
  const text = str(raw).toLowerCase().replace(/[,+]/g, "");
  if (!text) return undefined;
  const match = text.match(/(\d+(?:\.\d+)?)(万|w|k)?/);
  if (!match) return undefined;
  let value = parseFloat(match[1]);
  if (match[2] === "万" || match[2] === "w") value *= 10_000;
  else if (match[2] === "k") value *= 1_000;
  return Math.round(value);
}

function parsePostedAt(raw: unknown): number | undefined {
  const text = str(raw);
  const match = /^(\d{4})-(\d{1,2})-(\d{1,2})/.exec(text);
  if (!match) return undefined;
  const ms = Date.parse(`${match[1]}-${match[2].padStart(2, "0")}-${match[3].padStart(2, "0")}T00:00:00`);
  return Number.isFinite(ms) ? ms : undefined;
}

function mediaFromXhsImages(images: unknown): NoteMedia[] {
  const media: NoteMedia[] = [];
  for (const image of asArray(images)) {
    const record = asRecord(image);
    const src = str(record.local_path) || str(record.url);
    if (!src) continue;
    media.push({ kind: "image", src });
  }
  return media;
}

function mediaFromXhsVideo(video: unknown): NoteMedia | null {
  const record = asRecord(video);
  const src = str(record.local_path) || str(record.url);
  const poster = str(record.poster) || str(record.poster_url);
  if (!src && !poster) return null;
  return { kind: "video", src: src || undefined, poster: poster || undefined };
}

/** XhsNote wire entity (notes read in full) → registry NoteData. */
function noteFromXhsEntity(value: unknown): NoteData | null {
  const record = asRecord(value);
  const noteId = str(record.note_id);
  if (!noteId) return null;
  const media = mediaFromXhsImages(record.images);
  const video = mediaFromXhsVideo(record.video);
  if (video) media.unshift(video);
  const content = str(record.content);
  return {
    note_id: noteId,
    site: "xhs",
    url: str(record.url) || undefined,
    title: str(record.title) || undefined,
    content: content || undefined,
    excerpt: content ? content.slice(0, 90) : undefined,
    author: {
      name: str(record.author) || undefined,
      url: str(record.author_url) || undefined,
    },
    posted_at: parsePostedAt(record.date),
    ip_location: str(record.ip_location) || undefined,
    stats: {
      likes: parseCount(record.likes),
      collects: parseCount(record.favorites),
      comments: parseCount(record.comments_count),
    },
    media: media.length ? media : undefined,
  };
}

/** XhsNoteCard wire entity (search/profile cards) → registry NoteData. */
function noteFromXhsCard(value: unknown): NoteData | null {
  const record = asRecord(value);
  const noteId = str(record.note_id);
  if (!noteId) return null;
  const cover = str(record.cover_url);
  return {
    note_id: noteId,
    site: "xhs",
    url: str(record.link) || undefined,
    title: str(record.title) || undefined,
    author: {
      name: str(record.author) || undefined,
      url: str(record.author_url) || undefined,
    },
    stats: { likes: parseCount(record.likes) },
    media: cover ? [{ kind: "image", src: cover }] : undefined,
  };
}

function isRegistryNote(value: unknown): value is NoteData {
  const record = asRecord(value);
  return typeof record.note_id === "string" && !!record.note_id && typeof record.site === "string";
}

function pushNote(out: NoteData[], note: NoteData | null): void {
  if (note && !out.some((existing) => existing.note_id === note.note_id)) out.push(note);
}

function collectEntityNotes(entity: TimelineEntity, out: NoteData[]): void {
  const data = entity?.data;
  switch (entity?.type) {
    case "xhs_note_card_grid":
      for (const card of asArray(data)) pushNote(out, noteFromXhsCard(card));
      break;
    case "xhs_search":
      for (const entry of asArray(asRecord(data).notes)) {
        pushNote(out, noteFromXhsEntity(asRecord(entry).entity ?? entry));
      }
      break;
    case "xhs_note":
      pushNote(out, noteFromXhsEntity(data));
      break;
    case "xhs_author_profile":
      for (const card of asArray(asRecord(data).note_cards)) pushNote(out, noteFromXhsCard(card));
      break;
    default:
      // social_post_grid and legacy shapes carry registry-ready records.
      for (const note of asArray(asRecord(data).notes)) {
        if (isRegistryNote(note)) pushNote(out, note);
      }
  }
}

/** Merge every note an event list's entities carry into the note registry,
 *  so cards/citations in this same render pass resolve. Tolerates malformed
 *  entities — one bad payload never stops the rest. */
export function ingestTimelineEntities(events: AgentTaskEventPayload[]): void {
  const notes: NoteData[] = [];
  for (const event of events) {
    for (const entity of event.entities ?? []) {
      try {
        collectEntityNotes(entity, notes);
      } catch {
        // malformed entity — skipped, the row falls back to raw JSON
      }
    }
  }
  if (notes.length) mergeNoteRegistry(notes);
}

// ── entity → rich html ──────────────────────────────────────────────

const JSON_FALLBACK_MAX_CHARS = 4_000;
const COMMENTS_SHOWN = 10;

function renderJsonFallback(entity: TimelineEntity): string {
  let json: string;
  try {
    json = JSON.stringify(entity?.data ?? null, null, 2);
  } catch {
    json = String(entity?.data ?? "");
  }
  if (json.length > JSON_FALLBACK_MAX_CHARS) json = `${json.slice(0, JSON_FALLBACK_MAX_CHARS)}\n…`;
  const type = typeof entity?.type === "string" && entity.type ? entity.type : "data";
  return `<details class="act-json"><summary>${esc(type)} · ${esc(t("task.rawData"))}</summary><pre>${esc(json)}</pre></details>`;
}

function renderCardGroup(tool: string, context: string, refs: string[]): string {
  if (!refs.length) return "";
  const cards = renderNoteCards(refs, "rich");
  if (!cards) return "";
  const label = context
    ? `<span class="search-group__tool">${esc(tool)}</span><span class="search-group__q">${esc(context)}</span>`
    : `<span class="search-group__tool">${esc(tool)}</span>`;
  return `<div class="search-group"><div class="search-group__label">${label}</div><div class="search-group__row">${cards}</div></div>`;
}

function renderNoteGridEntity(entity: TimelineEntity, ev: AgentTaskEventPayload): string {
  const cards = asArray(entity.data)
    .map(noteFromXhsCard)
    .filter((note): note is NoteData => !!note);
  mergeNoteRegistry(cards);
  return renderCardGroup(ev.name ?? "cards", "", cards.map((note) => note.note_id));
}

function renderSearchEntity(entity: TimelineEntity, ev: AgentTaskEventPayload): string {
  const data = asRecord(entity.data);
  const notes = asArray(data.notes)
    .map((entry) => noteFromXhsEntity(asRecord(entry).entity ?? entry))
    .filter((note): note is NoteData => !!note);
  mergeNoteRegistry(notes);
  const query = str(data.query) || str(asRecord(ev.args).query);
  return renderCardGroup(ev.name ?? "search", query, notes.map((note) => note.note_id));
}

function renderProfileEntity(entity: TimelineEntity): string {
  const data = asRecord(entity.data);
  const name = str(data.display_name) || str(data.title);
  const xhsId = str(data.xhs_id);
  const bio = str(data.bio);
  const ip = str(data.ip_location);
  if (!name && !xhsId && !bio) return "";
  const stats = t("task.profileStats", {
    followers: str(data.followers) || "—",
    following: str(data.following) || "—",
    likes: str(data.likes_and_collections) || "—",
  });
  const cards = asArray(data.note_cards)
    .map(noteFromXhsCard)
    .filter((note): note is NoteData => !!note);
  mergeNoteRegistry(cards);
  const grid = renderCardGroup("notes", "", cards.map((note) => note.note_id));
  const meta = [xhsId ? `@${xhsId}` : "", stats, ip].filter(Boolean).join(" · ");
  return `<div class="tl-profile">
    <div class="tl-profile__head">
      <span class="tl-profile__avatar" aria-hidden="true">${esc(Array.from(name || "·")[0])}</span>
      <span class="tl-profile__id">
        <span class="tl-profile__name">${esc(name)}</span>
        <span class="tl-profile__meta">${esc(meta)}</span>
      </span>
    </div>
    ${bio ? `<p class="tl-profile__bio">${esc(bio)}</p>` : ""}
    ${grid}
  </div>`;
}

function renderCommentsEntity(entity: TimelineEntity): string {
  const comments = asArray(entity.data);
  if (!comments.length) return "";
  const renderOne = (value: unknown, isReply: boolean): string => {
    const comment = asRecord(value);
    const author = str(comment.username) || str(comment.author) || "·";
    const text = str(comment.text) || str(comment.content);
    if (!text) return "";
    const likes = typeof comment.like_count === "number" && comment.like_count > 0
      ? `♥ ${comment.like_count}`
      : str(comment.likes);
    const meta = [str(comment.time), likes].filter(Boolean).join(" · ");
    const replies = asArray(comment.sub_comments)
      .map((reply) => renderOne(reply, true))
      .filter(Boolean)
      .join("");
    return `<div class="tl-comment${isReply ? " tl-comment--reply" : ""}">
      <span class="tl-comment__author">${esc(author)}</span>
      <p class="tl-comment__text">${esc(text)}</p>
      ${meta ? `<span class="tl-comment__meta">${esc(meta)}</span>` : ""}
      ${replies ? `<div class="tl-comment__replies">${replies}</div>` : ""}
    </div>`;
  };
  const shown = comments.slice(0, COMMENTS_SHOWN).map((comment) => renderOne(comment, false)).filter(Boolean);
  if (!shown.length) return "";
  const more = comments.length > COMMENTS_SHOWN
    ? `<div class="tl-more">${esc(t("task.moreItems", { n: comments.length - COMMENTS_SHOWN }))}</div>`
    : "";
  return `<div class="tl-block"><span class="tl-block__label">${esc(t("task.commentsLabel"))}</span><div class="tl-comments">${shown.join("")}${more}</div></div>`;
}

function renderImageStripEntity(entity: TimelineEntity): string {
  const urls = asArray(entity.data)
    .map((url) => (typeof url === "string" ? url.trim() : ""))
    .filter(Boolean);
  if (!urls.length) return "";
  const images = urls
    .map((url) => `<img class="tl-images__img" src="${esc(url)}" alt="" loading="lazy" referrerpolicy="no-referrer" />`)
    .join("");
  return `<div class="tl-images">${images}</div>`;
}

function renderEntity(entity: TimelineEntity, ev: AgentTaskEventPayload): string {
  try {
    switch (entity?.type) {
      case "xhs_note_card_grid":
        return renderNoteGridEntity(entity, ev) || renderJsonFallback(entity);
      case "xhs_search":
        return renderSearchEntity(entity, ev) || renderJsonFallback(entity);
      case "xhs_author_profile":
        return renderProfileEntity(entity) || renderJsonFallback(entity);
      case "xhs_comments":
        return renderCommentsEntity(entity) || renderJsonFallback(entity);
      case "xhs_image_strip":
        return renderImageStripEntity(entity) || renderJsonFallback(entity);
      case "social_post_grid": {
        const notes = asArray(asRecord(entity.data).notes).filter(isRegistryNote);
        mergeNoteRegistry(notes);
        const html = renderCardGroup(ev.name ?? "posts", "", notes.map((note) => note.note_id));
        return html || renderJsonFallback(entity);
      }
      default:
        return renderJsonFallback(entity);
    }
  } catch {
    return renderJsonFallback(entity);
  }
}

/** Rich result body for a finished tool row ("" when the tool surfaced no
 *  entities — lightweight tools stay a plain label row). */
export function renderToolEntities(ev: AgentTaskEventPayload): string {
  const entities = ev.entities ?? [];
  if (!entities.length) return "";
  const blocks: string[] = [];
  // Notes from one call (get_notes reads several) render as a single card
  // row, not one stacked group per note.
  let noteRefs: string[] = [];
  const flushNotes = (): void => {
    if (!noteRefs.length) return;
    const group = renderCardGroup("note", "", noteRefs);
    if (group) blocks.push(group);
    noteRefs = [];
  };
  for (const entity of entities) {
    if (entity?.type === "xhs_note") {
      try {
        const note = noteFromXhsEntity(entity.data);
        if (note) {
          mergeNoteRegistry([note]);
          if (!noteRefs.includes(note.note_id)) noteRefs.push(note.note_id);
          continue;
        }
      } catch {
        // falls through to the JSON fallback below
      }
      flushNotes();
      blocks.push(renderJsonFallback(entity));
      continue;
    }
    flushNotes();
    blocks.push(renderEntity(entity, ev));
  }
  flushNotes();
  const html = blocks.filter(Boolean).join("");
  return html ? `<div class="act-tool__result">${html}</div>` : "";
}
