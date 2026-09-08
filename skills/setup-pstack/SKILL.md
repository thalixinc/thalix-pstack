---
name: setup-pstack
description: Install, inspect, or update the portable pstack skills for Codex, Claude Code, OMP, or Pi. Use for setup-pstack, configuring pstack on a host, checking installation paths, or repairing a managed installation.
---

# Set up pstack

## External action boundary

Local, reversible work inside the user-scoped workspace may proceed when the request authorizes the task. Before sending any external message or performing any state-changing action in an external service, obtain explicit user authorization for that exact class of action. This includes posting comments, opening or merging pull requests, updating tickets, starting deployments or remote jobs, sending email or chat messages, and changing hosted settings. Read-only external inspection is allowed when needed. A general autonomy instruction does not replace this authorization.

Use the `pstack` CLI for installation state. Do not write Cursor rules, assume a
fixed model slug, or edit a host's configuration file.

## Set up one host

1. Confirm the CLI and supported targets with `pstack version` and
   `pstack targets`.
2. Identify the active host. Use the user's explicit target when provided.
   Otherwise infer it only from reliable runtime context.
3. Show the resolved location with `pstack where <target>`.
4. Preview with `pstack install --target <target> --dry-run`.
5. Install with `pstack install --target <target>`.
6. Verify with `pstack status --target <target>` and
   `pstack doctor --target <target>`.

Supported target values are `codex`, `claude`, `omp`, and `pi`. Repeat
`--target` to install for several. Omitting the flag selects all four and should
be intentional.

## Update

After the binary is upgraded, preview and apply the packaged skill update:

```bash
pstack install --target <target> --dry-run --update
pstack install --target <target> --update
```

An unowned destination or a locally modified managed skill is not overwritten.
Report the collision or drift and leave it untouched.

## Model routing

Pstack inherits the active host's model and role defaults. Use an explicit model
only when the host exposes a verified selector and the task benefits from that
override. If the host does not support model-specific subagents, run the same
workflow with its available agents instead of manufacturing a model name.

## Confirmation

Report the installed target, resolved path, status outcome, doctor outcome, and
the CLI version. A created directory alone is not proof of a healthy
installation.
