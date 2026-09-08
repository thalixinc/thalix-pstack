#!/usr/bin/env bun

const args = process.argv.slice(2);
const native = Bun.spawnSync(["pstack", "orch", ...args], {
  cwd: process.cwd(),
  env: process.env,
  stdin: "inherit",
  stdout: "inherit",
  stderr: "inherit",
});

if (native.error) {
  process.stderr.write(
    "The legacy orch helper is disabled. Install the pstack CLI and run `pstack orch --help`.\n"
  );
  process.exit(127);
}

process.exit(native.exitCode ?? 1);
