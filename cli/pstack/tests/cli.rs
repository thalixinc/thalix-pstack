use std::process::{Command, Stdio};

#[test]
fn root_and_nested_commands_have_help() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let commands: &[&[&str]] = &[
        &["--help"],
        &["install", "--help"],
        &["uninstall", "--help"],
        &["status", "--help"],
        &["doctor", "--help"],
        &["skill", "list", "--help"],
        &["pr", "watch", "--help"],
        &["update", "--help"],
        &["plan", "check", "--help"],
        &["worktree", "audit", "--help"],
        &["decision", "log", "--help"],
        &["orch", "init", "--help"],
        &["orch", "add", "--help"],
        &["orch", "status", "--help"],
        &["orch", "evidence", "--help"],
        &["orch", "gate", "--help"],
    ];
    for args in commands {
        let output = Command::new(binary).args(*args).output().unwrap();
        assert!(
            output.status.success(),
            "pstack {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    }
}

#[test]
fn version_commands_report_workspace_version() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let short = Command::new(binary).arg("--version").output().unwrap();
    assert!(short.status.success());
    assert_eq!(
        String::from_utf8_lossy(&short.stdout).trim(),
        "pstack 0.1.0"
    );

    let json = Command::new(binary)
        .args(["--json", "version"])
        .output()
        .unwrap();
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["version"], "0.1.0");
}

#[test]
fn json_failures_use_a_stable_error_envelope() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-plan.md");
    let output = Command::new(binary)
        .args(["--json", "plan", "check", missing.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["ok"], false);
    assert_eq!(error["error"]["code"], "command_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not exist")
    );

    let invalid = Command::new(binary)
        .args(["--json", "not-a-command"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    let invalid: serde_json::Value = serde_json::from_slice(&invalid.stderr).unwrap();
    assert_eq!(invalid["error"]["code"], "invalid_arguments");
}

#[test]
fn doctor_failure_exits_nonzero_and_human_output_stays_compact() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    let failed = Command::new(binary)
        .args(["--json", "doctor", "--target", "codex"])
        .env("HOME", temp.path())
        .env("CODEX_HOME", &codex_home)
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert!(failed.stderr.is_empty());
    let failed: serde_json::Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["code"], "doctor_failed");
    assert!(failed["checks"][1]["detail"]["plan"]["actions"].is_array());

    let human = Command::new(binary)
        .args(["doctor", "--target", "codex"])
        .env("HOME", temp.path())
        .env("CODEX_HOME", &codex_home)
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(1));
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(
        human.lines().count() <= 4,
        "doctor output was not compact:\n{human}"
    );
    assert!(human.contains("codex-installation: fail (missing)"));
    assert!(!human.contains("actions"));
    assert!(!human.contains("source_sha256"));

    let status = Command::new(binary)
        .args(["--json", "status", "--target", "codex"])
        .env("HOME", temp.path())
        .env("CODEX_HOME", &codex_home)
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["results"][0]["outcome"], "missing");
}

#[test]
fn orchestration_commands_work_end_to_end() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("ledger");
    let run = |args: &[&str]| {
        let output = Command::new(binary).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "pstack {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let dir_text = dir.to_str().unwrap();
    run(&["orch", "init", "--dir", dir_text]);
    run(&[
        "orch",
        "add",
        "T1",
        "Build CLI",
        "--owner",
        "codex",
        "--dir",
        dir_text,
    ]);
    run(&[
        "orch",
        "evidence",
        "T1",
        "cargo test passed",
        "--dir",
        dir_text,
    ]);
    run(&[
        "orch", "gate", "T1", "--status", "pass", "--note", "verified", "--dir", dir_text,
    ]);
    run(&["orch", "status", "T1", "--set", "done", "--dir", dir_text]);
    let status = run(&["--json", "orch", "status", "T1", "--dir", dir_text]);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["tasks"][0]["status"], "done");
    assert_eq!(value["tasks"][0]["evidence_count"], 1);
    assert_eq!(value["tasks"][0]["last_gate"], "pass");
}

#[test]
fn concurrent_orchestration_add_allows_exactly_one_writer() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("ledger");
    let dir_text = dir.to_str().unwrap();
    let init = Command::new(binary)
        .args(["orch", "init", "--dir", dir_text])
        .output()
        .unwrap();
    assert!(init.status.success());

    let mut left = Command::new(binary)
        .args(["orch", "add", "SAME", "Left", "--dir", dir_text])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut right = Command::new(binary)
        .args(["orch", "add", "SAME", "Right", "--dir", dir_text])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let left = left.wait().unwrap();
    let right = right.wait().unwrap();
    assert_ne!(left.success(), right.success());

    let tasks = std::fs::read_to_string(dir.join("tasks.tsv")).unwrap();
    assert_eq!(tasks.lines().count(), 2, "duplicate task row was appended");
    let events = std::fs::read_to_string(dir.join("events.tsv")).unwrap();
    assert_eq!(
        events.lines().count(),
        2,
        "duplicate status event was appended"
    );
}

#[test]
fn install_lifecycle_uses_direct_skill_directories_and_honors_dry_run() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    let run = |args: &[&str]| {
        let output = Command::new(binary)
            .args(args)
            .env("HOME", temp.path())
            .env("CODEX_HOME", &codex_home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "pstack {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };

    let dry_run = run(&["--json", "install", "--target", "codex", "--dry-run"]);
    let dry_run: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(dry_run["results"][0]["outcome"], "planned");
    assert!(!codex_home.exists());

    let install = run(&["--json", "install", "--target", "codex"]);
    let install: serde_json::Value = serde_json::from_slice(&install.stdout).unwrap();
    assert_eq!(install["results"][0]["outcome"], "installed");
    assert!(codex_home.join("skills/architect/SKILL.md").is_file());
    assert!(codex_home.join("skills/.pstack/receipt.json").is_file());
    assert!(!codex_home.join("skills/pstack/architect/SKILL.md").exists());

    let wrapper = codex_home.join("skills/poteto-mode/scripts/watch-pr/watch-pr");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            std::fs::metadata(&wrapper).unwrap().permissions().mode() & 0o111,
            0
        );
    }
    let wrapper_help = Command::new(&wrapper)
        .arg("--help")
        .env(
            "PATH",
            format!(
                "{}:{}",
                std::path::Path::new(binary).parent().unwrap().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .unwrap();
    assert!(
        wrapper_help.status.success(),
        "installed wrapper failed: {}",
        String::from_utf8_lossy(&wrapper_help.stderr)
    );

    let status = run(&["--json", "status", "--target", "codex"]);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["results"][0]["outcome"], "healthy");

    let uninstall_plan = run(&["--json", "uninstall", "--target", "codex", "--dry-run"]);
    let uninstall_plan: serde_json::Value = serde_json::from_slice(&uninstall_plan.stdout).unwrap();
    assert_eq!(uninstall_plan["results"][0]["outcome"], "planned");
    assert!(codex_home.join("skills/architect/SKILL.md").is_file());

    let uninstall = run(&["--json", "uninstall", "--target", "codex"]);
    let uninstall: serde_json::Value = serde_json::from_slice(&uninstall.stdout).unwrap();
    assert_eq!(uninstall["results"][0]["outcome"], "removed");
    assert!(!codex_home.join("skills/architect").exists());
    assert!(!codex_home.join("skills/.pstack").exists());
}

#[test]
fn multi_target_install_preflights_every_target_before_writing_any() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    let claude_collision = temp.path().join(".claude/skills/architect");
    std::fs::create_dir_all(&claude_collision).unwrap();
    std::fs::write(claude_collision.join("mine.txt"), "keep\n").unwrap();

    let output = Command::new(binary)
        .args([
            "--json", "install", "--target", "codex", "--target", "claude",
        ])
        .env("HOME", temp.path())
        .env("CODEX_HOME", &codex_home)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "command_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not managed by pstack")
    );
    assert!(
        !codex_home.exists(),
        "Codex was mutated before Claude preflight failed"
    );
    assert_eq!(
        std::fs::read_to_string(claude_collision.join("mine.txt")).unwrap(),
        "keep\n"
    );
}

#[cfg(unix)]
#[test]
fn pr_watch_uses_gh_once_and_emits_json_verdict() {
    use std::os::unix::fs::PermissionsExt;

    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let gh = temp.path().join("gh");
    std::fs::write(
        &gh,
        "#!/bin/sh\nprintf '%s\\n' '{\"number\":42,\"state\":\"MERGED\",\"mergedAt\":\"2026-09-08T00:00:00Z\",\"mergeStateStatus\":\"CLEAN\",\"statusCheckRollup\":[]}'\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&gh).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&gh, permissions).unwrap();

    let output = Command::new(binary)
        .args(["--json", "pr", "watch", "42", "--once"])
        .env(
            "PATH",
            format!(
                "{}:{}",
                temp.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["verdict"], "merged");
    assert_eq!(value["terminal"], true);
    assert_eq!(value["polls"], 1);
}

#[cfg(unix)]
#[test]
fn pr_watch_never_reports_ready_with_an_unresolved_review_thread() {
    use std::os::unix::fs::PermissionsExt;

    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let gh = temp.path().join("gh");
    std::fs::write(
        &gh,
        r#"#!/bin/sh
if [ "$1" = "api" ]; then
  printf '%s\n' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[{"isResolved":false,"isOutdated":false}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}'
else
  printf '%s\n' '{"number":42,"url":"https://github.com/thalixinc/thalix-pstack/pull/42","state":"OPEN","mergedAt":null,"mergeStateStatus":"CLEAN","isDraft":false,"reviewDecision":"APPROVED","statusCheckRollup":[{"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"}]}'
fi
"#,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&gh).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&gh, permissions).unwrap();

    let output = Command::new(binary)
        .args(["--json", "pr", "watch", "42", "--once"])
        .env(
            "PATH",
            format!(
                "{}:{}",
                temp.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["verdict"], "pending");
    assert_eq!(value["terminal"], false);
    assert_eq!(value["unresolved_review_threads"], 1);
}

#[test]
fn update_check_and_worktree_audit_accept_a_local_git_repository() {
    let binary = env!("CARGO_BIN_EXE_pstack");
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git").args(args).output().unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", repo.to_str().unwrap()]);
    std::fs::write(repo.join("README.md"), "test\n").unwrap();
    git(&["-C", repo.to_str().unwrap(), "add", "README.md"]);
    git(&[
        "-C",
        repo.to_str().unwrap(),
        "-c",
        "user.name=pstack test",
        "-c",
        "user.email=pstack@example.invalid",
        "commit",
        "-m",
        "test",
    ]);
    git(&["-C", repo.to_str().unwrap(), "tag", "v9.0.0"]);
    git(&["-C", repo.to_str().unwrap(), "tag", "v99.0.0-rc.1"]);
    git(&["-C", repo.to_str().unwrap(), "tag", "vmalformed"]);

    let update = Command::new(binary)
        .args([
            "--json",
            "update",
            "--check",
            "--repository",
            repo.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(update.status.success());
    let update: serde_json::Value = serde_json::from_slice(&update.stdout).unwrap();
    assert_eq!(update["latest"], "9.0.0");
    assert_eq!(update["update_available"], true);

    let audit = Command::new(binary)
        .args(["--json", "worktree", "audit", repo.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(audit.status.success());
    let audit: serde_json::Value = serde_json::from_slice(&audit.stdout).unwrap();
    assert_eq!(audit["clean"], true);
    assert_eq!(audit["worktrees"].as_array().unwrap().len(), 1);
}
