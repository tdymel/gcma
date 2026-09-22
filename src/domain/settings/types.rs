//! Plain configuration values (what the user can choose).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCfg {
    #[serde(default = "default_days")]
    pub days: Vec<String>,
    #[serde(default = "default_hours")]
    pub hours: String,
    #[serde(default)]
    pub distribution: Distribution,
    #[serde(default)]
    pub seed: u64,
}

pub(super) fn default_days() -> Vec<String> {
    ["mon", "tue", "wed", "thu", "fri"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub(super) fn default_hours() -> String {
    "09:00-18:00".to_string()
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Distribution {
    #[default]
    Uniform,
    WeekdayWeighted,
    Bursty,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchSpec {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSpec {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityRule {
    #[serde(rename = "match")]
    pub match_: MatchSpec,
    pub set: SetSpec,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesCfg {
    #[serde(default)]
    pub strip_trailers: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Signing {
    #[default]
    Strip,
    Resign,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HookMode {
    #[default]
    Verify,
    Rewrite,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookCfg {
    #[serde(default)]
    pub mode: HookMode,
}

/// Which implementation performs the hot object operations (batch reads, existence checks and
/// commit writes). Everything else (refs, config, signing, hooks, ...) always uses the git CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Spawn `git` for every operation (one process per written commit).
    Git,
    /// In-process object access through gitoxide (needs the `gix` cargo feature).
    #[default]
    Gix,
}

impl std::str::FromStr for Backend {
    type Err = crate::domain::error::Error;
    fn from_str(s: &str) -> crate::domain::error::Result<Backend> {
        match s.trim().to_ascii_lowercase().as_str() {
            "git" => Ok(Backend::Git),
            "gix" => Ok(Backend::Gix),
            other => Err(crate::domain::error::Error::Usage(format!(
                "unknown backend {other:?} (expected `git` or `gix`)"
            ))),
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Backend::Git => "git",
            Backend::Gix => "gix",
        })
    }
}
