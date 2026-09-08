use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum, error::ErrorKind};
use fs2::FileExt;
use include_dir::{Dir, include_dir};
use pstack_core::Target;
use pstack_runtime::{InstallOptions, Installer};
use semver::Version;
use serde::Serialize;
use serde_json::{Value, json};
use walkdir::WalkDir;

const REPOSITORY: &str = "https://github.com/thalixinc/thalix-pstack";
const REVIEW_THREADS_QUERY: &str = r#"
query ReviewThreads($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: 100, after: $after) {
        nodes { isResolved isOutdated }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}
"#;
static PACKAGED_SKILLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../skills");
const EXECUTABLE_SKILL_FILES: &[&str] = &[
    "poteto-mode/scripts/check-plan.mjs",
    "poteto-mode/scripts/orch/orch.ts",
    "poteto-mode/scripts/watch-pr/watch-pr",
    "poteto-mode/scripts/worktree-audit.sh",
    "show-me-your-work/scripts/log.sh",
];

#[derive(Debug, Parser)]
#[command(
    name = "pstack",
    version,
    about = "Install and operate the Thalix pstack skill corpus"
)]
pub struct Cli {
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print the pstack CLI version.
    Version,
    /// Show the CLI and skill installation locations.
    Where {
        /// Limit output to one agent runtime.
        #[arg(value_enum)]
        target: Option<TargetArg>,
    },
    /// List supported agent runtimes and their skill directories.
    Targets,
    /// Install or refresh the pstack skill corpus.
    Install(InstallArgs),
    /// Remove the unchanged skill corpus installed by this CLI.
    Uninstall(UninstallArgs),
    /// Inspect installation state for supported runtimes.
    Status(TargetOptions),
    /// Run local, non-destructive environment diagnostics.
    Doctor(TargetOptions),
    /// Inspect skills packaged with pstack.
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
    /// Check whether a newer GitHub release tag exists.
    Update(UpdateArgs),
    /// Inspect GitHub pull requests with bounded polling.
    Pr {
        #[command(subcommand)]
        command: PrCommands,
    },
    /// Validate a plan before execution.
    Plan {
        #[command(subcommand)]
        command: PlanCommands,
    },
    /// Inspect Git worktrees without changing them.
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
    /// Record durable project decisions.
    Decision {
        #[command(subcommand)]
        command: DecisionCommands,
    },
    /// Maintain a plain-file orchestration ledger.
    Orch {
        #[command(subcommand)]
        command: OrchCommands,
    },
}

#[derive(Debug, Clone, Args)]
struct TargetOptions {
    /// Agent runtime to operate on; repeat to select several.
    #[arg(long, value_enum)]
    target: Vec<TargetArg>,
}

#[derive(Debug, Clone, Args)]
struct InstallArgs {
    #[command(flatten)]
    targets: TargetOptions,
    /// Print the AXI plan without writing files.
    #[arg(long)]
    dry_run: bool,
    /// Refresh an owned, unchanged install when the packaged version differs.
    #[arg(long)]
    update: bool,
}

#[derive(Debug, Clone, Args)]
struct UninstallArgs {
    #[command(flatten)]
    targets: TargetOptions,
    /// Print the protected-delete plan without removing files.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum TargetArg {
    Codex,
    Claude,
    Omp,
    Pi,
}

impl TargetArg {
    const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::Omp, Self::Pi];

    fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Omp => "omp",
            Self::Pi => "pi",
        }
    }
}

#[derive(Debug, Subcommand)]
enum SkillCommands {
    /// List skills embedded in this pstack release.
    List,
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Perform a read-only remote tag lookup.
    #[arg(long, required = true)]
    check: bool,
    /// Repository to query. Useful for mirrors and offline testing.
    #[arg(long, default_value = REPOSITORY)]
    repository: String,
}

#[derive(Debug, Subcommand)]
enum PlanCommands {
    /// Check that a plan exists, is readable, and has actionable content.
    Check {
        /// Plan file or directory. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum PrCommands {
    /// Watch one pull request until a terminal verdict or poll limit.
    ///
    /// Version 0.1 watches one PR only; stack and queued-stack modes are unsupported.
    Watch {
        /// Pull request number or URL accepted by gh.
        pr: String,
        /// GitHub OWNER/REPO when outside its checkout.
        #[arg(long)]
        repo: Option<String>,
        /// Query exactly once and return immediately.
        #[arg(long)]
        once: bool,
        /// Seconds between queries.
        #[arg(long, default_value_t = 30)]
        interval: u64,
        /// Maximum queries before returning a timeout verdict.
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..))]
        max_polls: u32,
    },
}

#[derive(Debug, Subcommand)]
enum WorktreeCommands {
    /// Audit branch, dirtiness, and linked worktrees read-only.
    Audit {
        /// Repository or worktree path.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum DecisionCommands {
    /// Append one decision to a tab-separated log.
    Log {
        /// Work phase in which the decision was made.
        #[arg(long, default_value = "implementation")]
        phase: String,
        /// The decision that was made.
        #[arg(long, alias = "summary")]
        decision: String,
        /// Rationale for the decision.
        #[arg(long, alias = "reason")]
        why: String,
        /// Evidence consulted or produced.
        #[arg(long)]
        evidence: String,
        /// Observed result.
        #[arg(long)]
        result: String,
        /// Destination TSV file.
        #[arg(long, default_value = ".pstack/decisions.tsv")]
        file: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum OrchCommands {
    /// Initialize an append-only orchestration ledger.
    Init(OrchRoot),
    /// Add a task to the ledger.
    Add {
        id: String,
        title: String,
        #[arg(long, default_value = "unassigned")]
        owner: String,
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
        #[command(flatten)]
        root: OrchRoot,
    },
    /// Show effective task status, or append a status transition.
    Status {
        id: Option<String>,
        #[arg(long, value_enum)]
        set: Option<TaskState>,
        #[command(flatten)]
        root: OrchRoot,
    },
    /// Append evidence to a task.
    Evidence {
        id: String,
        evidence: String,
        #[command(flatten)]
        root: OrchRoot,
    },
    /// Append a pass/fail gate result to a task.
    Gate {
        id: String,
        #[arg(long, value_enum)]
        status: GateState,
        #[arg(long, default_value = "")]
        note: String,
        #[command(flatten)]
        root: OrchRoot,
    },
}

#[derive(Debug, Clone, Args)]
struct OrchRoot {
    /// Ledger directory.
    #[arg(long, default_value = ".pstack/orch")]
    dir: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TaskState {
    Todo,
    Doing,
    Blocked,
    Done,
}

impl TaskState {
    fn name(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum GateState {
    Pass,
    Fail,
}

impl GateState {
    fn name(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Serialize)]
struct TargetView {
    target: &'static str,
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct TaskView {
    id: String,
    title: String,
    owner: String,
    dependencies: Vec<String>,
    status: String,
    evidence_count: usize,
    last_gate: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum RenderKind {
    Default,
    Doctor,
    Operation,
    PrWatch,
}

impl Commands {
    fn render_kind(&self) -> RenderKind {
        match self {
            Self::Doctor(_) => RenderKind::Doctor,
            Self::Install(_) | Self::Uninstall(_) | Self::Status(_) => RenderKind::Operation,
            Self::Pr { .. } => RenderKind::PrWatch,
            _ => RenderKind::Default,
        }
    }
}

pub fn run_from<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let json_requested = args.iter().any(|arg| arg == "--json");
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return 0;
        }
        Err(error) => {
            if json_requested {
                print_json_error("invalid_arguments", &error.to_string());
            } else {
                let _ = error.print();
            }
            return 2;
        }
    };
    let render_kind = cli.command.render_kind();
    match dispatch(cli.command) {
        Ok(value) => {
            let doctor_failed =
                matches!(render_kind, RenderKind::Doctor) && value["ok"].as_bool() == Some(false);
            if let Err(error) = print_value(&value, cli.json, render_kind) {
                if cli.json {
                    print_json_error("output_failed", &format!("{error:#}"));
                } else {
                    eprintln!("error: {error:#}");
                }
                1
            } else if doctor_failed {
                1
            } else {
                0
            }
        }
        Err(error) => {
            if cli.json {
                print_json_error("command_failed", &format!("{error:#}"));
            } else {
                eprintln!("error: {error:#}");
            }
            1
        }
    }
}

fn print_json_error(code: &str, message: &str) {
    eprintln!(
        "{}",
        serde_json::to_string(&json!({
            "ok": false,
            "error": { "code": code, "message": message }
        }))
        .expect("the fixed JSON error envelope is serializable")
    );
}

fn dispatch(command: Commands) -> Result<Value> {
    match command {
        Commands::Version => Ok(json!({ "version": env!("CARGO_PKG_VERSION") })),
        Commands::Where { target } => command_where(target),
        Commands::Targets => command_targets(),
        Commands::Install(options) => command_install(options),
        Commands::Uninstall(options) => command_uninstall(options),
        Commands::Status(options) => command_status(options),
        Commands::Doctor(options) => command_doctor(options),
        Commands::Skill { command } => match command {
            SkillCommands::List => command_skill_list(),
        },
        Commands::Update(args) => command_update(args),
        Commands::Pr { command } => match command {
            PrCommands::Watch {
                pr,
                repo,
                once,
                interval,
                max_polls,
            } => command_pr_watch(&pr, repo.as_deref(), once, interval, max_polls),
        },
        Commands::Plan { command } => match command {
            PlanCommands::Check { path } => command_plan_check(&path),
        },
        Commands::Worktree { command } => match command {
            WorktreeCommands::Audit { path } => command_worktree_audit(&path),
        },
        Commands::Decision { command } => match command {
            DecisionCommands::Log {
                phase,
                decision,
                why,
                evidence,
                result,
                file,
            } => command_decision_log(&file, &phase, &decision, &why, &evidence, &result),
        },
        Commands::Orch { command } => command_orch(command),
    }
}

fn selected_targets(options: TargetOptions) -> Vec<TargetArg> {
    if options.target.is_empty() {
        TargetArg::ALL.to_vec()
    } else {
        options
            .target
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

fn target_path(target: TargetArg) -> Result<PathBuf> {
    pstack_runtime::skills_root(runtime_target(target)).map_err(Into::into)
}

fn command_targets() -> Result<Value> {
    let targets = TargetArg::ALL
        .into_iter()
        .map(|target| {
            Ok(TargetView {
                target: target.name(),
                path: target_path(target)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({ "targets": targets }))
}

fn command_where(target: Option<TargetArg>) -> Result<Value> {
    let executable = std::env::current_exe().context("resolving the pstack executable")?;
    let targets = target.map_or_else(|| TargetArg::ALL.to_vec(), |target| vec![target]);
    let installs = targets
        .into_iter()
        .map(|target| {
            Ok(TargetView {
                target: target.name(),
                path: target_path(target)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({ "executable": executable, "installs": installs }))
}

// These four commands intentionally go through pstack-runtime. The adapter calls are kept
// together so the CLI cannot bypass the runtime's ownership and manifest checks.
fn command_install(args: InstallArgs) -> Result<Value> {
    let targets = selected_targets(args.targets);
    let source = PackagedSource::extract()?;
    let installer = source.installer()?;
    let results = installer.install_many(
        targets.into_iter().map(runtime_target),
        InstallOptions {
            dry_run: args.dry_run,
            update: args.update,
            ..Default::default()
        },
    )?;
    Ok(json!({ "operation": "install", "results": results }))
}

fn command_uninstall(args: UninstallArgs) -> Result<Value> {
    let targets = selected_targets(args.targets);
    let source = PackagedSource::extract()?;
    let installer = source.installer()?;
    let results =
        installer.uninstall_many(targets.into_iter().map(runtime_target), args.dry_run)?;
    Ok(json!({ "operation": "uninstall", "results": results }))
}

fn command_status(options: TargetOptions) -> Result<Value> {
    let targets = selected_targets(options);
    let source = PackagedSource::extract()?;
    let installer = source.installer()?;
    let mut results = Vec::new();
    for target in targets {
        results.push(serde_json::to_value(
            installer.status(runtime_target(target))?,
        )?);
    }
    Ok(json!({ "operation": "status", "results": results }))
}

fn command_doctor(options: TargetOptions) -> Result<Value> {
    let mut checks = vec![json!({
        "check": "git",
        "ok": command_available("git"),
        "detail": "required for update and worktree inspection"
    })];
    let source = PackagedSource::extract()?;
    let installer = source.installer()?;
    for target in selected_targets(options) {
        let result = installer.doctor(runtime_target(target))?;
        let healthy = matches!(result.outcome, pstack_core::Outcome::Healthy);
        checks.push(json!({
            "check": format!("{}-installation", target.name()),
            "ok": healthy,
            "detail": result,
        }));
    }
    let ok = checks
        .iter()
        .all(|check| check["ok"].as_bool() == Some(true));
    if ok {
        Ok(json!({ "ok": true, "checks": checks }))
    } else {
        Ok(json!({
            "ok": false,
            "error": {
                "code": "doctor_failed",
                "message": "one or more diagnostics failed"
            },
            "checks": checks
        }))
    }
}

fn command_skill_list() -> Result<Value> {
    let source = PackagedSource::extract()?;
    let skills = source.installer()?.packaged_skills()?;
    Ok(json!({ "skills": skills }))
}

fn runtime_target(target: TargetArg) -> Target {
    match target {
        TargetArg::Codex => Target::Codex,
        TargetArg::Claude => Target::Claude,
        TargetArg::Omp => Target::Omp,
        TargetArg::Pi => Target::Pi,
    }
}

struct PackagedSource {
    directory: tempfile::TempDir,
}

impl PackagedSource {
    fn extract() -> Result<Self> {
        let directory = tempfile::tempdir().context("creating packaged-skill staging directory")?;
        PACKAGED_SKILLS
            .extract(directory.path())
            .context("extracting packaged skills")?;
        restore_packaged_executables(directory.path())?;
        Ok(Self { directory })
    }

    fn installer(&self) -> Result<Installer> {
        Installer::new(self.directory.path()).map_err(Into::into)
    }
}

fn restore_packaged_executables(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for relative in EXECUTABLE_SKILL_FILES {
            let path = root.join(relative);
            let mut permissions = fs::metadata(&path)
                .with_context(|| format!("reading executable metadata for {}", path.display()))?
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions)
                .with_context(|| format!("restoring executable mode for {}", path.display()))?;
        }
    }
    Ok(())
}

fn command_update(args: UpdateArgs) -> Result<Value> {
    debug_assert!(args.check);
    let output = Command::new("git")
        .args(["ls-remote", "--tags", "--refs", &args.repository])
        .output()
        .with_context(|| format!("checking release tags at {}", args.repository))?;
    if !output.status.success() {
        bail!(
            "release check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let tags = String::from_utf8_lossy(&output.stdout);
    let latest = latest_stable_version(&tags);
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("the compiled pstack version is not valid SemVer")?;
    let update_available = latest.as_ref().is_some_and(|latest| latest > &current);
    Ok(json!({
        "repository": args.repository,
        "current": current.to_string(),
        "latest": latest.map(|version| version.to_string()),
        "update_available": update_available,
    }))
}

fn latest_stable_version(tags: &str) -> Option<Version> {
    tags.lines()
        .filter_map(|line| line.split_once("refs/tags/v").map(|(_, tag)| tag.trim()))
        .filter_map(|tag| Version::parse(tag).ok())
        .filter(|version| version.pre.is_empty())
        .max()
}

fn command_pr_watch(
    pr: &str,
    repo: Option<&str>,
    once: bool,
    interval: u64,
    max_polls: u32,
) -> Result<Value> {
    let poll_limit = if once { 1 } else { max_polls };
    for poll in 1..=poll_limit {
        let data = gh_pr_view(pr, repo)?;
        let unresolved_review_threads = if data["state"].as_str() == Some("OPEN") {
            Some(gh_unresolved_review_threads(&data)?)
        } else {
            None
        };
        let verdict = pr_verdict(&data, unresolved_review_threads)?;
        let terminal = verdict != "pending";
        if terminal || poll == poll_limit {
            return Ok(json!({
                "pr": pr,
                "repository": repo,
                "polls": poll,
                "terminal": terminal,
                "verdict": if terminal || once { verdict } else { "timeout" },
                "unresolved_review_threads": unresolved_review_threads,
                "data": data,
            }));
        }
        thread::sleep(Duration::from_secs(interval));
    }
    unreachable!("poll limit is constrained to at least one")
}

fn gh_pr_view(pr: &str, repo: Option<&str>) -> Result<Value> {
    let mut command = Command::new("gh");
    command.args([
        "pr",
        "view",
        pr,
        "--json",
        "number,url,title,state,mergedAt,mergeStateStatus,isDraft,statusCheckRollup,reviewDecision",
    ]);
    if let Some(repo) = repo {
        command.args(["--repo", repo]);
    }
    let output = command
        .output()
        .context("running `gh pr view`; install and authenticate GitHub CLI")?;
    if !output.status.success() {
        bail!(
            "gh pr view failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing `gh pr view` JSON")
}

fn gh_unresolved_review_threads(pr: &Value) -> Result<usize> {
    let url = pr["url"]
        .as_str()
        .context("pull request URL is missing; cannot query review threads")?;
    let (owner, name) = repository_from_pr_url(url)?;
    let number = pr["number"]
        .as_u64()
        .context("pull request number is missing; cannot query review threads")?;
    let mut after: Option<String> = None;
    let mut unresolved = 0usize;
    for _ in 0..100 {
        let mut command = Command::new("gh");
        command.args([
            "api",
            "graphql",
            "-f",
            &format!("query={REVIEW_THREADS_QUERY}"),
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("name={name}"),
            "-F",
            &format!("number={number}"),
        ]);
        if let Some(cursor) = &after {
            command.args(["-F", &format!("after={cursor}")]);
        }
        let output = command
            .output()
            .context("running `gh api graphql` for pull request review threads")?;
        if !output.status.success() {
            bail!(
                "review-thread query failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let response: Value =
            serde_json::from_slice(&output.stdout).context("parsing review-thread GraphQL JSON")?;
        let review_threads = &response["data"]["repository"]["pullRequest"]["reviewThreads"];
        let nodes = review_threads["nodes"]
            .as_array()
            .context("review-thread response has missing or unknown nodes")?;
        for node in nodes {
            let resolved = node["isResolved"]
                .as_bool()
                .context("review thread has missing or unknown isResolved")?;
            let outdated = node["isOutdated"]
                .as_bool()
                .context("review thread has missing or unknown isOutdated")?;
            if !resolved && !outdated {
                unresolved += 1;
            }
        }
        let page_info = &review_threads["pageInfo"];
        let has_next_page = page_info["hasNextPage"]
            .as_bool()
            .context("review-thread response has missing or unknown hasNextPage")?;
        if !has_next_page {
            return Ok(unresolved);
        }
        after = Some(
            page_info["endCursor"]
                .as_str()
                .context("review-thread response requires pagination but has no endCursor")?
                .to_owned(),
        );
    }
    bail!("review-thread pagination exceeded 100 pages")
}

fn repository_from_pr_url(url: &str) -> Result<(&str, &str)> {
    let parts: Vec<_> = url.trim_end_matches('/').split('/').collect();
    let pull = parts
        .iter()
        .rposition(|part| *part == "pull")
        .context("pull request URL has no /pull/ segment")?;
    if pull < 2 {
        bail!("pull request URL has no owner/repository path");
    }
    Ok((parts[pull - 2], parts[pull - 1]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckVerdict {
    Passed,
    Pending,
    Failed,
}

fn pr_verdict(data: &Value, unresolved_review_threads: Option<usize>) -> Result<&'static str> {
    if data.get("mergedAt").is_some_and(|value| !value.is_null())
        || data["state"].as_str() == Some("MERGED")
    {
        return Ok("merged");
    }
    if data["state"].as_str() == Some("CLOSED") {
        return Ok("closed");
    }
    if data["state"].as_str() != Some("OPEN") {
        bail!("unknown or missing pull request state");
    }
    let is_draft = data["isDraft"]
        .as_bool()
        .context("unknown or missing isDraft value")?;
    let review = data["reviewDecision"]
        .as_str()
        .context("unknown or missing reviewDecision value")?;
    if !matches!(
        review,
        "" | "APPROVED" | "REVIEW_REQUIRED" | "CHANGES_REQUESTED"
    ) {
        bail!("unknown reviewDecision value: {review}");
    }
    let merge_state = data["mergeStateStatus"]
        .as_str()
        .context("unknown or missing mergeStateStatus value")?;
    if !matches!(
        merge_state,
        "BEHIND" | "BLOCKED" | "CLEAN" | "DIRTY" | "DRAFT" | "HAS_HOOKS" | "UNSTABLE"
    ) {
        bail!("unknown mergeStateStatus value: {merge_state}");
    }
    let checks = data["statusCheckRollup"]
        .as_array()
        .context("unknown or missing statusCheckRollup")?;
    if checks.is_empty() {
        return Ok("pending");
    }
    let check_verdicts = checks
        .iter()
        .map(check_verdict)
        .collect::<Result<Vec<_>>>()?;
    if check_verdicts.contains(&CheckVerdict::Failed) {
        return Ok("failed");
    }
    let unresolved_review_threads = unresolved_review_threads
        .context("review-thread status is unknown for an open pull request")?;
    if is_draft
        || review != "APPROVED"
        || merge_state != "CLEAN"
        || check_verdicts.contains(&CheckVerdict::Pending)
        || unresolved_review_threads != 0
    {
        return Ok("pending");
    }
    Ok("ready")
}

fn check_verdict(check: &Value) -> Result<CheckVerdict> {
    match check["__typename"].as_str() {
        Some("CheckRun") => {
            let status = check["status"]
                .as_str()
                .context("CheckRun has unknown or missing status")?;
            match status {
                "QUEUED" | "IN_PROGRESS" | "PENDING" | "WAITING" => Ok(CheckVerdict::Pending),
                "COMPLETED" => match check["conclusion"].as_str() {
                    Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => Ok(CheckVerdict::Passed),
                    Some(
                        "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STALE"
                        | "STARTUP_FAILURE",
                    ) => Ok(CheckVerdict::Failed),
                    Some(value) => bail!("CheckRun has unknown conclusion: {value}"),
                    None => bail!("completed CheckRun has missing conclusion"),
                },
                value => bail!("CheckRun has unknown status: {value}"),
            }
        }
        Some("StatusContext") => match check["state"].as_str() {
            Some("SUCCESS") => Ok(CheckVerdict::Passed),
            Some("PENDING" | "EXPECTED") => Ok(CheckVerdict::Pending),
            Some("ERROR" | "FAILURE") => Ok(CheckVerdict::Failed),
            Some(value) => bail!("StatusContext has unknown state: {value}"),
            None => bail!("StatusContext has unknown or missing state"),
        },
        Some(value) => bail!("unknown statusCheckRollup entry type: {value}"),
        None => bail!("statusCheckRollup entry has missing __typename"),
    }
}

fn command_plan_check(path: &Path) -> Result<Value> {
    let files = find_plan_files(path)?;
    if files.is_empty() {
        bail!("no plan markdown found at {}", path.display());
    }
    let mut unchecked = 0usize;
    let mut todo_markers = 0usize;
    let mut actionable_items = 0usize;
    let mut problems = Vec::new();
    for file in &files {
        let content =
            fs::read_to_string(file).with_context(|| format!("reading plan {}", file.display()))?;
        if content.trim().is_empty() {
            problems.push(format!("{} is empty", file.display()));
        } else if !content.lines().any(|line| line.starts_with("# ")) {
            problems.push(format!("{} has no H1 title", file.display()));
        }
        unchecked += content
            .lines()
            .filter(|line| line.contains("- [ ]"))
            .count();
        todo_markers += content.match_indices("TODO").count();
        actionable_items += content
            .lines()
            .filter(|line| is_actionable_plan_line(line))
            .count();
    }
    if actionable_items == 0 {
        problems.push("plan has no actionable list item".to_owned());
    }
    if !problems.is_empty() {
        bail!("plan validation failed: {}", problems.join("; "));
    }
    Ok(json!({
        "ok": true,
        "path": path,
        "files": files,
        "unchecked_tasks": unchecked,
        "todo_markers": todo_markers,
        "actionable_items": actionable_items,
        "problems": problems,
    }))
}

fn is_actionable_plan_line(line: &str) -> bool {
    let line = line.trim_start();
    if line.starts_with("- ") || line.starts_with("* ") {
        return line.len() > 2;
    }
    let Some((number, rest)) = line.split_once(". ") else {
        return false;
    };
    !number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && !rest.trim().is_empty()
}

fn find_plan_files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.exists() {
        bail!("plan path does not exist: {}", path.display());
    }
    let uppercase = path.join("PLAN.md");
    let lowercase = path.join("plan.md");
    let mut files = if uppercase.is_file() {
        vec![uppercase]
    } else if lowercase.is_file() {
        vec![lowercase]
    } else {
        Vec::new()
    };
    let plan_root = path.join(".codex/ultra/plans");
    if plan_root.is_dir() {
        files.extend(
            WalkDir::new(plan_root)
                .max_depth(4)
                .into_iter()
                .filter_map(Result::ok)
                .map(|entry| entry.into_path())
                .filter(|file| file.extension().is_some_and(|extension| extension == "md")),
        );
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn command_worktree_audit(path: &Path) -> Result<Value> {
    let status = git_output(path, &["status", "--porcelain=v1", "--branch"])?;
    let mut lines = status.lines();
    let branch = lines
        .next()
        .unwrap_or("## unknown")
        .trim_start_matches("## ");
    let changes: Vec<_> = lines.map(str::to_owned).collect();
    let worktree_output = git_output(path, &["worktree", "list", "--porcelain"])?;
    let worktrees: Vec<_> = worktree_output
        .split("\n\n")
        .filter_map(|entry| {
            let fields: BTreeMap<_, _> = entry
                .lines()
                .filter_map(|line| line.split_once(' '))
                .collect();
            fields.get("worktree").map(|worktree| {
                json!({
                    "path": worktree,
                    "head": fields.get("HEAD"),
                    "branch": fields.get("branch"),
                    "detached": entry.lines().any(|line| line == "detached"),
                    "locked": entry.lines().any(|line| line.starts_with("locked")),
                    "prunable": entry.lines().any(|line| line.starts_with("prunable")),
                })
            })
        })
        .collect();
    Ok(json!({
        "repository": path,
        "branch": branch,
        "clean": changes.is_empty(),
        "changes": changes,
        "worktrees": worktrees,
    }))
}

fn git_output(path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn command_decision_log(
    file: &Path,
    phase: &str,
    decision: &str,
    why: &str,
    evidence: &str,
    result: &str,
) -> Result<Value> {
    ensure_parent(file)?;
    let new = !file.exists() || fs::metadata(file)?.len() == 0;
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .with_context(|| format!("opening {}", file.display()))?;
    if new {
        writeln!(output, "ts\tphase\tdecision\twhy\tevidence\tresult")?;
    }
    let timestamp = unix_timestamp()?;
    let cells = [phase, decision, why, evidence, result].map(sanitize_tsv_cell);
    writeln!(output, "{timestamp}\t{}", cells.join("\t"))?;
    Ok(json!({ "logged": true, "file": file, "timestamp": timestamp, "decision": decision }))
}

fn command_orch(command: OrchCommands) -> Result<Value> {
    match command {
        OrchCommands::Init(root) => orch_init(&root.dir),
        OrchCommands::Add {
            id,
            title,
            owner,
            depends_on,
            root,
        } => orch_add(&root.dir, &id, &title, &owner, &depends_on),
        OrchCommands::Status { id, set, root } => orch_status(&root.dir, id.as_deref(), set),
        OrchCommands::Evidence { id, evidence, root } => orch_evidence(&root.dir, &id, &evidence),
        OrchCommands::Gate {
            id,
            status,
            note,
            root,
        } => orch_gate(&root.dir, &id, status, &note),
    }
}

fn orch_init(dir: &Path) -> Result<Value> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    with_orch_lock(dir, || {
        ensure_file_with_header(
            &dir.join("tasks.tsv"),
            "timestamp\tid\towner\ttitle\tdependencies",
        )?;
        ensure_file_with_header(&dir.join("events.tsv"), "timestamp\tid\tkind\tvalue\tnote")?;
        Ok(json!({ "initialized": true, "directory": dir }))
    })
}

fn orch_add(
    dir: &Path,
    id: &str,
    title: &str,
    owner: &str,
    dependencies: &[String],
) -> Result<Value> {
    ensure_id(id)?;
    for dependency in dependencies {
        ensure_id(dependency)?;
    }
    orch_ensure_initialized(dir)?;
    with_orch_lock(dir, || {
        let tasks = orch_load(dir)?;
        if tasks.contains_key(id) {
            bail!("task already exists: {id}");
        }
        if dependencies.iter().any(|dependency| dependency == id) {
            bail!("task cannot depend on itself: {id}");
        }
        if let Some(missing) = dependencies
            .iter()
            .find(|dependency| !tasks.contains_key(dependency.as_str()))
        {
            bail!("dependency does not exist: {missing}");
        }
        let dependency_list = dependencies.join(",");
        append_tsv(
            &dir.join("tasks.tsv"),
            &[
                &unix_timestamp()?.to_string(),
                id,
                owner,
                title,
                &dependency_list,
            ],
        )?;
        append_event(dir, id, "status", "todo", "task added")?;
        Ok(json!({ "added": true, "id": id, "status": "todo" }))
    })
}

fn orch_status(dir: &Path, id: Option<&str>, set: Option<TaskState>) -> Result<Value> {
    orch_ensure_initialized(dir)?;
    let tasks = if let Some(state) = set {
        with_orch_lock(dir, || {
            let id = id.context("TASK_ID is required with --set")?;
            let tasks = orch_load(dir)?;
            let task = tasks
                .get(id)
                .with_context(|| format!("task not found: {id}"))?;
            if matches!(state, TaskState::Done) {
                if task.evidence_count == 0 {
                    bail!("task {id} cannot be done without evidence");
                }
                if task.last_gate.as_deref() != Some("pass") {
                    bail!("task {id} cannot be done without a passing gate");
                }
                if let Some(dependency) = task.dependencies.iter().find(|dependency| {
                    tasks
                        .get(*dependency)
                        .is_none_or(|task| task.status != "done")
                }) {
                    bail!("task {id} has an incomplete dependency: {dependency}");
                }
            }
            append_event(dir, id, "status", state.name(), "status transition")?;
            orch_load(dir)
        })?
    } else {
        orch_load(dir)?
    };
    let selected: Vec<_> = tasks
        .into_values()
        .filter(|task| id.is_none_or(|id| task.id == id))
        .collect();
    if let Some(id) = id
        && selected.is_empty()
    {
        bail!("task not found: {id}");
    }
    Ok(json!({ "tasks": selected }))
}

fn orch_evidence(dir: &Path, id: &str, evidence: &str) -> Result<Value> {
    ensure_id(id)?;
    orch_ensure_initialized(dir)?;
    with_orch_lock(dir, || {
        ensure_task_exists(dir, id)?;
        append_event(dir, id, "evidence", "recorded", evidence)?;
        Ok(json!({ "recorded": true, "id": id, "evidence": evidence }))
    })
}

fn orch_gate(dir: &Path, id: &str, status: GateState, note: &str) -> Result<Value> {
    ensure_id(id)?;
    orch_ensure_initialized(dir)?;
    with_orch_lock(dir, || {
        ensure_task_exists(dir, id)?;
        if matches!(status, GateState::Pass) && orch_load(dir)?[id].evidence_count == 0 {
            bail!("task {id} cannot pass a gate without recorded evidence");
        }
        append_event(dir, id, "gate", status.name(), note)?;
        Ok(json!({ "recorded": true, "id": id, "gate": status.name(), "note": note }))
    })
}

fn with_orch_lock<T>(dir: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock_path = dir.join(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening orchestration lock {}", lock_path.display()))?;
    lock.lock_exclusive()
        .with_context(|| format!("locking orchestration ledger {}", dir.display()))?;
    let result = operation();
    let unlock = FileExt::unlock(&lock)
        .with_context(|| format!("unlocking orchestration ledger {}", dir.display()));
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn orch_load(dir: &Path) -> Result<BTreeMap<String, TaskView>> {
    let mut tasks = BTreeMap::new();
    let content = fs::read_to_string(dir.join("tasks.tsv"))?;
    for line in content.lines().skip(1).filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 5 {
            bail!("invalid tasks.tsv row: {line}");
        }
        let dependencies = fields[4]
            .split(',')
            .filter(|dependency| !dependency.is_empty())
            .map(str::to_owned)
            .collect();
        tasks.insert(
            fields[1].to_owned(),
            TaskView {
                id: fields[1].to_owned(),
                owner: fields[2].to_owned(),
                title: fields[3].to_owned(),
                dependencies,
                status: "todo".to_owned(),
                evidence_count: 0,
                last_gate: None,
            },
        );
    }
    let events = fs::read_to_string(dir.join("events.tsv"))?;
    for line in events.lines().skip(1).filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 5 {
            bail!("invalid events.tsv row: {line}");
        }
        if let Some(task) = tasks.get_mut(fields[1]) {
            match fields[2] {
                "status" => task.status = fields[3].to_owned(),
                "evidence" => task.evidence_count += 1,
                "gate" => task.last_gate = Some(fields[3].to_owned()),
                _ => {}
            }
        }
    }
    Ok(tasks)
}

fn append_event(dir: &Path, id: &str, kind: &str, value: &str, note: &str) -> Result<()> {
    let timestamp = unix_timestamp()?.to_string();
    append_tsv(
        &dir.join("events.tsv"),
        &[&timestamp, id, kind, value, note],
    )
}

fn append_tsv(file: &Path, fields: &[&str]) -> Result<()> {
    let mut output = OpenOptions::new()
        .append(true)
        .open(file)
        .with_context(|| format!("opening {}", file.display()))?;
    let fields: Vec<_> = fields
        .iter()
        .map(|field| sanitize_tsv_cell(field))
        .collect();
    writeln!(output, "{}", fields.join("\t"))?;
    Ok(())
}

fn orch_ensure_initialized(dir: &Path) -> Result<()> {
    if !dir.join("tasks.tsv").is_file() || !dir.join("events.tsv").is_file() {
        bail!(
            "orchestration ledger is not initialized at {}; run `pstack orch init --dir {}`",
            dir.display(),
            dir.display()
        );
    }
    Ok(())
}

fn ensure_task_exists(dir: &Path, id: &str) -> Result<()> {
    if !orch_load(dir)?.contains_key(id) {
        bail!("task not found: {id}");
    }
    Ok(())
}

fn ensure_file_with_header(path: &Path, header: &str) -> Result<()> {
    if !path.exists() || fs::metadata(path)?.len() == 0 {
        fs::write(path, format!("{header}\n"))
            .with_context(|| format!("initializing {}", path.display()))?;
    }
    Ok(())
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    Ok(())
}

fn ensure_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("task ids may contain only letters, numbers, dot, dash, and underscore");
    }
    Ok(())
}

fn sanitize_tsv_cell(value: &str) -> String {
    let mut value = value.replace(['\t', '\n', '\r'], " ");
    if value.starts_with(['=', '+', '-', '@']) {
        value.insert(0, '\'');
    }
    value
}

fn unix_timestamp() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn print_value(value: &Value, as_json: bool, kind: RenderKind) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        match kind {
            RenderKind::Doctor => print_doctor(value),
            RenderKind::Operation => print_operation(value),
            RenderKind::PrWatch => print_pr_watch(value),
            RenderKind::Default => print_human(value, 0),
        }
    }
    Ok(())
}

fn print_doctor(value: &Value) {
    println!(
        "doctor: {}",
        if value["ok"].as_bool() == Some(true) {
            "ok"
        } else {
            "failed"
        }
    );
    if let Some(checks) = value["checks"].as_array() {
        for check in checks {
            let name = check["check"].as_str().unwrap_or("unknown");
            let status = if check["ok"].as_bool() == Some(true) {
                "ok"
            } else {
                "fail"
            };
            let detail = check["detail"]["outcome"]
                .as_str()
                .or_else(|| check["detail"].as_str())
                .unwrap_or("");
            if detail.is_empty() {
                println!("- {name}: {status}");
            } else {
                println!("- {name}: {status} ({detail})");
            }
        }
    }
}

fn print_operation(value: &Value) {
    let operation = value["operation"].as_str().unwrap_or("operation");
    println!("{operation}:");
    if let Some(results) = value["results"].as_array() {
        for result in results {
            let target = result["plan"]["target"].as_str().unwrap_or("unknown");
            let outcome = result["outcome"].as_str().unwrap_or("unknown");
            let destination = result["plan"]["destination"].as_str().unwrap_or("unknown");
            println!("- {target}: {outcome} -> {destination}");
            if let Some(diagnostics) = result["diagnostics"].as_array() {
                for diagnostic in diagnostics.iter().filter_map(Value::as_str) {
                    println!("  note: {diagnostic}");
                }
            }
        }
    }
}

fn print_pr_watch(value: &Value) {
    println!(
        "pr {}: {} (polls={}, unresolved_threads={})",
        value["pr"].as_str().unwrap_or("unknown"),
        value["verdict"].as_str().unwrap_or("unknown"),
        value["polls"],
        value["unresolved_review_threads"]
    );
}

fn print_human(value: &Value, indent: usize) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                match value {
                    Value::Array(values) => {
                        println!("{}{key}[{}]:", " ".repeat(indent), values.len());
                        for value in values {
                            print_human(value, indent + 2);
                        }
                    }
                    Value::Object(_) => {
                        println!("{}{key}:", " ".repeat(indent));
                        print_human(value, indent + 2);
                    }
                    _ => println!("{}{key}: {}", " ".repeat(indent), scalar(value)),
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                match value {
                    Value::Object(_) | Value::Array(_) => {
                        println!("{}-", " ".repeat(indent));
                        print_human(value, indent + 2);
                    }
                    _ => println!("{}- {}", " ".repeat(indent), scalar(value)),
                }
            }
        }
        _ => println!("{}{}", " ".repeat(indent), scalar(value)),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_selects_only_the_latest_stable_semver_tag() {
        let tags = concat!(
            "a\trefs/tags/v0.9.9\n",
            "b\trefs/tags/v0.10.0-rc.1\n",
            "c\trefs/tags/vmalformed\n",
            "d\trefs/tags/v0.10.0\n",
            "e\trefs/tags/v99.0.0-beta.1\n",
            "f\trefs/tags/not-versioned\n",
        );
        assert_eq!(
            latest_stable_version(tags),
            Some(Version::parse("0.10.0").unwrap())
        );
        assert_eq!(latest_stable_version("a\trefs/tags/v1.0.0-rc.1\n"), None);
    }

    fn open_pr() -> Value {
        json!({
            "state": "OPEN",
            "mergedAt": null,
            "isDraft": false,
            "reviewDecision": "APPROVED",
            "mergeStateStatus": "CLEAN",
            "statusCheckRollup": [{
                "__typename": "CheckRun",
                "status": "COMPLETED",
                "conclusion": "SUCCESS"
            }]
        })
    }

    #[test]
    fn pr_ready_requires_approved_nondraft_with_successful_checks() {
        assert_eq!(pr_verdict(&open_pr(), Some(0)).unwrap(), "ready");
        assert_eq!(pr_verdict(&open_pr(), Some(1)).unwrap(), "pending");
        assert!(pr_verdict(&open_pr(), None).is_err());

        let mut draft = open_pr();
        draft["isDraft"] = json!(true);
        assert_eq!(pr_verdict(&draft, Some(0)).unwrap(), "pending");

        for decision in ["REVIEW_REQUIRED", "CHANGES_REQUESTED"] {
            let mut pr = open_pr();
            pr["reviewDecision"] = json!(decision);
            assert_eq!(pr_verdict(&pr, Some(0)).unwrap(), "pending");
        }

        let mut no_checks = open_pr();
        no_checks["statusCheckRollup"] = json!([]);
        assert_eq!(pr_verdict(&no_checks, Some(0)).unwrap(), "pending");
    }

    #[test]
    fn pr_status_context_states_fail_closed() {
        for state in ["ERROR", "FAILURE"] {
            let mut pr = open_pr();
            pr["statusCheckRollup"] = json!([{
                "__typename": "StatusContext",
                "state": state
            }]);
            assert_eq!(pr_verdict(&pr, Some(0)).unwrap(), "failed");
        }
        for state in ["PENDING", "EXPECTED"] {
            let mut pr = open_pr();
            pr["statusCheckRollup"] = json!([{
                "__typename": "StatusContext",
                "state": state
            }]);
            assert_eq!(pr_verdict(&pr, Some(0)).unwrap(), "pending");
        }
    }

    #[test]
    fn pr_unknown_or_missing_check_data_is_an_error() {
        let mut unknown = open_pr();
        unknown["statusCheckRollup"] = json!([{
            "__typename": "FutureCheck",
            "state": "SUCCESS"
        }]);
        assert!(pr_verdict(&unknown, Some(0)).is_err());

        let mut missing = open_pr();
        missing.as_object_mut().unwrap().remove("statusCheckRollup");
        assert!(pr_verdict(&missing, Some(0)).is_err());

        let mut unknown_review = open_pr();
        unknown_review["reviewDecision"] = json!("FUTURE_VALUE");
        assert!(pr_verdict(&unknown_review, Some(0)).is_err());
    }

    #[test]
    fn plan_check_counts_open_tasks() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("PLAN.md"),
            "# Plan\n\n- [ ] ship\n- [x] test\nTODO verify\n",
        )
        .unwrap();
        let report = command_plan_check(temp.path()).unwrap();
        assert_eq!(report["ok"], true);
        assert_eq!(report["unchecked_tasks"], 1);
        assert_eq!(report["todo_markers"], 1);
    }

    #[test]
    fn plan_check_rejects_empty_or_untitled_plans() {
        let temp = tempfile::tempdir().unwrap();
        let plan = temp.path().join("PLAN.md");
        fs::write(&plan, "").unwrap();
        assert!(command_plan_check(&plan).is_err());
        fs::write(&plan, "- [ ] no title\n").unwrap();
        assert!(command_plan_check(&plan).is_err());
        fs::write(&plan, "# Title only\n\nSome context, but no action.\n").unwrap();
        assert!(command_plan_check(&plan).is_err());
    }

    #[test]
    fn orchestration_ledger_is_append_only_and_reconstructs_state() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("orch");
        orch_init(&dir).unwrap();
        orch_add(&dir, "T1", "Build CLI", "codex", &[]).unwrap();
        orch_evidence(&dir, "T1", "cargo test passed").unwrap();
        orch_gate(&dir, "T1", GateState::Pass, "tests green").unwrap();
        orch_status(&dir, Some("T1"), Some(TaskState::Done)).unwrap();

        let tasks = orch_load(&dir).unwrap();
        let task = &tasks["T1"];
        assert_eq!(task.status, "done");
        assert_eq!(task.evidence_count, 1);
        assert_eq!(task.last_gate.as_deref(), Some("pass"));
        assert_eq!(
            fs::read_to_string(dir.join("tasks.tsv"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        assert_eq!(
            fs::read_to_string(dir.join("events.tsv"))
                .unwrap()
                .lines()
                .count(),
            5
        );
    }

    #[test]
    fn tsv_fields_neutralize_record_and_formula_injection() {
        assert_eq!(sanitize_tsv_cell("bad\nrow\tcell"), "bad row cell");
        assert_eq!(sanitize_tsv_cell("=1+1"), "'=1+1");
        assert_eq!(sanitize_tsv_cell("@SUM(A:A)"), "'@SUM(A:A)");
    }
}
