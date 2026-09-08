# thalix-pstack

`pstack` is a Rust CLI and portable agent-skill distribution for rigorous
engineering work. It installs the pstack corpus for Codex, Claude Code, OMP,
or Pi, then protects each managed installation with ownership and integrity
receipts.

The CLI follows the AXI conventions used by other Thalix tools. Operations are
typed, plans expose intended effects and risk, human output stays compact, and
`--json` returns deterministic machine-readable results.

This project adapts Cursor's open-source `pstack` plugin. See
[Source and license](#source-and-license) for the pinned source and attribution.

## Install the CLI

macOS or Linux:

```bash
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/thalixinc/thalix-pstack/main/install/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/thalixinc/thalix-pstack/main/install/install.ps1 | iex
```

Both installers download the matching GitHub release archive, verify it against
the release's `SHA256SUMS`, and install only the `pstack` binary. Set
`PSTACK_INSTALL_DIR` to choose another binary directory. Set `PSTACK_VERSION`
to a release such as `0.1.0` to pin an installation.

To build from source:

```bash
git clone https://github.com/thalixinc/thalix-pstack.git
cd thalix-pstack
cargo install --locked --path cli/pstack
```

Verify the binary:

```bash
pstack version
pstack targets
```

## Install the skills

Preview one target, apply it, then verify it:

```bash
pstack install --target codex --dry-run
pstack install --target codex
pstack status --target codex
pstack doctor --target codex
```

Use `claude`, `omp`, or `pi` in place of `codex`. Repeat `--target` to select
several runtimes. Omitting `--target` selects all four targets, so prefer an
explicit target unless you intentionally use every supported runtime.
Multi-target install and uninstall operations preflight every selected target
before writing and roll back the whole operation if a later write fails.

The default user-level locations are:

| Target | Skill root |
| --- | --- |
| Codex | `${CODEX_HOME:-~/.codex}/skills` |
| Claude Code | `~/.claude/skills` |
| OMP | `~/.omp/agent/skills` |
| Pi | `${PI_CODING_AGENT_DIR:-~/.pi/agent}/skills` |

Run `pstack where` to see the paths resolved on the current machine. Run
`pstack where codex` to inspect one target.

After upgrading the CLI, preview and apply a managed skill-corpus update:

```bash
pstack install --target codex --dry-run --update
pstack install --target codex --update
```

The update flag does not override an unowned directory or local edits. See
[Safety](#safety).

## Use the skill

Start with the installed `pstack` skill for a non-trivial engineering task:

```text
$pstack add a --json flag to this command. Keep text output byte-identical and verify both modes.
```

Invocation syntax depends on the host. Codex commonly uses `$pstack`; Claude
Code, OMP, and Pi may expose installed skills as `/pstack` or through their
skill picker. The portable entry skill detects the active host and uses its
native tools. It never requires Cursor's plugin manager, model slugs, cloud
agents, or `/loop` command.

The original focused skills remain available as separate installed skills,
including `poteto-mode`, `how`, `why`, `architect`, `interrogate`, `tdd`,
`technical-writing`, and the engineering-principle skills. List the complete
embedded inventory with:

```bash
pstack skill list
```

The [pstack guide](./docs/guide/README.md) explains the workflows. Its examples
use host-neutral skill names. The original Cursor manifest is retained only as
provenance at [`manifests/upstream-cursor/plugin.json`](./manifests/upstream-cursor/plugin.json);
the CLI does not load it.

## AXI command surface

`pstack` turns several recurring skill operations into deterministic CLI
commands:

| Command | Purpose | Writes files |
| --- | --- | --- |
| `pstack plan check [PATH]` | Validate that a plan is readable and actionable. | No |
| `pstack worktree audit [PATH]` | Inspect branch, dirtiness, and linked worktrees. | No |
| `pstack pr watch PR [--repo OWNER/REPO] [--once]` | Inspect one GitHub PR until a terminal verdict or bounded timeout. | No |
| `pstack decision log --decision TEXT --why TEXT --evidence TEXT --result TEXT` | Append a durable, evidence-linked decision. | Yes |
| `pstack orch init` | Initialize a plain-file task ledger. | Yes |
| `pstack orch add ID TITLE` | Add a task with owner and dependencies. | Yes |
| `pstack orch status [ID]` | Read effective task state. | No |
| `pstack orch status ID --set STATE` | Append a task-state transition. | Yes |
| `pstack orch evidence ID TEXT` | Append completion evidence. | Yes |
| `pstack orch gate ID --status pass\|fail` | Append a verification gate. | Yes |

Decision logs default to `.pstack/decisions.tsv`. Orchestration state defaults
to `.pstack/orch`. Both formats are plain files that can be reviewed and
committed. Use `--file` or `--dir` to place them elsewhere.

Every command supports `--json` at the top level:

```bash
pstack --json status --target codex
pstack --json worktree audit .
```

Successful JSON responses use the command's typed result. Command failures
write `{ "ok": false, "error": { "code": "...", "message": "..." } }`
to standard error and exit 1; invalid arguments use `invalid_arguments` and
exit 2. `doctor --json` is the exception: it writes its complete `checks` array
to standard output and exits 1 when any check fails.

Check for a newer release tag without changing the installed binary:

```bash
pstack update --check
```

The update check considers stable `vMAJOR.MINOR.PATCH` tags only. It ignores
prerelease and malformed tags.

## Safety

- Skill installation is limited to the current user's recognized skill roots.
- `install --dry-run` and `uninstall --dry-run` return the planned action
  without changing files.
- Installation stages a complete sibling tree and then renames it into place.
- Each managed skill is tracked by a deterministic SHA-256 digest in
  `<skills-root>/.pstack/receipt.json`.
- An existing unowned skill directory is a collision. `pstack` leaves it
  untouched.
- Local changes to a managed skill are drift. Update and uninstall refuse to
  discard them.
- Updating an unchanged managed installation requires `--update`.
- Symbolic links inside the packaged or installed skill tree are rejected.
- `status`, `doctor`, `targets`, `where`, `plan check`, `worktree audit`, and
  `update --check` are read-only.
- Every installed skill requires explicit user authorization before it sends an
  external message or performs a state-changing action in an external service.
  This includes pushes, PR and ticket changes, deployments, remote jobs, email,
  chat, and hosted settings. General autonomy instructions do not cross this
  boundary.

No credential is required to install or run the local skill workflows. The
release installer uses HTTPS and refuses an archive that does not match the
published SHA-256 checksum.

## Uninstall

Preview removal before applying it:

```bash
pstack uninstall --target codex --dry-run
pstack uninstall --target codex
```

Only unchanged installations owned by this CLI are removed. Delete the binary
separately from the directory shown by `pstack where` when it is no longer
needed.

## Development

The workspace requires Rust 1.85 or newer.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release --locked
```

Release tags use semantic versions such as `v0.1.0`. The release workflow
builds native archives for Linux, macOS, and Windows and publishes one
`SHA256SUMS` file consumed by both installers.

## Source and license

The skills, guide, agent prompts, automation templates, and logo are derived
from Cursor's `pstack` directory at commit
[`27e2a62ff94f9af4b5e68435e41cdceacadb840c`](https://github.com/cursor/plugins/tree/27e2a62ff94f9af4b5e68435e41cdceacadb840c/pstack).
Lauren Tan authored and copyrighted that upstream work in 2026 and released it
under the MIT License.

Thalix's portable CLI, packaging, and adaptations are also licensed under MIT.
See [`LICENSE`](./LICENSE) and [`THIRD_PARTY_NOTICES`](./THIRD_PARTY_NOTICES/README.md).
