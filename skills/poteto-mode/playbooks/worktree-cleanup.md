### Worktree and simulator cleanup

**You own the disk and the safety gate.** Prune merged or abandoned git worktrees and stale iOS simulators to reclaim space. Deletion is irreversible, so every step guards against deleting something in use or holding uncommitted work.

1. Snapshot and audit. Record `df -h /`, then run `pstack worktree audit .` for the portable branch, dirtiness, and linked-worktree inventory. Use `scripts/worktree-audit.sh` only when its extended size, age, PR, and workspace-scoped transcript classification is needed. Both read paths from `git worktree list`, never from hand-typed guesses (principle-encode-lessons-in-structure).
2. The bucket is advice, not permission. The pinned and active chats are the real artifact (principle-prove-it-works). Get that set from the user or sidebar and cross-check every candidate. The lever has marked `safe` a worktree the user had pinned, so the pinned set wins.
3. Verify usage before deleting. For every `verify-recent-chat` row, or anything you doubt, fan subagents out to read the transcripts and report whether the chat is pinned or ongoing and which worktrees it touches (principle-guard-the-context-window, transcripts are bulk). A pinned chat spawns arena and repro trees into sibling worktrees via background subagents, and those are in use even when their names never hit the sidebar.
4. Pause on irreversible loss. `wip:N` is N tracked uncommitted edits. Show the diff and get a decision first, since removing a clean worktree is recoverable from its branch but uncommitted work is gone. `scratch:N` is untracked throwaway, safe to drop, but name the files. Per Autonomy, clean and merged and not-in-use proceeds. `wip` and in-use pause.
5. Prune only the confirmed set. Per explicit path, use `git worktree remove <path>` without force first. A failure caused by uncommitted or ignored files is a stop condition for that path. Use force only when the user explicitly authorized discarding the exact remaining files. Prefer moving a surviving directory to the operating system's trash, then run `git worktree prune`. Branch refs survive, but uncommitted files do not. Confirm with `df -h /` and re-list.
6. Simulators and other reclaimers are a separate, explicitly scoped cleanup. Inventory exact targets first. Clear only unavailable runtimes, generated data, or caches the user authorized, and report whether the operation is recoverable.

This is the one playbook that deletes user state with no code review to catch a slip, so the gates above are the review.

**Reply:** `df -h /` before and after with space reclaimed, the worktrees pruned, and a one-line reason for each held back (in-use by which chat, or uncommitted work).
