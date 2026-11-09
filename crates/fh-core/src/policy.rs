//! Policy (`policy.toml`, user-local and authoritative) and repo config
//! (`firsthand.toml`, committed, untrusted, suggestions only).

use crate::error::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Full policy. Missing sections take defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// Command execution rules.
    pub exec: ExecPolicy,
    /// Witness behavior.
    pub witness: WitnessPolicy,
    /// Snapshot limits and exclusions.
    pub snapshot: SnapshotPolicy,
    /// Brief settings.
    pub brief: BriefPolicy,
    /// Packet limits.
    pub packet: PacketPolicy,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            exec: ExecPolicy::default(),
            witness: WitnessPolicy::default(),
            snapshot: SnapshotPolicy::default(),
            brief: BriefPolicy {
                budget_tokens: 6000,
            },
            packet: PacketPolicy {
                max_bytes: 52_428_800,
            },
        }
    }
}

/// Command execution policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ExecPolicy {
    /// Allowed argv prefixes.
    pub allow: Vec<Vec<String>>,
    /// Program names that are never run.
    pub deny_always: Vec<String>,
    /// Whether shell probes are possible at all (still need approval).
    pub allow_shell: bool,
    /// Default timeout in seconds.
    pub timeout_s: u32,
    /// Output cap in bytes.
    pub max_output_bytes: u64,
    /// Extra environment variable names passed to probes.
    pub env_passthrough: Vec<String>,
    /// Auto-run allowlisted commands for own and trusted packets.
    pub auto_run_for_trusted_packets: bool,
}

fn argvs(v: &[&[&str]]) -> Vec<Vec<String>> {
    v.iter()
        .map(|a| a.iter().map(|s| (*s).to_owned()).collect())
        .collect()
}

impl Default for ExecPolicy {
    fn default() -> Self {
        ExecPolicy {
            allow: argvs(&[
                &["cargo", "test"],
                &["cargo", "build"],
                &["cargo", "clippy"],
                &["npm", "test"],
                &["npm", "run", "test"],
                &["pnpm", "test"],
                &["pytest"],
                &["python", "-m", "pytest"],
                &["py", "-m", "pytest"],
                &["go", "test"],
            ]),
            deny_always: [
                "sudo",
                "rm",
                "del",
                "curl",
                "wget",
                "Invoke-WebRequest",
                "powershell",
                "pwsh",
                "cmd",
                "sh",
                "bash",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
            allow_shell: false,
            timeout_s: 300,
            max_output_bytes: 1_048_576,
            env_passthrough: Vec::new(),
            auto_run_for_trusted_packets: true,
        }
    }
}

/// Default environment variables passed to probe commands.
pub const SAFE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
    "SystemRoot",
    "COMSPEC",
    "LANG",
    "LC_ALL",
    "CI",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NODE_ENV",
];

/// Witness policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WitnessPolicy {
    /// Extra verification argv prefixes, added to the built-in defaults.
    pub verification_patterns: Vec<Vec<String>>,
    /// Take an auto snapshot on Stop when the tree changed.
    pub auto_snapshot_on_stop: bool,
    /// Rotating auto snapshots to keep.
    pub auto_snapshots_keep: u32,
}

impl Default for WitnessPolicy {
    fn default() -> Self {
        WitnessPolicy {
            verification_patterns: Vec::new(),
            auto_snapshot_on_stop: true,
            auto_snapshots_keep: 20,
        }
    }
}

/// Snapshot policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SnapshotPolicy {
    /// Files larger than this are omitted.
    pub max_file_bytes: u64,
    /// Globs excluded as likely secrets.
    pub exclude_globs: Vec<String>,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        SnapshotPolicy {
            max_file_bytes: 5_242_880,
            exclude_globs: [
                "**/.env",
                "**/.env.*",
                "**/*.pem",
                "**/*.key",
                "**/id_rsa*",
                "**/id_ed25519*",
                "**/*.p12",
                "**/*.pfx",
                "**/credentials*",
                "**/.npmrc",
                "**/.pypirc",
                "**/secrets.*",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        }
    }
}

/// Brief policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BriefPolicy {
    /// Budget in estimated tokens (`ceil(bytes / 4)`).
    pub budget_tokens: u32,
}

impl Default for BriefPolicy {
    fn default() -> Self {
        BriefPolicy {
            budget_tokens: 6000,
        }
    }
}

/// Packet policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PacketPolicy {
    /// Maximum packet size.
    pub max_bytes: u64,
}

impl Default for PacketPolicy {
    fn default() -> Self {
        PacketPolicy {
            max_bytes: 52_428_800,
        }
    }
}

/// Result of checking an argv against the exec policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecDecision {
    /// argv matches an allow prefix and no deny rule.
    Allowed,
    /// argv hits `deny_always`. Never runs, even with approval.
    Denied(String),
    /// Not on the allowlist. Needs explicit approval from a terminal.
    NeedsApproval,
}

/// Program name for matching: file name without directory and without a
/// Windows executable extension, compared case-insensitively.
pub fn program_key(arg0: &str) -> String {
    let base = arg0.rsplit(['/', '\\']).next().unwrap_or(arg0);
    let lower = base.to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".com", ".ps1"] {
        if let Some(s) = lower.strip_suffix(ext) {
            return s.to_owned();
        }
    }
    lower
}

impl Policy {
    /// Load the effective policy: defaults, then the user-level file, then the repo-local file.
    /// Each file replaces whole sections it defines. Missing files are fine; bad files are errors.
    pub fn load(user_level: Option<&Path>, repo_local: Option<&Path>) -> Result<Policy> {
        let mut table = toml::Table::new();
        for p in [user_level, repo_local].into_iter().flatten() {
            match std::fs::read_to_string(p) {
                Ok(s) => {
                    let t: toml::Table = toml::from_str(&s).map_err(|e| Error::Config {
                        path: p.to_owned(),
                        reason: e.to_string(),
                    })?;
                    for (k, v) in t {
                        match (table.get_mut(&k), v) {
                            (Some(toml::Value::Table(old)), toml::Value::Table(new)) => {
                                old.extend(new);
                            }
                            (_, v) => {
                                table.insert(k, v);
                            }
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(p, e)),
            }
        }
        let path = repo_local
            .or(user_level)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        toml::Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| Error::Config {
                path,
                reason: e.to_string(),
            })
    }

    /// Decide whether an argv may run. Matching is by whole argv elements, never by substring.
    pub fn exec_decision(&self, argv: &[String]) -> ExecDecision {
        let Some(arg0) = argv.first() else {
            return ExecDecision::Denied("empty argv".to_owned());
        };
        let prog = program_key(arg0);
        if self.exec.deny_always.iter().any(|d| program_key(d) == prog) {
            return ExecDecision::Denied(format!("{prog} is in exec.deny_always"));
        }
        let matches = self.exec.allow.iter().any(|prefix| {
            !prefix.is_empty()
                && prefix.len() <= argv.len()
                && program_key(&prefix[0]) == prog
                && prefix[1..].iter().zip(&argv[1..]).all(|(a, b)| a == b)
        });
        if matches {
            ExecDecision::Allowed
        } else {
            ExecDecision::NeedsApproval
        }
    }
}

/// Repo-level `firsthand.toml`. A cloned repo is untrusted input, so this file can
/// only suggest probes and scopes. Any allowlist, approval, trust or env key is
/// rejected by `deny_unknown_fields`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RepoConfig {
    /// Suggested probes, shown to the user, never auto-trusted.
    pub probes: Vec<SuggestedProbe>,
    /// Default scope globs for `run` and `claim add`.
    pub default_scopes: Vec<String>,
}

/// A suggested probe in `firsthand.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestedProbe {
    /// Label.
    pub name: String,
    /// argv, still subject to the user's policy.
    pub argv: Vec<String>,
    /// Optional scope globs.
    #[serde(default)]
    pub scope: Vec<String>,
}

impl RepoConfig {
    /// Parse `firsthand.toml` text.
    pub fn parse(text: &str, path: &Path) -> Result<RepoConfig> {
        toml::from_str(text).map_err(|e| Error::Config {
            path: path.to_owned(),
            reason: format!("{e} (firsthand.toml may only define probes and default_scopes)"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn exec_matching() {
        let p = Policy::default();
        assert_eq!(
            p.exec_decision(&v(&["cargo", "test", "-p", "x"])),
            ExecDecision::Allowed
        );
        assert_eq!(
            p.exec_decision(&v(&["C:\\bin\\Cargo.EXE", "test"])),
            ExecDecision::Allowed
        );
        assert_eq!(
            p.exec_decision(&v(&["cargo", "testx"])),
            ExecDecision::NeedsApproval
        );
        assert_eq!(p.exec_decision(&v(&["cargo"])), ExecDecision::NeedsApproval);
        assert_eq!(
            p.exec_decision(&v(&["cargox", "test"])),
            ExecDecision::NeedsApproval
        );
        assert!(matches!(
            p.exec_decision(&v(&["/bin/rm", "-rf", "/"])),
            ExecDecision::Denied(_)
        ));
        assert!(matches!(
            p.exec_decision(&v(&["bash", "-c", "cargo test"])),
            ExecDecision::Denied(_)
        ));
        assert!(matches!(p.exec_decision(&[]), ExecDecision::Denied(_)));
    }

    #[test]
    fn load_merges_and_rejects_unknown() {
        let d = std::env::temp_dir().join(format!("fhpol-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let u = d.join("user.toml");
        let l = d.join("local.toml");
        std::fs::write(&u, "[brief]\nbudget_tokens = 100\n[exec]\ntimeout_s = 5\n").unwrap();
        std::fs::write(&l, "[exec]\ntimeout_s = 9\n").unwrap();
        let p = Policy::load(Some(&u), Some(&l)).unwrap();
        assert_eq!(p.brief.budget_tokens, 100);
        assert_eq!(p.exec.timeout_s, 9);
        assert_eq!(p.exec.allow, ExecPolicy::default().allow);
        std::fs::write(&l, "[exec]\nbogus = 1\n").unwrap();
        assert!(Policy::load(None, Some(&l)).is_err());
        assert_eq!(
            Policy::load(None, Some(&d.join("missing.toml"))).unwrap(),
            Policy::default()
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn repo_config_cannot_grant() {
        let p = Path::new("firsthand.toml");
        assert!(RepoConfig::parse("[exec]\nallow = [[\"rm\"]]\n", p).is_err());
        assert!(RepoConfig::parse("env_passthrough = [\"AWS_SECRET\"]\n", p).is_err());
        assert!(RepoConfig::parse("trusted = true\n", p).is_err());
        let ok = RepoConfig::parse(
            "default_scopes = [\"src/**\"]\n[[probes]]\nname = \"t\"\nargv = [\"cargo\", \"test\"]\n",
            p,
        )
        .unwrap();
        assert_eq!(ok.probes.len(), 1);
    }
}
