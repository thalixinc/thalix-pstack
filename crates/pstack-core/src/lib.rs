//! Stable, JSON-serializable contracts shared by the pstack CLI and runtime.

use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, str::FromStr};

/// A supported agent skill host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Codex,
    Claude,
    Omp,
    Pi,
}

impl Target {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::Omp, Self::Pi];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Omp => "omp",
            Self::Pi => "pi",
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Target {
    type Err = ParseTargetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "omp" => Ok(Self::Omp),
            "pi" => Ok(Self::Pi),
            _ => Err(ParseTargetError(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTargetError(pub String);

impl fmt::Display for ParseTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported target {:?}; expected codex, claude, omp, or pi",
            self.0
        )
    }
}

impl std::error::Error for ParseTargetError {}

/// AXI risk attached to an intended filesystem action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    ReadOnly,
    UserScopedWrite,
    ProtectedOverwrite,
    ProtectedDelete,
}

/// The operation represented by a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Install,
    Status,
    Doctor,
    Uninstall,
}

/// A single inspectable step in an AXI plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub description: String,
    pub path: PathBuf,
    pub risk: Risk,
}

/// An explicit, serializable description of intended effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub operation: Operation,
    pub target: Target,
    pub destination: PathBuf,
    pub dry_run: bool,
    pub actions: Vec<Action>,
}

/// The terminal state of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Planned,
    Installed,
    Updated,
    AlreadyCurrent,
    Healthy,
    Missing,
    Drifted,
    Collision,
    Removed,
    AlreadyAbsent,
}

/// Installation provenance and integrity data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillReceipt {
    pub name: String,
    pub destination: PathBuf,
    pub sha256: String,
}

/// Installation provenance and integrity data for a complete pstack corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema_version: u32,
    pub package: String,
    pub package_version: String,
    pub target: Target,
    pub targets: Vec<Target>,
    pub destination: PathBuf,
    pub source_sha256: String,
    pub installed_sha256: String,
    pub installed_at_unix_seconds: u64,
    pub skills: Vec<SkillReceipt>,
}

/// Uniform result envelope used by mutating and read-only commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResult {
    pub plan: Plan,
    pub outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_round_trips_and_accepts_case() {
        for target in Target::ALL {
            assert_eq!(target.as_str().parse(), Ok(target));
        }
        assert_eq!("CoDeX".parse(), Ok(Target::Codex));
    }

    #[test]
    fn result_is_json_serializable_with_stable_names() {
        let result = CommandResult {
            plan: Plan {
                operation: Operation::Install,
                target: Target::Omp,
                destination: PathBuf::from("/tmp/skills/pstack"),
                dry_run: true,
                actions: vec![],
            },
            outcome: Outcome::Planned,
            receipt: None,
            diagnostics: vec![],
        };
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["plan"]["operation"], "install");
        assert_eq!(json["plan"]["target"], "omp");
        assert_eq!(json["outcome"], "planned");
        assert!(json.get("receipt").is_none());
    }
}
