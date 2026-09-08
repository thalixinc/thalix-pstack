### Orchestrate

Use this playbook for a standing program that outlives one agent session. The
coordinator owns decomposition, dispatch, evidence, and status. It does not
write application code.

Use the native `pstack orch` ledger. Do not run the legacy Bun orchestration
source or install dependencies inside a managed skill directory.

#### External action boundary

Read-only inspection may proceed. Pushing branches, opening or merging PRs,
posting comments, updating tickets, starting remote jobs, deploying, sending
messages, or changing hosted settings requires explicit user authorization for
that class of action. A request for autonomy or a long-running program does not
replace this authorization.

#### Roles

- **Coordinator.** Frames the program, writes briefs, reads the ledger, assigns
  ready tasks, records evidence and gates, and reports status.
- **Worker.** Owns one bounded task in one worktree with explicit file
  ownership and verification commands.
- **Verifier.** Independently checks high-risk or judgment-heavy work. A cheap,
  deterministic test may be verified directly by the coordinator.

Keep one writer per worktree and task. Run independent tasks concurrently only
when their write sets do not overlap.

#### Ledger

Initialize the append-only project ledger:

```bash
pstack orch init --dir .pstack/orch
```

Add every task before dispatch. Each task has an owner and may depend on other
task IDs:

```bash
pstack orch add API "Implement the API" --owner api --dir .pstack/orch
pstack orch add E2E "Verify the user journey" --owner verifier \
  --depends-on API --dir .pstack/orch
```

Valid states are `todo`, `doing`, `blocked`, and `done`:

```bash
pstack orch status --dir .pstack/orch
pstack orch status API --set doing --dir .pstack/orch
```

Record concrete evidence and the latest verification gate:

```bash
pstack orch evidence API "commit abc123; cargo test passed" --dir .pstack/orch
pstack orch gate API --status pass --note "targeted tests and live check passed" \
  --dir .pstack/orch
pstack orch status API --set done --dir .pstack/orch
```

Use top-level `--json` when another tool consumes the ledger output.

#### Brief

Each worker receives:

```text
GOAL         One observable outcome.
SCOPE        Paths it owns and paths it must not change.
CONTEXT      Repository paths, issues, and dependency outputs.
ACCEPTANCE   Checkable criteria.
VERIFY       Exact commands and live checks.
TIMEBOX      When to return partial evidence instead of running on.
FORBIDDEN    Destructive or external actions not explicitly authorized.
REPORT       State, branch, commit, evidence, deviations, and blockers.
```

A missing scope, acceptance check, or verification command is a
refuse-to-dispatch condition. A dependency is both ordering and context; pass
the upstream task's evidence to its dependent.

#### Loop

1. **Frame.** State a countable done predicate and the task graph. If one agent
   can finish within the session, use Autonomous run instead.
2. **Initialize.** Create the ledger and add every known task with dependencies.
3. **Pilot.** Run one representative task through implementation and
   verification. Correct the brief before wider fan-out.
4. **Dispatch.** Read `pstack orch status`. Assign only `todo` tasks whose
   dependencies are `done` with passing gates.
5. **Drain.** When a worker returns, inspect its branch and evidence. Record the
   evidence. Run or assign verification. Append a `pass` or `fail` gate.
6. **Advance.** Mark a task `done` only after a passing gate. Mark it `blocked`
   on a confirmed failure or missing authority. Add follow-up tasks explicitly.
7. **Report.** Give counts by state, newly completed tasks, failed gates,
   blockers, and external actions waiting for authorization.
8. **Close.** Confirm every task has a terminal, evidence-backed state and test
   the final integrated artifact.

Use the host's native wait or scheduling mechanism for long gaps. A timer is
not progress. Count commits, test results, artifacts, and ledger transitions.

#### Failure rules

- Retry a transient network failure once with the same scope.
- Reduce scope after an out-of-memory or time-budget failure.
- A failed verification gate creates a fix task. It does not become a pass on
  retry without new evidence.
- Never mark a task done from a worker summary alone.
- Do not resume an agent merely to ask for status. Read the ledger, branch, PR,
  or host-native agent status first.
- Stop the affected lane when progress requires missing authorization. Keep
  independent authorized work moving.

#### Reply

Report the done predicate and counts from `pstack orch status`, tasks completed
since the last checkpoint, evidence and gate results, active owners, blocked
tasks, and external actions awaiting explicit authorization.
