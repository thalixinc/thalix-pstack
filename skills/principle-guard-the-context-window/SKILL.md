---
name: principle-guard-the-context-window
description: "Apply when context is filling up: large outputs, long files, repeated reads, fan-out planning. Route bulk to subagents; keep summaries in the main thread, not raw payloads."
---

# Guard the Context Window

## External action boundary

Local, reversible work inside the user-scoped workspace may proceed when the request authorizes the task. Before sending any external message or performing any state-changing action in an external service, obtain explicit user authorization for that exact class of action. This includes posting comments, opening or merging pull requests, updating tickets, starting deployments or remote jobs, sending email or chat messages, and changing hosted settings. Read-only external inspection is allowed when needed. A general autonomy instruction does not replace this authorization.

The context window is finite and non-renewable within a session. Every token should be worth its cost.

**Why:** Context overflow degrades reasoning quality, creates compression artifacts, and halts progress.

**Pattern:**
- **Isolate large payloads.** Route verbose outputs, screenshots, and large documents to subagents. The main context gets summaries, not raw data.
- **Don't read what you won't use.** Read selectively based on relevance. If a file isn't needed for the current task, skip it.
- **Keep frequently used content inline.** Templates and references used on every invocation belong in the skill file, not in separate files that cost a read each time.
- **Size phases and cap scope.** Limit files per phase, set turn budgets, account for mechanism costs.
