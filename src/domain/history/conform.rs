//! "Conforming" predicate and the frozen set.

use std::collections::{HashMap, HashSet};

use super::commit::{Commit, RawIdent};
use crate::domain::scheduling::Window;
use crate::domain::settings::{Config, Signing};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Time,
    Offset,
    TimeOrder,
    Signature,
    Identity,
    Message,
    /// The tree contains a path that the rules exclude.
    Paths,
}

pub struct Ctx<'a> {
    pub cfg: &'a Config,
    /// Present only when a schedule is configured: time rules apply only then.
    pub window: Option<&'a Window>,
    /// Committer time of every commit we have loaded (range and external parents).
    pub times: &'a HashMap<String, i64>,
    /// Commits whose tree contains an excluded path (computed by the caller, which has the trees).
    pub excluded: Option<&'a HashSet<String>>,
}

fn ident_conforms(cfg: &Config, id: &RawIdent) -> bool {
    let name = String::from_utf8_lossy(&id.name);
    let email = String::from_utf8_lossy(&id.email);
    match cfg.map_identity(&name, &email) {
        None => true,
        Some((n, e)) => n.as_bytes() == id.name.as_slice() && e.as_bytes() == id.email.as_slice(),
    }
}

/// Reasons a commit is nonconforming; empty means it conforms.
pub fn reasons(c: &Commit, ctx: &Ctx) -> Vec<Reason> {
    let mut out = Vec::new();
    if let Some(w) = ctx.window {
        for id in [&c.author, &c.committer] {
            if !w.contains(id.time) {
                out.push(Reason::Time);
            } else if id.tz != w.tz_offset_minutes(id.time) {
                out.push(Reason::Offset);
            }
        }
        let parents_ok = c
            .parents
            .iter()
            .all(|p| ctx.times.get(p).is_none_or(|pt| *pt <= c.committer.time));
        if c.committer.time < c.author.time || !parents_ok {
            out.push(Reason::TimeOrder);
        }
    }
    match ctx.cfg.signing {
        Signing::Strip if c.has_signature() => out.push(Reason::Signature),
        Signing::Resign if !c.has_signature() => out.push(Reason::Signature),
        _ => {}
    }
    if !ident_conforms(ctx.cfg, &c.author) || !ident_conforms(ctx.cfg, &c.committer) {
        out.push(Reason::Identity);
    }
    if ctx.cfg.rewrite_message(&c.message) != c.message {
        out.push(Reason::Message);
    }
    if ctx.excluded.is_some_and(|set| set.contains(&c.oid)) {
        out.push(Reason::Paths);
    }
    out.dedup();
    out
}

/// Frozen = conforming AND every in-range ancestor frozen (closed under ancestry).
/// `order` must list the range parents-first.
pub fn frozen_set(
    order: &[String],
    commits: &HashMap<String, Commit>,
    ctx: &Ctx,
) -> HashSet<String> {
    let in_range: HashSet<&String> = order.iter().collect();
    let mut frozen: HashSet<String> = HashSet::new();
    for oid in order {
        let c = &commits[oid];
        let parents_frozen = c
            .parents
            .iter()
            .filter(|p| in_range.contains(p))
            .all(|p| frozen.contains(p));
        if parents_frozen && reasons(c, ctx).is_empty() {
            frozen.insert(oid.clone());
        }
    }
    frozen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::history::commit::Header;
    use crate::domain::settings::{
        Distribution, IdentityRule, MatchSpec, MessagesCfg, ScheduleCfg, SetSpec,
    };

    fn commit(
        oid: &str,
        parents: &[&str],
        t: i64,
        tz: i32,
        name: &str,
        email: &str,
        msg: &str,
    ) -> Commit {
        let id = RawIdent {
            name: name.into(),
            email: email.into(),
            time: t,
            tz,
        };
        Commit {
            oid: oid.into(),
            tree: "t".into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            author: id.clone(),
            committer: id,
            extra: Vec::new(),
            message: msg.as_bytes().to_vec(),
        }
    }

    fn strip_cfg() -> Config {
        Config {
            messages: MessagesCfg {
                strip_trailers: vec!["Signed-off-by".into()],
                ..MessagesCfg::default()
            },
            ..Config::default()
        }
    }

    #[test]
    fn without_schedule_time_never_matters() {
        let c = Config::default();
        let times = HashMap::new();
        let ctx = Ctx {
            cfg: &c,
            window: None,
            times: &times,
            excluded: None,
        };
        assert!(reasons(&commit("a", &[], 0, 777, "N", "n@x", "m\n"), &ctx).is_empty());
    }

    #[test]
    fn identity_and_signature_and_message() {
        let c = Config {
            identity: vec![IdentityRule {
                match_: MatchSpec {
                    name: None,
                    email: Some("me@home".into()),
                },
                set: SetSpec {
                    name: "Jane".into(),
                    email: "j@work".into(),
                },
            }],
            ..strip_cfg()
        };
        let times = HashMap::new();
        let ctx = Ctx {
            cfg: &c,
            window: None,
            times: &times,
            excluded: None,
        };
        let bad = commit("a", &[], 0, 0, "Me", "me@home", "s\n\nSigned-off-by: x\n");
        let r = reasons(&bad, &ctx);
        assert!(r.contains(&Reason::Identity) && r.contains(&Reason::Message));
        let good = commit("b", &[], 0, 0, "Jane", "j@work", "s\n");
        assert!(reasons(&good, &ctx).is_empty());
        let mut signed = good.clone();
        signed.extra.push(Header {
            key: "gpgsig".into(),
            raw: b"gpgsig x".to_vec(),
        });
        assert_eq!(reasons(&signed, &ctx), vec![Reason::Signature]);
    }

    #[test]
    fn missing_added_trailer_is_nonconforming() {
        let c = Config {
            messages: MessagesCfg {
                add_trailers: vec!["Assisted-By: Bot".into()],
                ..MessagesCfg::default()
            },
            ..Config::default()
        };
        let times = HashMap::new();
        let ctx = Ctx {
            cfg: &c,
            window: None,
            times: &times,
            excluded: None,
        };
        let bare = commit("a", &[], 0, 0, "N", "n@x", "s\n");
        assert_eq!(reasons(&bare, &ctx), vec![Reason::Message]);
        let done = commit("b", &[], 0, 0, "N", "n@x", "s\n\nAssisted-By: Bot\n");
        assert!(reasons(&done, &ctx).is_empty());
    }

    #[test]
    fn excluded_paths_make_a_commit_nonconforming() {
        let c = Config::default();
        let times = HashMap::new();
        let bad: HashSet<String> = ["a".to_string()].into();
        let ctx = Ctx {
            cfg: &c,
            window: None,
            times: &times,
            excluded: Some(&bad),
        };
        assert_eq!(
            reasons(&commit("a", &[], 0, 0, "N", "n@x", "m\n"), &ctx),
            vec![Reason::Paths]
        );
        assert!(reasons(&commit("b", &[], 0, 0, "N", "n@x", "m\n"), &ctx).is_empty());
    }

    #[test]
    fn resign_requires_signature() {
        let c = Config {
            signing: Signing::Resign,
            ..Config::default()
        };
        let times = HashMap::new();
        let ctx = Ctx {
            cfg: &c,
            window: None,
            times: &times,
            excluded: None,
        };
        let mut k = commit("a", &[], 0, 0, "N", "n@x", "m\n");
        assert_eq!(reasons(&k, &ctx), vec![Reason::Signature]);
        k.extra.push(Header {
            key: "gpgsig".into(),
            raw: b"gpgsig x".to_vec(),
        });
        assert!(reasons(&k, &ctx).is_empty());
    }

    #[test]
    fn schedule_rules() {
        let c = Config {
            from: Some("2026-04-01".into()),
            to: Some("2026-05-01".into()),
            schedule: Some(ScheduleCfg {
                days: ["mon", "tue", "wed", "thu", "fri"]
                    .map(String::from)
                    .to_vec(),
                hours: "09:00-18:00".into(),
                distribution: Distribution::Uniform,
                seed: 0,
            }),
            ..Config::default()
        };
        let tz = c.tz().unwrap();
        let w = Window::build(
            tz,
            &crate::domain::settings::parse_days(&c.schedule.as_ref().unwrap().days).unwrap(),
            (540, 1080),
            c.from_utc().unwrap(),
            c.resolve_to(0).unwrap(),
        );
        let mut times = HashMap::new();
        let mon_noon = 1775476800; // 2026-04-06T12:00Z (Monday)
        times.insert("p".to_string(), mon_noon + 3600);
        let ctx = Ctx {
            cfg: &c,
            window: Some(&w),
            times: &times,
            excluded: None,
        };
        assert!(reasons(&commit("a", &[], mon_noon, 0, "N", "n@x", "m\n"), &ctx).is_empty());
        // Outside hours.
        assert!(
            reasons(
                &commit("a", &[], mon_noon - 5 * 3600, 0, "N", "n@x", "m\n"),
                &ctx
            )
            .contains(&Reason::Time)
        );
        // Wrong offset.
        assert!(
            reasons(&commit("a", &[], mon_noon, 60, "N", "n@x", "m\n"), &ctx)
                .contains(&Reason::Offset)
        );
        // Earlier than its parent.
        assert!(
            reasons(&commit("a", &["p"], mon_noon, 0, "N", "n@x", "m\n"), &ctx)
                .contains(&Reason::TimeOrder)
        );
    }

    #[test]
    fn frozen_set_is_closed_under_ancestry() {
        // a (bad) <- b (good) ; c (good, unrelated root)
        let c = strip_cfg();
        let times = HashMap::new();
        let ctx = Ctx {
            cfg: &c,
            window: None,
            times: &times,
            excluded: None,
        };
        let a = commit("a", &[], 0, 0, "N", "n@x", "s\n\nSigned-off-by: x\n");
        let b = commit("b", &["a"], 0, 0, "N", "n@x", "ok\n");
        let cc = commit("c", &[], 0, 0, "N", "n@x", "ok\n");
        let commits: HashMap<String, Commit> =
            [a, b, cc].into_iter().map(|x| (x.oid.clone(), x)).collect();
        let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let f = frozen_set(&order, &commits, &ctx);
        assert!(!f.contains("a"));
        assert!(
            !f.contains("b"),
            "conforming descendant of a nonconforming commit is not frozen"
        );
        assert!(f.contains("c"));
    }
}
