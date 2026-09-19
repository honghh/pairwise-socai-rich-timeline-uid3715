# Agent skills and self-healing

SocAI exposes focused procedural knowledge to the TUI and desktop agents as
Agent Skills. Skills use progressive disclosure: the system prompt contains a
compact name and description, while the full instruction is returned only when
the agent calls `read_skill`.

The initial catalog contains one bundled skill:

- `self-healing`: diagnose a failed or incomplete action, make the smallest
  safe recovery, verify the original outcome, and retain a reusable learning
  only after recovery succeeds.

The bundled instruction lives at
`core/src/agent/skills/self-healing/SKILL.md`. Its frontmatter follows the Agent
Skills `name` and `description` contract. The file is compiled into
`socai-core`, so retained local data cannot replace the canonical safety
instruction.

## Runtime contract

`local_agent_tools()` registers the skill tools for both interactive
entrypoints:

- `read_skill({ "name": "self-healing" })` returns the canonical instruction
  and the newest bounded local learnings. Calling it marks the skill as loaded
  for the current agent run.
- `record_skill_learning(...)` accepts a structured symptom, verified root
  cause, recovery, and validation. It is rejected unless `read_skill` loaded
  `self-healing` in the same run and the runtime independently observed the
  same non-skill tool fail and then succeed in a later step.

This keeps the always-present prompt small and makes failure recovery automatic
at the policy level: when a task matches the skill, or a tool/action fails or
returns incomplete data, the system prompt directs the agent to load the skill
before it attempts recovery.

## Local retention

Verified learnings are stored separately from the bundled instruction:

```text
$SOCAI_HOME/skills/self-healing/learnings.json
```

When `SOCAI_HOME` is unset, the default is
`~/.socai/skills/self-healing/learnings.json`. The JSON store is versioned,
written through a temporary file, protected by process-local and cross-process
locks, deduplicated, and bounded to 24 entries and 64 KiB. Unix uses atomic
rename and Windows uses `MoveFileExW` with replace/write-through flags, without
unlinking the last good store first. `read_skill` shows at most the newest eight
entries to limit context growth.

The write tool has no path argument and only recognizes the bundled
`self-healing` name. It rejects oversized fields, unsupported records, common
credential/session markers, and common prompt-control text. Files edited
outside SocAI are validated again before their contents are shown to the
agent. Local learnings are explicitly presented as advisory data and must be
revalidated against the current environment.

## Verification

Run the focused contract tests with:

```bash
cargo test -p socai-core agent::skills
```

The tests cover bundled metadata validation, progressive disclosure, TUI and
desktop shared-tool registration, read-before-write and verified-outcome gates,
runtime failure/recovery evidence, round-trip persistence, deduplication,
bounded eviction, external-file tamper handling, and unsafe-content rejection.
