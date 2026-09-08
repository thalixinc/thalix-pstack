# Pstack Ultracode Brief

Build and publish `thalixinc/thalix-pstack`: an AXI-style Rust CLI named `pstack` that packages a portable, attributed fork of Cursor's pstack skill corpus and installs it for Codex, Claude Code, OMP, or Pi.

## Acceptance evidence

- Rust formatting, Clippy, tests, and release build pass.
- Isolated smoke tests prove install, status/doctor, drift protection, and uninstall for all four targets.
- Compiled safe workflow operations have behavioral tests.
- Installer and release workflows exist and the GitHub repository is public.
- Upstream MIT notice and pinned provenance are present.

## Stop condition

Stop when the repository is published and the installed CLI reports its version and successfully manages an isolated skill installation, or when a credential/organization policy blocks publication after local verification.

