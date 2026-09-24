//! Plain configuration values (what the user can choose).

use serde::{Deserialize, Serialize};

use crate::domain::text::messages::TrailerRewrite;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCfg {
    #[serde(default = "default_days")]
    pub days: Vec<String>,
    #[serde(default = "default_hours")]
    pub hours: Hours,
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

pub(super) fn default_hours() -> Hours {
    Hours::from("09:00-18:00")
}

/// Working hours: one `HH:MM-HH:MM` range or a list of them. A range that ends at or before its
/// start runs past midnight into the next day (`18:00-06:00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hours(pub Vec<String>);

impl From<&str> for Hours {
    fn from(range: &str) -> Hours {
        Hours(vec![range.to_string()])
    }
}

impl<'de> Deserialize<'de> for Hours {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Hours, D::Error> {
        use serde::de::{self, SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Hours;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a range like \"09:00-18:00\" or a list of them")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Hours, E> {
                Ok(Hours::from(v))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Hours, A::Error> {
                let mut ranges = Vec::new();
                while let Some(r) = seq.next_element::<String>()? {
                    ranges.push(r);
                }
                Ok(Hours(ranges))
            }
        }
        d.deserialize_any(V)
    }
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
    /// Full `Key: value` lines appended when no such trailer is present yet.
    #[serde(default)]
    pub add_trailers: Vec<String>,
    /// sed-like rewrites of the trailer lines (`match` regex, `replace` template), run first.
    #[serde(default, deserialize_with = "compile_rewrites")]
    pub rewrite_trailers: Vec<TrailerRewrite>,
    /// Drop the message body; the subject and the trailer block stay.
    #[serde(default)]
    pub title_only: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRewrite {
    #[serde(rename = "match")]
    pattern: String,
    replace: String,
}

/// Compiles every `match` while the config is read, so a bad regex is a config error up front.
fn compile_rewrites<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<TrailerRewrite>, D::Error> {
    let raw = Vec::<RawRewrite>::deserialize(d)?;
    raw.into_iter()
        .enumerate()
        .map(|(i, r)| {
            let pattern = regex::bytes::Regex::new(&r.pattern).map_err(|e| {
                serde::de::Error::custom(format!("messages.rewrite_trailers[{i}].match: {e}"))
            })?;
            Ok(TrailerRewrite {
                pattern,
                replacement: r.replace,
            })
        })
        .collect()
}

/// What happens to a commit that touched nothing but excluded paths.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OnlyExcluded {
    /// The commit is removed from history.
    #[default]
    Drop,
    /// The commit stays, as an empty commit.
    Keep,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathsCfg {
    /// gitignore-style patterns of the paths to remove from every rewritten commit.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Add the patterns to `.gitignore`, starting with the first kept commit that had such paths,
    /// so the files stay in the working copy but out of git.
    #[serde(default = "default_true")]
    pub gitignore: bool,
    #[serde(default)]
    pub only_excluded_commits: OnlyExcluded,
}

impl Default for PathsCfg {
    fn default() -> Self {
        PathsCfg {
            exclude: Vec::new(),
            gitignore: true,
            only_excluded_commits: OnlyExcluded::Drop,
        }
    }
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
