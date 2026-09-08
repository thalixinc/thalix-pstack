# Set up pstack

Install the Rust CLI, preview the target path, install the skills, and verify
the managed result.

## Install the CLI

On macOS or Linux:

```bash
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/thalixinc/thalix-pstack/main/install/install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/thalixinc/thalix-pstack/main/install/install.ps1 | iex
```

The installer verifies the release archive against its published SHA-256
checksum before copying the binary.

## Choose a target

List the supported hosts and their resolved skill roots:

```bash
pstack targets
pstack where codex
```

The target values are `codex`, `claude`, `omp`, and `pi`.

## Preview and install

Preview one target before writing it:

```bash
pstack install --target codex --dry-run
pstack install --target codex
```

Repeat `--target` to select more than one host. Omitting the flag selects all
four, so make that choice intentionally.

The CLI installs every pstack skill as a directly discoverable child of the
host's user skill root. It records ownership and SHA-256 integrity metadata at
`<skills-root>/.pstack/receipt.json`.

## Verify the installation

```bash
pstack status --target codex
pstack doctor --target codex
pstack skill list
```

`status` checks ownership and content digests. `doctor` also checks the local
environment. A directory existing is not enough to call the installation
healthy.

Pstack inherits the host's model and role defaults. It does not write a model
configuration file or assume Cursor model slugs. A workflow uses an explicit
model only when the host reports that model as available.

## Run your first task

Pick something real but small and describe it as you would to a colleague:

```text
pstack: add a --json flag to this command. Keep text output byte-identical. Verify both modes.
```

Invoke the installed skill through the host's skill picker or native syntax.
Codex commonly uses `$pstack`; other hosts may expose `/pstack`.

Next: [Route work through poteto-mode](./02-poteto-mode.md).
