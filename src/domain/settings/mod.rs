//! User settings: the validated, in-memory form of `.git-hide-my-ass.yml`.
//! Reading the file lives in an adapter; this module only knows the rules.

mod calendar;
mod types;

use chrono_tz::Tz;
use serde::Deserialize;

use crate::domain::error::{Error, Result};
use crate::domain::paths::{GITIGNORE, PathFilter};
use crate::domain::text::messages;

use calendar::parse_instant;
pub use calendar::{parse_days, parse_hours, weekday_index};
#[cfg(test)]
pub use types::SetSpec;
pub use types::{
    Backend, Distribution, HookCfg, HookMode, IdentityRule, MatchSpec, MessagesCfg, OnlyExcluded,
    PathsCfg, ScheduleCfg, Signing,
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
    pub paths: PathsCfg,
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
            paths: PathsCfg::default(),
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
            if !matches!(self.to.as_deref(), None | Some("now"))
                && self.resolve_to(0)? <= self.resolve_from()?
            {
                return bad("`to` is not after `from`".into());
            }
        }
        if self.from.is_some() {
            self.resolve_from()?;
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
            // These would corrupt the commit header the identity is written into.
            let forbidden = |c: char| matches!(c, '\n' | '\r' | '\0' | '<' | '>');
            if r.set.name.contains(forbidden) || r.set.email.contains(forbidden) {
                return bad(format!(
                    "identity rule {i}: name and email must not contain line breaks, NUL, `<` or `>`"
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
        self.path_filter()?;
        for line in &self.messages.add_trailers {
            let Some(key) = messages::trailer_key(line.as_bytes()) else {
                return bad(format!(
                    "messages.add_trailers: {line:?} is not a `Key: value` trailer"
                ));
            };
            if line.contains(['\n', '\r']) || line[key.len() + 1..].trim().is_empty() {
                return bad(format!(
                    "messages.add_trailers: {line:?} must be a single line with a value"
                ));
            }
            if self
                .messages
                .strip_trailers
                .iter()
                .any(|s| s.as_bytes().eq_ignore_ascii_case(key))
            {
                return bad(format!(
                    "messages.add_trailers: {line:?} would be removed again by `strip_trailers`"
                ));
            }
        }
        Ok(())
    }

    /// True when no rule is configured, so nothing would ever be rewritten.
    pub fn is_inert(&self) -> bool {
        self.schedule.is_none()
            && self.identity.is_empty()
            && self.messages.strip_trailers.is_empty()
            && self.messages.add_trailers.is_empty()
            && self.paths.exclude.is_empty()
            && self.signing == Signing::Strip
    }

    /// The filter for `paths.exclude`, `None` when nothing is excluded.
    pub fn path_filter(&self) -> Result<Option<PathFilter>> {
        let patterns = &self.paths.exclude;
        if patterns.is_empty() {
            return Ok(None);
        }
        for p in patterns {
            if p.trim() != p || p.is_empty() || p.starts_with('#') {
                return Err(Error::Usage(format!(
                    "paths.exclude: {p:?} must not be empty, start with `#` or have surrounding whitespace"
                )));
            }
        }
        let filter = PathFilter::new(patterns)?;
        if filter.excludes(GITIGNORE, false) {
            return Err(Error::Usage(
                "paths.exclude: the patterns would exclude `.gitignore` itself".into(),
            ));
        }
        Ok(Some(filter))
    }

    /// The message after the configured trailer rules.
    pub fn rewrite_message(&self, msg: &[u8]) -> Vec<u8> {
        messages::apply_trailer_rules(
            msg,
            &self.messages.strip_trailers,
            &self.messages.add_trailers,
        )
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

    /// `from` as a UTC instant; a bare date means the start of that day.
    pub fn resolve_from(&self) -> Result<i64> {
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
