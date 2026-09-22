//! User settings: the validated, in-memory form of `.git-hide-my-ass.yml`.
//! Reading the file lives in an adapter; this module only knows the rules.

mod calendar;
mod types;

use chrono_tz::Tz;
use serde::Deserialize;

use crate::domain::error::{Error, Result};

use calendar::parse_instant;
pub use calendar::{parse_days, parse_hours, weekday_index};
pub use types::{
    Backend, Distribution, HookCfg, HookMode, IdentityRule, MatchSpec, MessagesCfg, ScheduleCfg,
    SetSpec, Signing,
};

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
    pub backend: Option<Backend>,
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
