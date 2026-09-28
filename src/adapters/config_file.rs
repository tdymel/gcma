//! Reads `gcma.yml` into the domain `Config`.

use std::path::Path;

use crate::domain::error::{Error, Result};
use crate::domain::settings::Config;

pub const CONFIG_FILE: &str = "gcma.yml";

pub fn load(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::Usage(format!("cannot read {}: {e}", path.display())))?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<Config> {
    let cfg: Config = serde_yaml_ng::from_str(text)?;
    cfg.validate()?;
    Ok(cfg)
}

pub fn starter_config() -> &'static str {
    include_str!("starter_config.yml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::{Distribution, HookMode, Signing};

    #[test]
    fn parses_full_config() {
        let c = parse(
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
        assert!(parse("version: 1\nlast: 6mo\n").is_err());
        assert!(parse("version: 2\n").is_err());
        assert!(parse("{}").is_err());
    }

    #[test]
    fn bad_hours_say_what_is_expected() {
        let e = parse("version: 1\nfrom: 2026-01-01\nschedule:\n  hours: 5\n").unwrap_err();
        assert!(
            e.to_string()
                .contains("a range like \"09:00-18:00\" or a list of them"),
            "{e}"
        );
        assert!(
            parse("version: 1\nfrom: 2026-01-01\nschedule:\n  hours: [\"09:00-12:00\", \"13:00-18:00\"]\n")
                .is_ok()
        );
    }

    #[test]
    fn schedule_requires_from() {
        assert!(parse("version: 1\nschedule: {}\n").is_err());
        assert!(parse("version: 1\nfrom: 2026-01-01\nschedule: {}\n").is_ok());
    }

    #[test]
    fn identity_fixed_point_validated() {
        // A -> B, B -> C is not a fixed point.
        let chain = "version: 1\nidentity:\n  - match: {email: a@x}\n    set: {name: B, email: b@x}\n  - match: {email: b@x}\n    set: {name: C, email: c@x}\n";
        assert!(parse(chain).is_err());
        // Mapping to itself under its own rule is fine.
        let selfmap =
            "version: 1\nidentity:\n  - match: {email: a@x}\n    set: {name: New, email: a@x}\n";
        assert!(parse(selfmap).is_ok());
        // Partial set rejected.
        assert!(
            parse("version: 1\nidentity:\n  - match: {email: a@x}\n    set: {name: N}\n").is_err()
        );
        // Empty match rejected.
        assert!(
            parse("version: 1\nidentity:\n  - match: {}\n    set: {name: N, email: e@x}\n")
                .is_err()
        );
    }

    #[test]
    fn to_resolution() {
        let c = parse("version: 1\nto: 2026-01-02\n").unwrap();
        assert_eq!(c.resolve_to(5).unwrap(), 1767398400); // 2026-01-03T00:00Z, end of Jan 2
        let c = parse("version: 1\nto: now\n").unwrap();
        assert_eq!(c.resolve_to(5).unwrap(), 5);
        let c = parse("version: 1\nfrom: 2026-01-01\n").unwrap();
        assert_eq!(c.resolve_from().unwrap(), 1767225600);
    }

    #[test]
    fn path_rules_parse_and_validate() {
        let c = parse(
            "version: 1\npaths:\n  exclude: [\"secrets/\", \"*.env\"]\n  only_excluded_commits: keep\n  gitignore: false\n",
        )
        .unwrap();
        assert_eq!(c.paths.exclude.len(), 2);
        assert!(!c.paths.gitignore);
        assert_eq!(
            c.paths.only_excluded_commits,
            crate::domain::settings::OnlyExcluded::Keep
        );
        assert!(c.path_filter().unwrap().is_some());
        // Defaults: nothing excluded, drop and ignore when something is.
        let d = parse("version: 1\n").unwrap();
        assert!(d.path_filter().unwrap().is_none() && d.paths.gitignore);
        for bad in ["\"# comment\"", "\" x\"", "\"\"", "\".gitignore\"", "\"*\""] {
            assert!(
                parse(&format!("version: 1\npaths:\n  exclude: [{bad}]\n")).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn starter_config_is_valid_and_inert() {
        let c = parse(starter_config()).unwrap();
        assert!(c.is_inert(), "a fresh config must not change anything");
    }

    #[test]
    fn starter_config_features_are_valid_when_switched_on() {
        let on = starter_config().replace("#~ ", "");
        let c = parse(&on).unwrap();
        assert!(c.schedule.is_some() && !c.identity.is_empty() && !c.paths.exclude.is_empty());
    }
}
