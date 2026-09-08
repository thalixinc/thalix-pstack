# Native CLI operations

Use these deterministic operations instead of rebuilding their mechanics in a
prompt or one-off shell pipeline.

```bash
pstack plan check [PATH]
pstack worktree audit [PATH]
pstack decision log --decision TEXT --why TEXT --evidence TEXT --result TEXT
pstack orch init
pstack orch add ID TITLE --owner OWNER --depends-on ID
pstack orch status [ID]
pstack orch status ID --set todo|doing|blocked|done
pstack orch evidence ID TEXT
pstack orch gate ID --status pass|fail --note TEXT
```

Decision logs default to `.pstack/decisions.tsv`. Orchestration ledgers default
to `.pstack/orch`; select another ledger with `--dir`.

`plan check`, `worktree audit`, and an orchestration status read are
non-mutating. Decision logging, orchestration initialization, and ledger
transitions append local project state. Keep their files reviewable and do not
record secrets.
