# Installation operations

Use the native CLI so ownership and local drift are checked before a write.

```bash
pstack targets
pstack where codex
pstack install --target codex --dry-run
pstack install --target codex
pstack status --target codex
pstack doctor --target codex
```

Replace `codex` with `claude`, `omp`, or `pi`. Repeat `--target` to select
several hosts. Omitting it selects all targets.

After a CLI upgrade, update unchanged managed skills explicitly:

```bash
pstack install --target codex --dry-run --update
pstack install --target codex --update
```

Never work around a collision or drift error by deleting the reported path.
Show the path and explain whether it is unowned content or a locally changed
managed skill. The user owns that decision.

Removal uses the same guard:

```bash
pstack uninstall --target codex --dry-run
pstack uninstall --target codex
```

Use top-level `--json` when another tool will consume the result.
