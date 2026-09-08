# Changelog

All notable changes to the Thalix distribution of pstack appear here.

This project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Rust `pstack` CLI with AXI-style plans, receipts, structured errors, and
  deterministic JSON output.
- Skill installation for Codex, Claude Code, OMP, and Pi.
- Native skill inventory, inspection, verification, and safe workflow helpers.
- Cross-platform binary installers and release automation.
- Portable pstack entry skill and host-neutral setup guidance.

### Changed

- Adapted Cursor's pstack skill corpus for multiple agent hosts.
- Replaced Cursor-only setup and fixed-model defaults with native `pstack`
  target and configuration discovery.

### Attribution

- Imported Cursor's `pstack` plugin at commit
  `27e2a62ff94f9af4b5e68435e41cdceacadb840c` under its MIT license.

[Unreleased]: https://github.com/thalixinc/thalix-pstack/commits/main
