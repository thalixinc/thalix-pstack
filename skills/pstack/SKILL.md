---
name: pstack
description: Run rigorous, evidence-driven engineering workflows or safely manage the pstack skill installation on Codex, Claude Code, OMP, or Pi. Use for pstack, portable poteto-mode work, installation health, worktree audits, plan checks, decision logs, or orchestration ledgers.
---

# pstack

## External action boundary

Local, reversible work inside the user-scoped workspace may proceed when the request authorizes the task. Before sending any external message or performing any state-changing action in an external service, obtain explicit user authorization for that exact class of action. This includes posting comments, opening or merging pull requests, updating tickets, starting deployments or remote jobs, sending email or chat messages, and changing hosted settings. Read-only external inspection is allowed when needed. A general autonomy instruction does not replace this authorization.

Use the host's native tools and agent roles to complete engineering work with
small changes, explicit evidence, and independent verification.

## Route the request

- For an engineering task, read
  [`../poteto-mode/SKILL.md`](../poteto-mode/SKILL.md), select the matching
  playbook, and translate its tool references through the portability rules
  below.
- For installation, update, status, or removal, read
  [`references/installation.md`](references/installation.md) and use the
  `pstack` CLI.
- For plan checks, worktree audits, decision logs, or task ledgers, prefer the
  corresponding native `pstack` command described in
  [`references/cli-operations.md`](references/cli-operations.md).

## Host portability

Read [`references/host-portability.md`](references/host-portability.md) before
using a playbook that delegates work, schedules a follow-up, reads transcripts,
or invokes another skill.

These rules override host-specific examples in the imported corpus:

- Use the active host's native subagent, wait, question, scheduling, browser,
  and skill-invocation mechanisms.
- Treat named Cursor commands, Cursor cloud agents, `cursor-team-kit`, and
  Cursor transcript paths as optional examples, not dependencies.
- Inherit the current model unless the host exposes a verified model selector
  and the task has a concrete reason to override it. Never invent or assume an
  upstream model slug.
- Keep author and verifier independent when the host supports delegation. If it
  does not, perform sequential author and verification passes and disclose the
  limitation.
- Do not emulate a missing privileged or destructive tool with arbitrary shell
  commands. Preserve the user's authorization boundary.

## Completion

Report the outcome first. Name the evidence that proves it. Distinguish passing
checks from checks that could not run. Stop when the requested result is
complete and verified, or when missing authority or an irreversible decision
requires the user.
