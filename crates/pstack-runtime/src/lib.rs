//! Safe user-scoped installation of the complete pstack skill corpus.

use directories::{BaseDirs, ProjectDirs};
use fs2::FileExt;
use pstack_core::{
    Action, CommandResult, Operation, Outcome, Plan, Receipt, Risk, SkillReceipt, Target,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    env, fs,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::{Builder, NamedTempFile, TempDir};
use thiserror::Error;
use walkdir::WalkDir;

pub const SKILL_NAME: &str = "pstack";
pub const STATE_DIRECTORY: &str = ".pstack";
pub const RECEIPT_FILE: &str = "receipt.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstallMode {
    #[default]
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstallOptions {
    pub dry_run: bool,
    /// Allow refresh only when every previously managed skill is unchanged.
    pub update: bool,
    pub mode: InstallMode,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("could not determine the current user's home directory")]
    HomeUnavailable,
    #[error("skill corpus does not exist or is not a directory: {0}")]
    InvalidSource(PathBuf),
    #[error("skill corpus contains no immediate child directories with SKILL.md: {0}")]
    EmptySource(PathBuf),
    #[error("invalid skill directory name: {0}")]
    InvalidSkillName(PathBuf),
    #[error("invalid OMP profile name: {0}")]
    InvalidProfile(String),
    #[error("destination is occupied by content not managed by pstack: {0}")]
    Collision(PathBuf),
    #[error("managed install has local drift and was left untouched: {0}")]
    Drift(PathBuf),
    #[error("managed install is outdated; rerun with update enabled: {0}")]
    UpdateRequired(PathBuf),
    #[error("unsafe symbolic link found in skill tree: {0}")]
    Symlink(PathBuf),
    #[error("receipt at {path} is invalid: {source}")]
    InvalidReceipt {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("filesystem operation failed for {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("transaction failed: {failures:?}")]
    Transaction { failures: Vec<String> },
    #[error("uninstall committed, but cleanup remains at {path}: {source}")]
    CleanupRetained { path: PathBuf, source: io::Error },
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Inputs used to resolve host-specific user skill paths. Keeping this pure
/// makes precedence testable without mutating process-global environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEnvironment {
    pub home: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub claude_config_dir: Option<PathBuf>,
    pub pi_coding_agent_dir: Option<PathBuf>,
    pub omp_profile: Option<String>,
    pub pi_profile: Option<String>,
}

impl HostEnvironment {
    pub fn detect() -> Result<Self> {
        let home = BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_owned())
            .ok_or(RuntimeError::HomeUnavailable)?;
        Ok(Self {
            home,
            codex_home: nonempty_path_env("CODEX_HOME"),
            claude_config_dir: nonempty_path_env("CLAUDE_CONFIG_DIR"),
            pi_coding_agent_dir: nonempty_path_env("PI_CODING_AGENT_DIR"),
            omp_profile: env::var("OMP_PROFILE").ok(),
            pi_profile: env::var("PI_PROFILE").ok(),
        })
    }
}

#[derive(Debug, Clone)]
struct SourceSkill {
    name: String,
    path: PathBuf,
    sha256: String,
}

#[derive(Debug, Clone)]
pub struct Installer {
    source: PathBuf,
}

impl Installer {
    pub fn new(source: impl Into<PathBuf>) -> Result<Self> {
        let source = source.into();
        if !source.is_dir() {
            return Err(RuntimeError::InvalidSource(source));
        }
        discover_skills(&source)?;
        Ok(Self { source })
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn packaged_skills(&self) -> Result<Vec<String>> {
        Ok(discover_skills(&self.source)?
            .into_iter()
            .map(|skill| skill.name)
            .collect())
    }

    pub fn plan(&self, target: Target, operation: Operation, dry_run: bool) -> Result<Plan> {
        self.plan_at(target, skills_root(target)?, operation, dry_run)
    }

    pub fn install(&self, target: Target, options: InstallOptions) -> Result<CommandResult> {
        self.install_to(target, skills_root(target)?, options)
    }

    pub fn install_many(
        &self,
        targets: impl IntoIterator<Item = Target>,
        options: InstallOptions,
    ) -> Result<Vec<CommandResult>> {
        let destinations = unique_targets(targets)
            .into_iter()
            .map(|target| Ok((target, skills_root(target)?)))
            .collect::<Result<Vec<_>>>()?;
        self.install_many_to(destinations, options)
    }

    pub fn install_many_to(
        &self,
        destinations: impl IntoIterator<Item = (Target, PathBuf)>,
        options: InstallOptions,
    ) -> Result<Vec<CommandResult>> {
        let destinations = unique_destinations(destinations)?;
        let _locks = if options.dry_run {
            None
        } else {
            Some(lock_roots(
                destinations.iter().map(|(_, root)| root.as_path()),
            )?)
        };
        let mut planned = Vec::new();
        for (targets, root) in &destinations {
            let target = targets[0];
            planned.push(self.install_to_locked_for(
                target,
                targets,
                root.clone(),
                InstallOptions {
                    dry_run: true,
                    ..options
                },
            )?);
        }
        if options.dry_run {
            return Ok(planned);
        }

        // All targets are proven eligible before the first snapshot or write.
        let mut snapshots = Vec::new();
        for (targets, root) in &destinations {
            snapshots.push(snapshot_target(targets[0], root)?);
        }
        let mut results = Vec::new();
        for (index, (targets, root)) in destinations.iter().enumerate() {
            match self.install_to_locked_for(targets[0], targets, root.clone(), options) {
                Ok(result) => {
                    for target in targets {
                        let mut target_result = result.clone();
                        target_result.plan.target = *target;
                        results.push(target_result);
                    }
                }
                Err(primary) => {
                    let failures = rollback_targets(&snapshots[..=index]);
                    return Err(transaction_failure(primary, failures));
                }
            }
        }
        Ok(results)
    }

    pub fn uninstall_many(
        &self,
        targets: impl IntoIterator<Item = Target>,
        dry_run: bool,
    ) -> Result<Vec<CommandResult>> {
        let destinations = unique_targets(targets)
            .into_iter()
            .map(|target| Ok((target, skills_root(target)?)))
            .collect::<Result<Vec<_>>>()?;
        self.uninstall_many_from(destinations, dry_run)
    }

    pub fn uninstall_many_from(
        &self,
        destinations: impl IntoIterator<Item = (Target, PathBuf)>,
        dry_run: bool,
    ) -> Result<Vec<CommandResult>> {
        let destinations = unique_destinations(destinations)?;
        let _locks = if dry_run {
            None
        } else {
            Some(lock_roots(
                destinations.iter().map(|(_, root)| root.as_path()),
            )?)
        };
        let mut planned = Vec::new();
        for (targets, root) in &destinations {
            planned.push(self.uninstall_from_locked_for(
                targets[0],
                targets,
                root.clone(),
                true,
            )?);
        }
        if dry_run {
            return Ok(planned);
        }
        let mut snapshots = Vec::new();
        for (targets, root) in &destinations {
            snapshots.push(snapshot_target(targets[0], root)?);
        }
        let mut results = Vec::new();
        for (index, (targets, root)) in destinations.iter().enumerate() {
            match self.uninstall_from_locked_for(targets[0], targets, root.clone(), false) {
                Ok(result) => {
                    for target in targets {
                        let mut target_result = result.clone();
                        target_result.plan.target = *target;
                        results.push(target_result);
                    }
                }
                Err(primary) => {
                    let failures = rollback_targets(&snapshots[..=index]);
                    return Err(transaction_failure(primary, failures));
                }
            }
        }
        Ok(results)
    }

    pub fn status(&self, target: Target) -> Result<CommandResult> {
        self.status_at(target, skills_root(target)?, Operation::Status)
    }

    pub fn doctor(&self, target: Target) -> Result<CommandResult> {
        self.status_at(target, skills_root(target)?, Operation::Doctor)
    }

    pub fn uninstall(&self, target: Target, dry_run: bool) -> Result<CommandResult> {
        self.uninstall_from(target, skills_root(target)?, dry_run)
    }

    pub fn plan_at(
        &self,
        target: Target,
        root: PathBuf,
        operation: Operation,
        dry_run: bool,
    ) -> Result<Plan> {
        Ok(plan_for(
            target,
            root,
            operation,
            dry_run,
            &discover_skills(&self.source)?,
        ))
    }

    pub fn install_to(
        &self,
        target: Target,
        root: PathBuf,
        options: InstallOptions,
    ) -> Result<CommandResult> {
        if options.dry_run {
            return self.install_to_locked(target, root, options);
        }
        let _locks = lock_roots([root.as_path()])?;
        self.install_to_locked(target, root, options)
    }

    fn install_to_locked(
        &self,
        target: Target,
        root: PathBuf,
        options: InstallOptions,
    ) -> Result<CommandResult> {
        self.install_to_locked_for(target, &[target], root, options)
    }

    fn install_to_locked_for(
        &self,
        target: Target,
        owners: &[Target],
        root: PathBuf,
        options: InstallOptions,
    ) -> Result<CommandResult> {
        let skills = discover_skills(&self.source)?;
        let plan = plan_for(
            target,
            root.clone(),
            Operation::Install,
            options.dry_run,
            &skills,
        );
        let previous = match inspect_for_install(&root, &skills)? {
            InstallState::Missing => None,
            InstallState::Collision(path) => return Err(RuntimeError::Collision(path)),
            InstallState::Drifted(path, _) => return Err(RuntimeError::Drift(path)),
            InstallState::Healthy(mut receipt) if receipt_matches_source(&receipt, &skills) => {
                let added = merge_receipt_owners(&mut receipt, owners);
                if added {
                    if !options.dry_run {
                        atomic_write_receipt(&root, &receipt)?;
                    }
                    return Ok(CommandResult {
                        plan,
                        outcome: if options.dry_run {
                            Outcome::Planned
                        } else {
                            Outcome::Updated
                        },
                        receipt: Some(receipt),
                        diagnostics: vec![
                            "attached additional target ownership to the healthy shared corpus"
                                .into(),
                        ],
                    });
                }
                return Ok(CommandResult {
                    plan,
                    outcome: if options.dry_run {
                        Outcome::Planned
                    } else {
                        Outcome::AlreadyCurrent
                    },
                    receipt: Some(receipt),
                    diagnostics: vec![
                        "installed corpus already matches the packaged skills".into(),
                    ],
                });
            }
            InstallState::Healthy(_) if !options.update => {
                return Err(RuntimeError::UpdateRequired(root));
            }
            InstallState::Healthy(receipt) => Some(receipt),
            InstallState::Unowned(_) => unreachable!("ownerless state requires a target check"),
        };
        let receipt = build_receipt(target, owners, &root, &skills);
        if options.dry_run {
            return Ok(CommandResult {
                plan,
                outcome: Outcome::Planned,
                receipt: Some(receipt),
                diagnostics: vec![],
            });
        }
        atomic_install_corpus(&skills, &root, previous.as_ref(), &receipt)?;
        Ok(CommandResult {
            plan,
            outcome: if previous.is_some() {
                Outcome::Updated
            } else {
                Outcome::Installed
            },
            receipt: Some(receipt),
            diagnostics: vec![],
        })
    }

    pub fn status_at(
        &self,
        target: Target,
        root: PathBuf,
        operation: Operation,
    ) -> Result<CommandResult> {
        let skills = discover_skills(&self.source)?;
        let plan = plan_for(target, root.clone(), operation, false, &skills);
        let (outcome, receipt, diagnostics) = match inspect(&root, Some(&skills), target)? {
            InstallState::Missing => (Outcome::Missing, None, vec![]),
            InstallState::Unowned(receipt) => (
                Outcome::Missing,
                Some(receipt),
                vec!["shared corpus exists but this target is not an owner".into()],
            ),
            InstallState::Collision(path) => (
                Outcome::Collision,
                None,
                vec![format!("unmanaged collision at {}", path.display())],
            ),
            InstallState::Drifted(path, receipt) => (
                Outcome::Drifted,
                Some(receipt),
                vec![format!("managed skill drift at {}", path.display())],
            ),
            InstallState::Healthy(receipt) => (Outcome::Healthy, Some(receipt), vec![]),
        };
        Ok(CommandResult {
            plan,
            outcome,
            receipt,
            diagnostics,
        })
    }

    pub fn uninstall_from(
        &self,
        target: Target,
        root: PathBuf,
        dry_run: bool,
    ) -> Result<CommandResult> {
        if dry_run {
            return self.uninstall_from_locked(target, root, dry_run);
        }
        let _locks = lock_roots([root.as_path()])?;
        self.uninstall_from_locked(target, root, dry_run)
    }

    fn uninstall_from_locked(
        &self,
        target: Target,
        root: PathBuf,
        dry_run: bool,
    ) -> Result<CommandResult> {
        self.uninstall_from_locked_for(target, &[target], root, dry_run)
    }

    fn uninstall_from_locked_for(
        &self,
        target: Target,
        owners: &[Target],
        root: PathBuf,
        dry_run: bool,
    ) -> Result<CommandResult> {
        let skills = discover_skills(&self.source)?;
        let plan = plan_for(target, root.clone(), Operation::Uninstall, dry_run, &skills);
        let receipt = match inspect(&root, None, target)? {
            InstallState::Missing => {
                return Ok(CommandResult {
                    plan,
                    outcome: if dry_run {
                        Outcome::Planned
                    } else {
                        Outcome::AlreadyAbsent
                    },
                    receipt: None,
                    diagnostics: vec![],
                });
            }
            InstallState::Unowned(receipt) => {
                return Ok(CommandResult {
                    plan,
                    outcome: if dry_run {
                        Outcome::Planned
                    } else {
                        Outcome::AlreadyAbsent
                    },
                    receipt: Some(receipt),
                    diagnostics: vec!["target was not an owner of the shared corpus".into()],
                });
            }
            InstallState::Collision(path) => return Err(RuntimeError::Collision(path)),
            InstallState::Drifted(path, _) => return Err(RuntimeError::Drift(path)),
            InstallState::Healthy(receipt) => receipt,
        };
        let remaining = receipt
            .targets
            .iter()
            .copied()
            .filter(|owner| !owners.contains(owner))
            .collect::<Vec<_>>();
        if !remaining.is_empty() {
            let mut updated = receipt;
            updated.targets = remaining;
            if !updated.targets.contains(&updated.target) {
                updated.target = updated.targets[0];
            }
            if !dry_run {
                atomic_write_receipt(&root, &updated)?;
            }
            return Ok(CommandResult {
                plan,
                outcome: if dry_run {
                    Outcome::Planned
                } else {
                    Outcome::Updated
                },
                receipt: Some(updated.clone()),
                diagnostics: vec![format!(
                    "detached target ownership ({}); {} target owner(s) remain: {}",
                    owners
                        .iter()
                        .map(|target| target.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    updated.targets.len(),
                    updated
                        .targets
                        .iter()
                        .map(|target| target.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )],
            });
        }
        let mut diagnostics = Vec::new();
        if !dry_run {
            match atomic_uninstall_corpus(&root, &receipt) {
                Ok(()) => {}
                Err(RuntimeError::CleanupRetained { path, .. }) => diagnostics.push(format!(
                    "uninstall committed; recoverable cleanup remains at {}",
                    path.display()
                )),
                Err(error) => return Err(error),
            }
        }
        Ok(CommandResult {
            plan,
            outcome: if dry_run {
                Outcome::Planned
            } else {
                Outcome::Removed
            },
            receipt: Some(receipt),
            diagnostics,
        })
    }
}

pub fn skills_root(target: Target) -> Result<PathBuf> {
    skills_root_with(target, &HostEnvironment::detect()?)
}

pub fn skills_root_with(target: Target, host: &HostEnvironment) -> Result<PathBuf> {
    let root = match target {
        Target::Codex => host
            .codex_home
            .clone()
            .unwrap_or_else(|| host.home.join(".codex")),
        Target::Claude => host
            .claude_config_dir
            .clone()
            .unwrap_or_else(|| host.home.join(".claude")),
        Target::Omp => omp_agent_root(host)?,
        Target::Pi => host
            .pi_coding_agent_dir
            .clone()
            .unwrap_or_else(|| host.home.join(".pi").join("agent")),
    };
    Ok(root.join("skills"))
}

pub fn skill_destination(target: Target) -> Result<PathBuf> {
    Ok(skills_root(target)?.join(SKILL_NAME))
}

struct RootLock {
    _file: File,
}

fn lock_roots<'a>(roots: impl IntoIterator<Item = &'a Path>) -> Result<Vec<RootLock>> {
    let lock_dir = ProjectDirs::from("com", "Thalix", "pstack")
        .map(|dirs| dirs.data_local_dir().join("locks"))
        .ok_or(RuntimeError::HomeUnavailable)?;
    fs::create_dir_all(&lock_dir).map_err(|error| io_error(&lock_dir, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&lock_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| io_error(&lock_dir, error))?;
    }

    let mut canonical = roots
        .into_iter()
        .map(canonical_target_path)
        .collect::<Result<Vec<_>>>()?;
    canonical.sort();
    canonical.dedup();
    let mut locks = Vec::new();
    for root in canonical {
        let mut digest = Sha256::new();
        digest.update(root.as_os_str().as_encoded_bytes());
        let path = lock_dir.join(format!("{}.lock", hex::encode(digest.finalize())));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| io_error(&path, error))?;
        file.lock_exclusive()
            .map_err(|error| io_error(&path, error))?;
        locks.push(RootLock { _file: file });
    }
    Ok(locks)
}

fn canonical_target_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|error| io_error(path, error));
    }
    let mut suffix = Vec::new();
    let mut ancestor = path;
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| io_error(path, io::Error::other("path has no existing ancestor")))?;
        suffix.push(name.to_owned());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| io_error(path, io::Error::other("path has no existing ancestor")))?;
    }
    let mut canonical = fs::canonicalize(ancestor).map_err(|error| io_error(ancestor, error))?;
    for part in suffix.into_iter().rev() {
        canonical.push(part);
    }
    Ok(canonical)
}

fn nonempty_path_env(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn omp_agent_root(host: &HostEnvironment) -> Result<PathBuf> {
    let selected = host.omp_profile.as_ref().or(host.pi_profile.as_ref());
    let profile = selected
        .map(|value| normalize_profile(value))
        .transpose()?
        .flatten();
    if let Some(profile) = profile {
        return Ok(host.home.join(".omp/profiles").join(profile).join("agent"));
    }

    // An explicitly default OMP profile must not inherit a PI_PROFILE-derived
    // agent dir propagated by a parent OMP process. A genuinely custom agent
    // override remains authoritative in default mode.
    let inherited_profile_dir = host.pi_profile.as_ref().and_then(|value| {
        normalize_profile(value)
            .ok()
            .flatten()
            .map(|profile| host.home.join(".omp/profiles").join(profile).join("agent"))
    });
    if let Some(agent) = &host.pi_coding_agent_dir {
        if inherited_profile_dir.as_ref() != Some(agent) {
            return Ok(agent.clone());
        }
    }
    Ok(host.home.join(".omp/agent"))
}

fn normalize_profile(value: &str) -> Result<Option<&str>> {
    let profile = value.trim();
    if profile.is_empty() || profile == "default" {
        return Ok(None);
    }
    let valid = profile.len() <= 64
        && !profile.ends_with('.')
        && profile.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        });
    if !valid || is_windows_reserved(profile) {
        return Err(RuntimeError::InvalidProfile(value.into()));
    }
    Ok(Some(profile))
}

fn is_windows_reserved(value: &str) -> bool {
    let base = value.split('.').next().unwrap_or(value);
    matches!(
        base.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
    ) || (base.len() == 4
        && matches!(&base[..3].to_ascii_uppercase(), prefix if prefix == "COM" || prefix == "LPT")
        && base.as_bytes()[3].is_ascii_digit())
}

fn discover_skills(source: &Path) -> Result<Vec<SourceSkill>> {
    let mut skills = Vec::new();
    for entry in fs::read_dir(source).map_err(|error| io_error(source, error))? {
        let entry = entry.map_err(|error| io_error(source, error))?;
        let kind = entry
            .file_type()
            .map_err(|error| io_error(entry.path(), error))?;
        if kind.is_symlink() {
            return Err(RuntimeError::Symlink(entry.path()));
        }
        if !kind.is_dir() || !entry.path().join("SKILL.md").is_file() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| RuntimeError::InvalidSkillName(entry.path()))?;
        if !is_safe_skill_name(&name) {
            return Err(RuntimeError::InvalidSkillName(entry.path()));
        }
        skills.push(SourceSkill {
            name,
            sha256: directory_sha256(&entry.path())?,
            path: entry.path(),
        });
    }
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    if skills.is_empty() {
        return Err(RuntimeError::EmptySource(source.to_owned()));
    }
    Ok(skills)
}

fn plan_for(
    target: Target,
    root: PathBuf,
    operation: Operation,
    dry_run: bool,
    skills: &[SourceSkill],
) -> Plan {
    let risk = match operation {
        Operation::Status | Operation::Doctor => Risk::ReadOnly,
        Operation::Install => Risk::UserScopedWrite,
        Operation::Uninstall => Risk::ProtectedDelete,
    };
    let verb = match operation {
        Operation::Install => "copy",
        Operation::Status => "inspect",
        Operation::Doctor => "verify",
        Operation::Uninstall => "remove unchanged",
    };
    Plan {
        operation,
        target,
        destination: root.clone(),
        dry_run,
        actions: skills
            .iter()
            .map(|skill| Action {
                description: format!("{verb} skill {}", skill.name),
                path: root.join(&skill.name),
                risk,
            })
            .collect(),
    }
}

enum InstallState {
    Missing,
    Unowned(Receipt),
    Collision(PathBuf),
    Healthy(Receipt),
    Drifted(PathBuf, Receipt),
}
fn state_directory(root: &Path) -> PathBuf {
    root.join(STATE_DIRECTORY)
}
fn receipt_path(root: &Path) -> PathBuf {
    state_directory(root).join(RECEIPT_FILE)
}

fn inspect(root: &Path, source: Option<&[SourceSkill]>, target: Target) -> Result<InstallState> {
    inspect_internal(root, source, Some(target))
}

fn inspect_for_install(root: &Path, source: &[SourceSkill]) -> Result<InstallState> {
    inspect_internal(root, Some(source), None)
}

fn inspect_internal(
    root: &Path,
    source: Option<&[SourceSkill]>,
    required_target: Option<Target>,
) -> Result<InstallState> {
    let path = receipt_path(root);
    let state = state_directory(root);
    if let Ok(metadata) = fs::symlink_metadata(&state) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Ok(InstallState::Collision(state));
        }
    }
    if !path.exists() {
        if state.exists() {
            return Ok(InstallState::Collision(state));
        }
        if let Some(source) = source {
            if let Some(skill) = source.iter().find(|skill| root.join(&skill.name).exists()) {
                return Ok(InstallState::Collision(root.join(&skill.name)));
            }
        }
        return Ok(InstallState::Missing);
    }
    let receipt_metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
    if receipt_metadata.file_type().is_symlink() || !receipt_metadata.is_file() {
        return Ok(InstallState::Collision(path));
    }
    let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
    let receipt: Receipt =
        serde_json::from_slice(&bytes).map_err(|source| RuntimeError::InvalidReceipt {
            path: path.clone(),
            source,
        })?;
    if !valid_receipt_header(&receipt, root, required_target) {
        return Ok(InstallState::Collision(path));
    }
    let canonical_root = canonical_directory(root)?;
    let mut names = HashSet::new();
    let mut prior_name: Option<&str> = None;
    for skill in &receipt.skills {
        if !is_safe_skill_name(&skill.name)
            || !is_sha256(&skill.sha256)
            || !names.insert(skill.name.as_str())
            || prior_name.is_some_and(|prior| prior >= skill.name.as_str())
        {
            return Ok(InstallState::Collision(path));
        }
        prior_name = Some(&skill.name);

        // Receipt paths are provenance only. Derive every accessed path from
        // a validated single-component name under the trusted root.
        let destination = root.join(&skill.name);
        if skill.destination != destination {
            return Ok(InstallState::Collision(path));
        }
        if !is_direct_contained_directory(&canonical_root, &destination)?
            || directory_sha256(&destination)? != skill.sha256
        {
            return Ok(InstallState::Drifted(destination, receipt));
        }
    }
    if receipt.skills.is_empty() || bundle_sha256(&receipt.skills) != receipt.installed_sha256 {
        return Ok(InstallState::Collision(path));
    }
    if let Some(source) = source {
        for skill in source {
            let destination = root.join(&skill.name);
            if destination.exists() && !receipt.skills.iter().any(|old| old.name == skill.name) {
                return Ok(InstallState::Collision(destination));
            }
        }
    }
    if required_target.is_some_and(|target| !receipt.targets.contains(&target)) {
        Ok(InstallState::Unowned(receipt))
    } else {
        Ok(InstallState::Healthy(receipt))
    }
}

fn receipt_matches_source(receipt: &Receipt, source: &[SourceSkill]) -> bool {
    receipt.skills.len() == source.len()
        && receipt
            .skills
            .iter()
            .zip(source)
            .all(|(a, b)| a.name == b.name && a.sha256 == b.sha256)
}

fn merge_receipt_owners(receipt: &mut Receipt, owners: &[Target]) -> bool {
    let original = receipt.targets.clone();
    receipt.targets = Target::ALL
        .into_iter()
        .filter(|target| original.contains(target) || owners.contains(target))
        .collect();
    receipt.targets != original
}

fn atomic_write_receipt(root: &Path, receipt: &Receipt) -> Result<()> {
    let state = state_directory(root);
    let mut staged = NamedTempFile::new_in(&state).map_err(|error| io_error(&state, error))?;
    staged
        .write_all(&serde_json::to_vec_pretty(receipt).expect("receipt serialization cannot fail"))
        .map_err(|error| io_error(staged.path(), error))?;
    staged
        .as_file()
        .sync_all()
        .map_err(|error| io_error(staged.path(), error))?;
    staged.persist(receipt_path(root)).map_err(|error| {
        let path = error.file.path().to_owned();
        io_error(path, error.error)
    })?;
    Ok(())
}

fn valid_receipt_header(receipt: &Receipt, root: &Path, _target: Option<Target>) -> bool {
    receipt.schema_version == 3
        && receipt.package == SKILL_NAME
        && !receipt.targets.is_empty()
        && receipt
            .targets
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
            == receipt.targets.len()
        && receipt.targets.contains(&receipt.target)
        && receipt.destination == root
        && is_sha256(&receipt.source_sha256)
        && receipt.source_sha256 == receipt.installed_sha256
}

fn is_safe_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(RuntimeError::Symlink(path.to_owned()));
    }
    fs::canonicalize(path).map_err(|error| io_error(path, error))
}

fn is_direct_contained_directory(canonical_root: &Path, path: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error(path, error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(false);
    }
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    Ok(canonical.parent() == Some(canonical_root) && canonical.starts_with(canonical_root))
}

fn bundle_sha256(skills: &[SkillReceipt]) -> String {
    let mut aggregate = Sha256::new();
    for skill in skills {
        aggregate.update((skill.name.len() as u64).to_le_bytes());
        aggregate.update(skill.name.as_bytes());
        aggregate.update(skill.sha256.as_bytes());
    }
    hex::encode(aggregate.finalize())
}

fn build_receipt(
    target: Target,
    targets: &[Target],
    root: &Path,
    skills: &[SourceSkill],
) -> Receipt {
    let installed = skills
        .iter()
        .map(|skill| SkillReceipt {
            name: skill.name.clone(),
            destination: root.join(&skill.name),
            sha256: skill.sha256.clone(),
        })
        .collect::<Vec<_>>();
    let sha = bundle_sha256(&installed);
    Receipt {
        schema_version: 3,
        package: SKILL_NAME.into(),
        package_version: env!("CARGO_PKG_VERSION").into(),
        target,
        targets: targets.to_vec(),
        destination: root.to_owned(),
        source_sha256: sha.clone(),
        installed_sha256: sha,
        installed_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        skills: installed,
    }
}

fn atomic_install_corpus(
    skills: &[SourceSkill],
    root: &Path,
    previous: Option<&Receipt>,
    receipt: &Receipt,
) -> Result<()> {
    fs::create_dir_all(root).map_err(|error| io_error(root, error))?;
    let tx = Builder::new()
        .prefix(".pstack-transaction-")
        .tempdir_in(root)
        .map_err(|error| io_error(root, error))?;
    let staged = tx.path().join("staged");
    let backup = tx.path().join("backup");
    fs::create_dir(&staged).map_err(|error| io_error(&staged, error))?;
    fs::create_dir(&backup).map_err(|error| io_error(&backup, error))?;
    for skill in skills {
        copy_tree(&skill.path, &staged.join(&skill.name))?;
    }

    // Finish every fallible staging write before moving installed content.
    let staged_state = tx.path().join("state");
    fs::create_dir(&staged_state).map_err(|error| io_error(&staged_state, error))?;
    let staged_receipt = staged_state.join(RECEIPT_FILE);
    fs::write(
        &staged_receipt,
        serde_json::to_vec_pretty(receipt).expect("receipt serialization cannot fail"),
    )
    .map_err(|error| io_error(&staged_receipt, error))?;

    let state = state_directory(root);
    let old_state = if state.exists() {
        let old = tx.path().join("old-state");
        fs::rename(&state, &old).map_err(|error| io_error(&state, error))?;
        Some(old)
    } else {
        None
    };
    let mut moved = Vec::new();
    if let Some(previous) = previous {
        for skill in &previous.skills {
            let to = backup.join(&skill.name);
            let installed_path = root.join(&skill.name);
            if let Err(error) = fs::rename(&installed_path, &to) {
                let mut failures = rollback(&[], &moved);
                failures.extend(restore_state(old_state.as_deref(), &state));
                return Err(transaction_failure(
                    io_error(&installed_path, error),
                    failures,
                ));
            }
            moved.push((to, installed_path));
        }
    }
    let mut installed = Vec::new();
    for skill in skills {
        let to = root.join(&skill.name);
        if let Err(error) = fs::rename(staged.join(&skill.name), &to) {
            let mut failures = rollback(&installed, &moved);
            failures.extend(restore_state(old_state.as_deref(), &state));
            return Err(transaction_failure(io_error(&to, error), failures));
        }
        installed.push(to);
    }
    if let Err(error) = fs::rename(&staged_state, &state) {
        let mut failures = rollback(&installed, &moved);
        failures.extend(restore_state(old_state.as_deref(), &state));
        return Err(transaction_failure(io_error(&state, error), failures));
    }
    tx.close().map_err(|error| io_error(root, error))
}

fn restore_state(old_state: Option<&Path>, state: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(old) = old_state {
        if let Err(error) = fs::rename(old, state) {
            failures.push(io_error(state, error).to_string());
        }
    }
    failures
}

fn rollback(installed: &[PathBuf], moved: &[(PathBuf, PathBuf)]) -> Vec<String> {
    let mut failures = Vec::new();
    for path in installed.iter().rev() {
        if let Err(error) = fs::remove_dir_all(path) {
            failures.push(io_error(path, error).to_string());
        }
    }
    for (backup, original) in moved.iter().rev() {
        if let Err(error) = fs::rename(backup, original) {
            failures.push(io_error(original, error).to_string());
        }
    }
    failures
}

fn transaction_failure(primary: RuntimeError, rollback_failures: Vec<String>) -> RuntimeError {
    if rollback_failures.is_empty() {
        primary
    } else {
        let mut failures = vec![primary.to_string()];
        failures.extend(rollback_failures);
        RuntimeError::Transaction { failures }
    }
}

fn atomic_uninstall_corpus(root: &Path, receipt: &Receipt) -> Result<()> {
    atomic_uninstall_corpus_with(
        root,
        receipt,
        |from, to| fs::rename(from, to),
        cleanup_transaction,
    )
}

#[cfg(test)]
thread_local! {
    static INJECT_CLEANUP_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn cleanup_transaction(path: &Path) -> io::Result<()> {
    #[cfg(test)]
    if INJECT_CLEANUP_FAILURE.with(|enabled| enabled.get()) {
        let skills = path.join("skills");
        if let Some(entry) = fs::read_dir(&skills)?.next() {
            fs::remove_dir_all(entry?.path())?;
        }
        return Err(io::Error::other("injected partial cleanup failure"));
    }
    fs::remove_dir_all(path)
}

fn atomic_uninstall_corpus_with(
    root: &Path,
    receipt: &Receipt,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    cleanup: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<()> {
    let tx = Builder::new()
        .prefix(".pstack-uninstall-")
        .tempdir_in(root)
        .map_err(|error| io_error(root, error))?;
    let backup = tx.path().join("skills");
    fs::create_dir(&backup).map_err(|error| io_error(&backup, error))?;
    let mut moved = Vec::new();
    for skill in &receipt.skills {
        let source = root.join(&skill.name);
        let destination = backup.join(&skill.name);
        if let Err(error) = rename(&source, &destination) {
            let failures = rollback(&[], &moved);
            return Err(transaction_failure(io_error(&source, error), failures));
        }
        moved.push((destination, source));
    }

    let state = state_directory(root);
    let backup_state = tx.path().join("state");
    if let Err(error) = rename(&state, &backup_state) {
        let failures = rollback(&[], &moved);
        return Err(transaction_failure(io_error(&state, error), failures));
    }

    // Nothing remains at a managed destination. Deletion now operates only on
    // the transaction backup, never on live paths. At this point absence is
    // committed: cleanup failure retains a recoverable backup and must not try
    // to reconstruct live state from a potentially partially deleted tree.
    let tx_path = tx.keep();
    if let Err(source) = cleanup(&tx_path) {
        return Err(RuntimeError::CleanupRetained {
            path: tx_path,
            source,
        });
    }
    Ok(())
}

struct TargetSnapshot {
    target: Target,
    root: PathBuf,
    receipt: Option<Receipt>,
    backup: Option<TempDir>,
}

fn unique_targets(targets: impl IntoIterator<Item = Target>) -> Vec<Target> {
    let mut seen = HashSet::new();
    targets
        .into_iter()
        .filter(|target| seen.insert(*target))
        .collect()
}

fn unique_destinations(
    destinations: impl IntoIterator<Item = (Target, PathBuf)>,
) -> Result<Vec<(Vec<Target>, PathBuf)>> {
    let mut seen = HashSet::new();
    let mut unique: Vec<(Vec<Target>, PathBuf, PathBuf)> = Vec::new();
    for (target, root) in destinations {
        if !seen.insert(target) {
            continue;
        }
        let canonical = canonical_target_path(&root)?;
        if let Some((targets, _, _)) = unique
            .iter_mut()
            .find(|(_, _, existing)| *existing == canonical)
        {
            targets.push(target);
        } else {
            unique.push((vec![target], root, canonical));
        }
    }
    Ok(unique
        .into_iter()
        .map(|(targets, root, _)| (targets, root))
        .collect())
}

fn snapshot_target(target: Target, root: &Path) -> Result<TargetSnapshot> {
    let receipt = match inspect(root, None, target)? {
        InstallState::Missing => None,
        InstallState::Unowned(receipt) => Some(receipt),
        InstallState::Healthy(receipt) => Some(receipt),
        InstallState::Collision(path) => return Err(RuntimeError::Collision(path)),
        InstallState::Drifted(path, _) => return Err(RuntimeError::Drift(path)),
    };
    let Some(receipt) = receipt else {
        return Ok(TargetSnapshot {
            target,
            root: root.to_owned(),
            receipt: None,
            backup: None,
        });
    };
    let backup = Builder::new()
        .prefix(".pstack-multi-backup-")
        .tempdir_in(root)
        .map_err(|error| io_error(root, error))?;
    let skills = backup.path().join("skills");
    fs::create_dir(&skills).map_err(|error| io_error(&skills, error))?;
    for skill in &receipt.skills {
        copy_tree(&root.join(&skill.name), &skills.join(&skill.name))?;
    }
    copy_tree(&state_directory(root), &backup.path().join("state"))?;
    Ok(TargetSnapshot {
        target,
        root: root.to_owned(),
        receipt: Some(receipt),
        backup: Some(backup),
    })
}

fn rollback_targets(snapshots: &[TargetSnapshot]) -> Vec<String> {
    let mut failures = Vec::new();
    for snapshot in snapshots.iter().rev() {
        if let Err(error) = restore_target(snapshot) {
            failures.push(error.to_string());
        }
    }
    failures
}

fn restore_target(snapshot: &TargetSnapshot) -> Result<()> {
    match inspect(&snapshot.root, None, snapshot.target)? {
        InstallState::Healthy(receipt) => atomic_uninstall_corpus(&snapshot.root, &receipt)?,
        InstallState::Missing => {}
        InstallState::Unowned(receipt) => atomic_uninstall_corpus(&snapshot.root, &receipt)?,
        InstallState::Collision(path) => return Err(RuntimeError::Collision(path)),
        InstallState::Drifted(path, _) => return Err(RuntimeError::Drift(path)),
    }
    let (Some(receipt), Some(backup)) = (&snapshot.receipt, &snapshot.backup) else {
        return Ok(());
    };
    let mut restored = Vec::new();
    for skill in &receipt.skills {
        let source = backup.path().join("skills").join(&skill.name);
        let destination = snapshot.root.join(&skill.name);
        if let Err(error) = fs::rename(&source, &destination) {
            for path in restored.iter().rev() {
                let _ = fs::remove_dir_all(path);
            }
            return Err(io_error(destination, error));
        }
        restored.push(destination);
    }
    if let Err(error) = fs::rename(backup.path().join("state"), state_directory(&snapshot.root)) {
        let failures = rollback(&restored, &[]);
        return Err(transaction_failure(
            io_error(state_directory(&snapshot.root), error),
            failures,
        ));
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination).map_err(|error| io_error(destination, error))?;
    for entry in WalkDir::new(source).min_depth(1).follow_links(false) {
        let entry = entry.map_err(|error| io_error(source, io::Error::other(error)))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .expect("walker entry below source");
        let output = destination.join(relative);
        if entry.file_type().is_symlink() {
            return Err(RuntimeError::Symlink(entry.path().to_owned()));
        }
        if entry.file_type().is_dir() {
            fs::create_dir(&output).map_err(|error| io_error(&output, error))?;
        } else if entry.file_type().is_file() {
            fs::copy(entry.path(), &output).map_err(|error| io_error(&output, error))?;
        }
    }
    Ok(())
}

pub fn directory_sha256(root: &Path) -> Result<String> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).min_depth(1).follow_links(false) {
        let entry = entry.map_err(|error| io_error(root, io::Error::other(error)))?;
        if entry.file_type().is_symlink() {
            return Err(RuntimeError::Symlink(entry.path().to_owned()));
        }
        if entry.file_type().is_file() || entry.file_type().is_dir() {
            entries.push(entry.path().to_owned());
        }
    }
    entries.sort_by(|a, b| {
        a.strip_prefix(root)
            .unwrap()
            .as_os_str()
            .as_encoded_bytes()
            .cmp(b.strip_prefix(root).unwrap().as_os_str().as_encoded_bytes())
    });
    let mut digest = Sha256::new();
    for path in entries {
        let relative = path.strip_prefix(root).expect("entry below root");
        let bytes = relative.as_os_str().as_encoded_bytes();
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.is_dir() {
            digest.update(b"directory");
            continue;
        }
        digest.update(b"regular-file");
        digest.update(executable_bits(&metadata).to_le_bytes());
        let mut file = fs::File::open(&path).map_err(|error| io_error(&path, error))?;
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| io_error(&path, error))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
    }
    Ok(hex::encode(digest.finalize()))
}

#[cfg(unix)]
fn executable_bits(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111
}

#[cfg(not(unix))]
fn executable_bits(_metadata: &fs::Metadata) -> u32 {
    0
}

fn io_error(path: impl Into<PathBuf>, source: io::Error) -> RuntimeError {
    RuntimeError::Io {
        path: path.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Installer) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        for name in ["pstack", "architect"] {
            let skill = source.join(name);
            fs::create_dir_all(&skill).unwrap();
            fs::write(skill.join("SKILL.md"), format!("# {name}\n")).unwrap();
        }
        (temp, Installer::new(source).unwrap())
    }
    fn root(temp: &TempDir) -> PathBuf {
        temp.path().join("target-skills")
    }

    fn host(home: &Path) -> HostEnvironment {
        HostEnvironment {
            home: home.to_owned(),
            codex_home: None,
            claude_config_dir: None,
            pi_coding_agent_dir: None,
            omp_profile: None,
            pi_profile: None,
        }
    }

    #[test]
    fn host_paths_honor_overrides_and_omp_profile_precedence() {
        let home = PathBuf::from("/portable/home");
        let mut environment = host(&home);
        environment.codex_home = Some(PathBuf::from("/config/codex"));
        environment.claude_config_dir = Some(PathBuf::from("/config/claude"));
        environment.pi_coding_agent_dir = Some(PathBuf::from("/agents/custom"));
        assert_eq!(
            skills_root_with(Target::Codex, &environment).unwrap(),
            PathBuf::from("/config/codex/skills")
        );
        assert_eq!(
            skills_root_with(Target::Claude, &environment).unwrap(),
            PathBuf::from("/config/claude/skills")
        );
        assert_eq!(
            skills_root_with(Target::Pi, &environment).unwrap(),
            PathBuf::from("/agents/custom/skills")
        );
        assert_eq!(
            skills_root_with(Target::Omp, &environment).unwrap(),
            PathBuf::from("/agents/custom/skills")
        );

        environment.omp_profile = Some("work".into());
        environment.pi_profile = Some("legacy".into());
        assert_eq!(
            skills_root_with(Target::Omp, &environment).unwrap(),
            home.join(".omp/profiles/work/agent/skills")
        );

        environment.omp_profile = Some(String::new());
        environment.pi_coding_agent_dir = Some(home.join(".omp/profiles/legacy/agent"));
        assert_eq!(
            skills_root_with(Target::Omp, &environment).unwrap(),
            home.join(".omp/agent/skills")
        );
        environment.pi_coding_agent_dir = Some(PathBuf::from("/agents/genuinely-custom"));
        assert_eq!(
            skills_root_with(Target::Omp, &environment).unwrap(),
            PathBuf::from("/agents/genuinely-custom/skills")
        );
        environment.omp_profile = Some("../escape".into());
        assert!(matches!(
            skills_root_with(Target::Omp, &environment),
            Err(RuntimeError::InvalidProfile(_))
        ));
    }

    #[test]
    fn dry_run_plans_each_skill_without_side_effects() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        let result = installer
            .install_to(
                Target::Codex,
                root.clone(),
                InstallOptions {
                    dry_run: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(result.outcome, Outcome::Planned);
        assert_eq!(result.plan.actions.len(), 2);
        assert!(!root.exists());
        serde_json::to_string(&result).unwrap();
    }

    #[test]
    fn full_corpus_lifecycle_is_idempotent_and_healthy() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        assert_eq!(
            installer
                .install_to(Target::Claude, root.clone(), InstallOptions::default())
                .unwrap()
                .outcome,
            Outcome::Installed
        );
        assert!(root.join("pstack/SKILL.md").is_file());
        assert!(root.join("architect/SKILL.md").is_file());
        assert!(receipt_path(&root).is_file());
        assert_eq!(
            installer
                .status_at(Target::Claude, root.clone(), Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );
        assert_eq!(
            installer
                .install_to(Target::Claude, root.clone(), InstallOptions::default())
                .unwrap()
                .outcome,
            Outcome::AlreadyCurrent
        );
        assert_eq!(
            installer
                .uninstall_from(Target::Claude, root.clone(), false)
                .unwrap()
                .outcome,
            Outcome::Removed
        );
        assert_eq!(
            installer
                .uninstall_from(Target::Claude, root, false)
                .unwrap()
                .outcome,
            Outcome::AlreadyAbsent
        );
    }

    #[test]
    fn collision_preflight_prevents_partial_install() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        fs::create_dir_all(root.join("architect")).unwrap();
        fs::write(root.join("architect/mine.txt"), "keep").unwrap();
        assert!(matches!(
            installer.install_to(Target::Pi, root.clone(), InstallOptions::default()),
            Err(RuntimeError::Collision(_))
        ));
        assert!(!root.join("pstack").exists());
        assert_eq!(
            fs::read_to_string(root.join("architect/mine.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn multi_target_collision_preflight_leaves_eligible_target_untouched() {
        let (temp, installer) = fixture();
        let eligible = temp.path().join("codex-skills");
        let collision = temp.path().join("claude-skills");
        fs::create_dir_all(collision.join("architect")).unwrap();
        fs::write(collision.join("architect/mine.txt"), "keep").unwrap();
        assert!(matches!(
            installer.install_many_to(
                [
                    (Target::Codex, eligible.clone()),
                    (Target::Claude, collision.clone()),
                ],
                InstallOptions::default()
            ),
            Err(RuntimeError::Collision(_))
        ));
        assert!(!eligible.exists());
        assert_eq!(
            fs::read_to_string(collision.join("architect/mine.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn drift_blocks_update_and_uninstall_of_all_skills() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        installer
            .install_to(Target::Omp, root.clone(), InstallOptions::default())
            .unwrap();
        fs::write(root.join("architect/SKILL.md"), "local edit\n").unwrap();
        assert_eq!(
            installer
                .status_at(Target::Omp, root.clone(), Operation::Doctor)
                .unwrap()
                .outcome,
            Outcome::Drifted
        );
        assert!(matches!(
            installer.install_to(
                Target::Omp,
                root.clone(),
                InstallOptions {
                    update: true,
                    ..Default::default()
                }
            ),
            Err(RuntimeError::Drift(_))
        ));
        assert!(matches!(
            installer.uninstall_from(Target::Omp, root.clone(), false),
            Err(RuntimeError::Drift(_))
        ));
        assert!(root.join("pstack").exists());
    }

    #[test]
    fn uninstall_restores_every_skill_when_a_move_fails() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        let installed = installer
            .install_to(Target::Codex, root.clone(), InstallOptions::default())
            .unwrap();
        let receipt = installed.receipt.unwrap();
        let mut calls = 0;
        let result = atomic_uninstall_corpus_with(
            &root,
            &receipt,
            |from, to| {
                calls += 1;
                if calls == 2 {
                    Err(io::Error::other("injected move failure"))
                } else {
                    fs::rename(from, to)
                }
            },
            |path| fs::remove_dir_all(path),
        );
        assert!(result.is_err());
        assert!(root.join("architect/SKILL.md").is_file());
        assert!(root.join("pstack/SKILL.md").is_file());
        assert_eq!(
            installer
                .status_at(Target::Codex, root, Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );
    }

    #[cfg(unix)]
    #[test]
    fn executable_permission_changes_are_integrity_drift() {
        use std::os::unix::fs::PermissionsExt;

        let (temp, installer) = fixture();
        let root = root(&temp);
        installer
            .install_to(Target::Codex, root.clone(), InstallOptions::default())
            .unwrap();
        let skill = root.join("pstack/SKILL.md");
        let mut permissions = fs::metadata(&skill).unwrap().permissions();
        permissions.set_mode(permissions.mode() ^ 0o100);
        fs::set_permissions(&skill, permissions).unwrap();
        assert_eq!(
            installer
                .status_at(Target::Codex, root, Operation::Doctor)
                .unwrap()
                .outcome,
            Outcome::Drifted
        );
    }

    #[test]
    fn cleanup_failure_commits_absence_and_retains_recovery_path() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        let receipt = installer
            .install_to(Target::Codex, root.clone(), InstallOptions::default())
            .unwrap()
            .receipt
            .unwrap();
        let error = atomic_uninstall_corpus_with(
            &root,
            &receipt,
            |from, to| fs::rename(from, to),
            |path| {
                fs::remove_dir_all(path.join("skills/architect"))?;
                Err(io::Error::other("injected partial cleanup failure"))
            },
        )
        .unwrap_err();
        let RuntimeError::CleanupRetained { path, .. } = error else {
            panic!("expected retained cleanup error");
        };
        assert!(path.exists());
        assert!(!root.join("architect").exists());
        assert!(!root.join("pstack").exists());
        assert!(!state_directory(&root).exists());
        assert_eq!(
            installer
                .status_at(Target::Codex, root, Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Missing
        );
    }

    #[test]
    fn concurrent_identical_installs_leave_one_healthy_install() {
        use std::sync::{Arc, Barrier};

        let (temp, installer) = fixture();
        let root = root(&temp);
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let installer = installer.clone();
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                installer.install_to(Target::Codex, root, InstallOptions::default())
            }));
        }
        barrier.wait();
        let outcomes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap().outcome)
            .collect::<Vec<_>>();
        assert!(outcomes.contains(&Outcome::Installed));
        assert!(outcomes.contains(&Outcome::AlreadyCurrent));
        assert_eq!(
            installer
                .status_at(Target::Codex, root, Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );
    }

    #[test]
    fn shared_omp_pi_root_is_installed_once_and_healthy_for_both() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        let results = installer
            .install_many_to(
                [(Target::Omp, root.clone()), (Target::Pi, root.clone())],
                InstallOptions::default(),
            )
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            installer
                .status_at(Target::Omp, root.clone(), Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );
        assert_eq!(
            installer
                .status_at(Target::Pi, root.clone(), Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );
        installer
            .uninstall_from(Target::Pi, root.clone(), false)
            .unwrap();
        assert_eq!(
            installer
                .status_at(Target::Omp, root.clone(), Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );
        installer.uninstall_from(Target::Omp, root, false).unwrap();
    }

    #[test]
    fn incremental_shared_owner_attach_and_detach_preserves_corpus() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        installer
            .install_to(Target::Omp, root.clone(), InstallOptions::default())
            .unwrap();
        let attached = installer
            .install_many_to(
                [(Target::Omp, root.clone()), (Target::Pi, root.clone())],
                InstallOptions::default(),
            )
            .unwrap();
        assert!(
            attached
                .iter()
                .all(|result| result.outcome == Outcome::Updated)
        );
        assert_eq!(
            installer
                .status_at(Target::Pi, root.clone(), Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );

        let detached = installer
            .uninstall_from(Target::Omp, root.clone(), false)
            .unwrap();
        assert_eq!(detached.outcome, Outcome::Updated);
        assert!(detached.diagnostics[0].contains("pi"));
        assert!(root.join("pstack/SKILL.md").is_file());
        assert_eq!(
            installer
                .status_at(Target::Pi, root.clone(), Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );
        assert_eq!(
            installer
                .status_at(Target::Omp, root.clone(), Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Missing
        );
        installer
            .uninstall_from(Target::Pi, root.clone(), false)
            .unwrap();
        assert_eq!(
            installer
                .status_at(Target::Pi, root, Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Missing
        );
    }

    #[test]
    fn multi_uninstall_treats_retained_cleanup_as_committed_success() {
        let (temp, installer) = fixture();
        let first = temp.path().join("first-skills");
        let second = temp.path().join("second-skills");
        installer
            .install_many_to(
                [
                    (Target::Codex, first.clone()),
                    (Target::Claude, second.clone()),
                ],
                InstallOptions::default(),
            )
            .unwrap();
        INJECT_CLEANUP_FAILURE.with(|enabled| enabled.set(true));
        let results = installer.uninstall_many_from(
            [
                (Target::Codex, first.clone()),
                (Target::Claude, second.clone()),
            ],
            false,
        );
        INJECT_CLEANUP_FAILURE.with(|enabled| enabled.set(false));
        let results = results.unwrap();
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|result| result.outcome == Outcome::Removed)
        );
        assert!(results.iter().all(|result| {
            result
                .diagnostics
                .iter()
                .any(|line| line.contains("cleanup remains"))
        }));
        assert_eq!(
            installer
                .status_at(Target::Codex, first, Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Missing
        );
        assert_eq!(
            installer
                .status_at(Target::Claude, second, Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Missing
        );
    }

    #[test]
    fn crafted_receipt_cannot_delete_outside_the_skills_root() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        let victim = temp.path().join("victim");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("keep.txt"), "must survive").unwrap();
        fs::create_dir_all(state_directory(&root)).unwrap();

        let victim_sha = directory_sha256(&victim).unwrap();
        let malicious_skill = SkillReceipt {
            name: "../../victim".into(),
            destination: victim.clone(),
            sha256: victim_sha,
        };
        let aggregate = bundle_sha256(std::slice::from_ref(&malicious_skill));
        let malicious = Receipt {
            schema_version: 3,
            package: SKILL_NAME.into(),
            package_version: env!("CARGO_PKG_VERSION").into(),
            target: Target::Codex,
            targets: vec![Target::Codex],
            destination: root.clone(),
            source_sha256: aggregate.clone(),
            installed_sha256: aggregate,
            installed_at_unix_seconds: 0,
            skills: vec![malicious_skill],
        };
        fs::write(receipt_path(&root), serde_json::to_vec(&malicious).unwrap()).unwrap();

        assert!(matches!(
            installer.uninstall_from(Target::Codex, root, false),
            Err(RuntimeError::Collision(_))
        ));
        assert_eq!(
            fs::read_to_string(victim.join("keep.txt")).unwrap(),
            "must survive"
        );
    }

    #[test]
    fn update_reconciles_complete_managed_set() {
        let (temp, installer) = fixture();
        let root = root(&temp);
        installer
            .install_to(Target::Codex, root.clone(), InstallOptions::default())
            .unwrap();
        fs::remove_dir_all(installer.source().join("architect")).unwrap();
        let new = installer.source().join("why");
        fs::create_dir(&new).unwrap();
        fs::write(new.join("SKILL.md"), "# why\n").unwrap();
        assert!(matches!(
            installer.install_to(Target::Codex, root.clone(), InstallOptions::default()),
            Err(RuntimeError::UpdateRequired(_))
        ));
        assert_eq!(
            installer
                .install_to(
                    Target::Codex,
                    root.clone(),
                    InstallOptions {
                        update: true,
                        ..Default::default()
                    }
                )
                .unwrap()
                .outcome,
            Outcome::Updated
        );
        assert!(!root.join("architect").exists());
        assert!(root.join("why/SKILL.md").is_file());
        assert_eq!(
            installer
                .status_at(Target::Codex, root, Operation::Status)
                .unwrap()
                .outcome,
            Outcome::Healthy
        );
    }

    #[test]
    fn directory_hash_is_stable_across_creation_order() {
        let temp = tempfile::tempdir().unwrap();
        let left = temp.path().join("left");
        let right = temp.path().join("right");
        fs::create_dir_all(left.join("nested")).unwrap();
        fs::write(left.join("a"), "a").unwrap();
        fs::write(left.join("nested/b"), "b").unwrap();
        fs::create_dir_all(right.join("nested")).unwrap();
        fs::write(right.join("nested/b"), "b").unwrap();
        fs::write(right.join("a"), "a").unwrap();
        assert_eq!(
            directory_sha256(&left).unwrap(),
            directory_sha256(&right).unwrap()
        );
    }
}
