use std::path::Path;

use chrono::{NaiveDate, TimeZone, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const CONFIG_FILE: &str = ".git-hide-my-ass.yml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub from: Option<String>,
    pub to: Option<String>,
    pub timezone: Option<String>,
    pub schedule: Option<ScheduleCfg>,
    #[serde(default)]
    pub identity: Vec<IdentityRule>,
    #[serde(default)]
    pub messages: MessagesCfg,
    #[serde(default)]
    pub signing: Signing,
    #[serde(default)]
    pub hook: HookCfg,
    /// Object backend (`git` or `gix`); `--backend` and `GHMA_BACKEND` take precedence.
    pub backend: Option<crate::git::Backend>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 1,
            from: None,
            to: None,
            timezone: None,
            schedule: None,
            identity: Vec::new(),
            messages: MessagesCfg::default(),
            signing: Signing::Strip,
            hook: HookCfg::default(),
            backend: None,
        }
    }
}

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

fn default_days() -> Vec<String> {
    ["mon", "tue", "wed", "thu", "fri"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn default_hours() -> String {
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

impl MatchSpec {
    fn matches(&self, name: &str, email: &str) -> bool {
        self.name.as_deref().is_none_or(|n| n == name)
            && self
                .email
                .as_deref()
                .is_none_or(|e| e.to_lowercase() == email.to_lowercase())
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Usage(format!("cannot read {}: {e}", path.display())))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Config> {
        let cfg: Config = serde_yaml::from_str(text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(Error::Usage(format!("config: {m}")));
        if self.version != 1 {
            return bad(format!("unsupported version {} (expected 1)", self.version));
        }
        self.tz()?;
        if let Some(s) = &self.schedule {
            if self.from.is_none() {
                return bad("`from` is required when `schedule` is present".into());
            }
            parse_days(&s.days)?;
            parse_hours(&s.hours)?;
        }
        if self.from.is_some() {
            self.from_utc()?;
        }
        for (i, r) in self.identity.iter().enumerate() {
            if r.match_.name.is_none() && r.match_.email.is_none() {
                return bad(format!(
                    "identity rule {i}: `match` needs a name or an email"
                ));
            }
            if r.set.name.is_empty() || r.set.email.is_empty() {
                return bad(format!(
                    "identity rule {i}: `set` needs both name and email"
                ));
            }
        }
        // Fixed point: applying the rules to any rule's output must leave it unchanged.
        for (i, r) in self.identity.iter().enumerate() {
            if let Some((n, e)) = self.map_identity(&r.set.name, &r.set.email)
                && (n != r.set.name || e != r.set.email)
            {
                return bad(format!(
                    "identity rule {i}: its output ({} <{}>) is rewritten again by another rule",
                    r.set.name, r.set.email
                ));
            }
        }
        for k in &self.messages.strip_trailers {
            if k.trim().is_empty() || k.contains(':') {
                return bad(format!("messages.strip_trailers: invalid key {k:?}"));
            }
        }
        Ok(())
    }

    pub fn tz(&self) -> Result<Tz> {
        match &self.timezone {
            None => Ok(chrono_tz::UTC),
            Some(s) => s
                .parse::<Tz>()
                .map_err(|_| Error::Usage(format!("config: unknown timezone {s:?}"))),
        }
    }

    /// First matching identity rule (first-match-wins), if any.
    pub fn map_identity(&self, name: &str, email: &str) -> Option<(String, String)> {
        self.identity
            .iter()
            .find(|r| r.match_.matches(name, email))
            .map(|r| (r.set.name.clone(), r.set.email.clone()))
    }

    pub fn from_utc(&self) -> Result<i64> {
        let s = self
            .from
            .as_deref()
            .ok_or_else(|| Error::Usage("config: `from` is not set".into()))?;
        parse_instant(s, &self.tz()?, false)
    }

    /// `to`: absent or "now" resolves to `now`; a bare date means the end of that day.
    pub fn resolve_to(&self, now: i64) -> Result<i64> {
        match self.to.as_deref() {
            None | Some("now") => Ok(now),
            Some(s) => parse_instant(s, &self.tz()?, true),
        }
    }
}

fn parse_instant(s: &str, tz: &Tz, end_of_day: bool) -> Result<i64> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let d = if end_of_day {
            d.succ_opt().unwrap_or(d)
        } else {
            d
        };
        let naive = d.and_hms_opt(0, 0, 0).unwrap();
        return match tz.from_local_datetime(&naive) {
            chrono::LocalResult::Single(t) => Ok(t.timestamp()),
            chrono::LocalResult::Ambiguous(a, _) => Ok(a.timestamp()),
            chrono::LocalResult::None => Ok(tz.from_utc_datetime(&naive).timestamp()),
        };
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|t| t.timestamp())
        .map_err(|_| {
            Error::Usage(format!(
                "config: cannot parse date {s:?} (use YYYY-MM-DD or RFC 3339)"
            ))
        })
}

pub fn parse_days(days: &[String]) -> Result<Vec<Weekday>> {
    if days.is_empty() {
        return Err(Error::Usage("config: schedule.days is empty".into()));
    }
    days.iter()
        .map(|d| {
            d.parse::<Weekday>()
                .map_err(|_| Error::Usage(format!("config: unknown weekday {d:?}")))
        })
        .collect()
}

/// "HH:MM-HH:MM" -> (start_minute, end_minute); end may be 24:00.
pub fn parse_hours(s: &str) -> Result<(u32, u32)> {
    let bad = || Error::Usage(format!("config: bad hours {s:?} (expected HH:MM-HH:MM)"));
    let (a, b) = s.split_once('-').ok_or_else(bad)?;
    let p = |x: &str| -> Option<u32> {
        let (h, m) = x.trim().split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        (m < 60 && (h < 24 || (h == 24 && m == 0))).then_some(h * 60 + m)
    };
    let (start, end) = (p(a).ok_or_else(bad)?, p(b).ok_or_else(bad)?);
    if start >= end {
        return Err(Error::Usage(format!(
            "config: hours {s:?}: start must be before end"
        )));
    }
    Ok((start, end))
}

pub fn weekday_index(w: Weekday) -> usize {
    w.num_days_from_monday() as usize
}

pub fn starter_config() -> &'static str {
    "# ghma configuration. See DESIGN.md.\n\
version: 1\n\
from: 2026-04-01        # schedule lower bound (required when `schedule` is set)\n\
to: now                 # `now` or a date\n\
timezone: UTC           # IANA name, e.g. Europe/Berlin\n\
schedule:\n\
\x20 days: [mon, tue, wed, thu, fri]\n\
\x20 hours: \"09:30-18:00\"\n\
\x20 distribution: uniform   # uniform | weekday-weighted | bursty\n\
\x20 seed: 42\n\
# identity:\n\
#   - match: { email: me@home.org }\n\
#     set:   { name: Jane Doe, email: jane@work.com }\n\
messages:\n\
\x20 strip_trailers: []\n\
signing: strip          # strip | resign\n\
hook:\n\
\x20 mode: verify          # verify | rewrite\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let c = Config::parse(
            "version: 1\nfrom: 2026-04-01\nto: now\ntimezone: Europe/Berlin\nschedule:\n  hours: \"09:30-18:00\"\n  distribution: bursty\n  seed: 7\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\nsigning: resign\nhook: {mode: rewrite}\n",
        )
        .unwrap();
        assert_eq!(
            c.schedule.as_ref().unwrap().distribution,
            Distribution::Bursty
        );
        assert_eq!(c.signing, Signing::Resign);
        assert_eq!(c.hook.mode, HookMode::Rewrite);
        assert_eq!(
            c.map_identity("x", "ME@home.org"),
            Some(("Jane Doe".to_string(), "jane@work.com".to_string()))
        );
    }

    #[test]
    fn rejects_unknown_fields_and_bad_version() {
        assert!(Config::parse("version: 1\nlast: 6mo\n").is_err());
        assert!(Config::parse("version: 2\n").is_err());
        assert!(Config::parse("{}").is_err());
    }

    #[test]
    fn schedule_requires_from() {
        assert!(Config::parse("version: 1\nschedule: {}\n").is_err());
        assert!(Config::parse("version: 1\nfrom: 2026-01-01\nschedule: {}\n").is_ok());
    }

    #[test]
    fn identity_fixed_point_validated() {
        // A -> B, B -> C is not a fixed point.
        let chain = "version: 1\nidentity:\n  - match: {email: a@x}\n    set: {name: B, email: b@x}\n  - match: {email: b@x}\n    set: {name: C, email: c@x}\n";
        assert!(Config::parse(chain).is_err());
        // Mapping to itself under its own rule is fine.
        let selfmap =
            "version: 1\nidentity:\n  - match: {email: a@x}\n    set: {name: New, email: a@x}\n";
        assert!(Config::parse(selfmap).is_ok());
        // Partial set rejected.
        assert!(
            Config::parse("version: 1\nidentity:\n  - match: {email: a@x}\n    set: {name: N}\n")
                .is_err()
        );
        // Empty match rejected.
        assert!(
            Config::parse("version: 1\nidentity:\n  - match: {}\n    set: {name: N, email: e@x}\n")
                .is_err()
        );
    }

    #[test]
    fn hours_parse() {
        assert_eq!(parse_hours("09:30-18:00").unwrap(), (570, 1080));
        assert_eq!(parse_hours("00:00-24:00").unwrap(), (0, 1440));
        assert!(parse_hours("18:00-09:00").is_err());
        assert!(parse_hours("9-18").is_err());
        assert!(parse_hours("09:00-25:00").is_err());
    }

    #[test]
    fn to_resolution() {
        let c = Config::parse("version: 1\nto: 2026-01-02\n").unwrap();
        assert_eq!(c.resolve_to(5).unwrap(), 1767398400); // 2026-01-03T00:00Z, end of Jan 2
        let c = Config::parse("version: 1\nto: now\n").unwrap();
        assert_eq!(c.resolve_to(5).unwrap(), 5);
        let c = Config::parse("version: 1\nfrom: 2026-01-01\n").unwrap();
        assert_eq!(c.from_utc().unwrap(), 1767225600);
    }

    #[test]
    fn starter_config_is_valid() {
        Config::parse(starter_config()).unwrap();
    }
}
