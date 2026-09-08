# Host portability

The pstack corpus originated as a Cursor plugin. This distribution installs the
same engineering workflows as standard `SKILL.md` packages on four hosts.

| Capability | Portable behavior |
| --- | --- |
| Invoke a skill | Use the host's skill picker or native skill syntax. Do not require slash-command spelling. |
| Delegate work | Use the host's native subagent interface and installed role catalog. |
| Run in parallel | Use native concurrency when available; otherwise run independent lanes sequentially. |
| Select a model | Inherit the parent by default. Use an explicit model only after the host confirms it is available. |
| Ask a question | Use the host's structured question tool when available; otherwise ask one concise question. |
| Wait or schedule | Use the host's native wait or scheduling mechanism. Do not assume Cursor `/loop`. |
| Read prior context | Stay inside the active workspace and host-provided thread or transcript APIs. Never scan unrelated private histories. |
| Control a UI or CLI | Use an installed, host-native control tool. If none exists, state the verification gap. |

Host-specific integrations may still be useful when they are actually
installed. Detect them before use and keep the portable workflow functional
without them.

External messages and state-changing external service actions require explicit
user authorization for that exact class of action. A request for autonomy or a
long-running loop does not authorize pushes, PR or ticket changes, deployments,
remote jobs, messages, or hosted-setting changes.

The original Cursor plugin manifest is retained at
`manifests/upstream-cursor/plugin.json` for attribution and comparison. The
`pstack` CLI does not install or load that manifest.
