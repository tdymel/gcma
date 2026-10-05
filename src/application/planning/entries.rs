//! Turning the commits to rewrite into plan entries.

use std::collections::HashMap;

use super::pathplan::PathOutcome;
use crate::domain::error::Result;
use crate::domain::history::commit::{Commit, RawIdent};
use crate::domain::history::parents::resolve_parents;
use crate::domain::history::plan::{Entry, PIdent, Plan};
use crate::domain::history::wire::Text;
use crate::domain::scheduling::Window;
use crate::domain::settings::{Config, Signing};

/// The commits of `plan` that change on their own (see `changes_on_its_own`), and the dropped ones.
pub(super) fn own_changes(
    signing: Signing,
    plan: &Plan,
    commits: &HashMap<String, Commit>,
) -> Vec<String> {
    plan.entries
        .iter()
        .filter(|e| changes_on_its_own(signing, e, &commits[&e.old_oid]))
        .map(|e| e.old_oid.clone())
        .chain(plan.dropped.iter().cloned())
        .collect()
}

/// Whether entry `e` changes its old commit `c` on its own: its identities, times, message,
/// signature or tree. One that only gets a new parent (a conforming commit on top of a rewritten
/// one) does not: it follows the rules already.
fn changes_on_its_own(signing: Signing, e: &Entry, c: &Commit) -> bool {
    let signature = match signing {
        Signing::Strip => c.has_signature(),
        Signing::Resign => !c.has_signature(),
    };
    e.author.to_raw() != c.author
        || e.committer.to_raw() != c.committer
        || e.message != c.message
        || e.tree.as_ref().is_some_and(|t| *t != c.tree)
        || signature
}

/// The old parents of every dropped commit.
pub(super) fn dropped_parents<'a>(
    outcome: &'a PathOutcome,
    commits: &'a HashMap<String, Commit>,
) -> HashMap<&'a str, &'a [String]> {
    outcome
        .dropped
        .iter()
        .map(|o| (o.as_str(), commits[o].parents.as_slice()))
        .collect()
}

/// One entry per commit of `linear` (parents first). In schedule mode `new_times[i]` is the time
/// of entry `i`; otherwise the original times and offsets are kept.
pub(super) fn build_entries(
    cfg: &Config,
    window: Option<&Window>,
    new_times: &[i64],
    outcome: &PathOutcome,
    commits: &HashMap<String, Commit>,
) -> Result<Vec<Entry>> {
    let linear = &outcome.kept;
    let index: HashMap<&str, usize> = linear
        .iter()
        .enumerate()
        .map(|(i, o)| (o.as_str(), i))
        .collect();
    let dropped = dropped_parents(outcome, commits);
    let mut entries = Vec::with_capacity(linear.len());
    for (i, oid) in linear.iter().enumerate() {
        let c = &commits[oid];
        let mapped = |id: &RawIdent| -> Result<PIdent> {
            // Identity rules match text, lossily for bytes that are not UTF-8 (the conformance check
            // does the same). A rule that matches replaces both fields; otherwise the bytes stay.
            let mapped = cfg.map_identity(
                &String::from_utf8_lossy(&id.name),
                &String::from_utf8_lossy(&id.email),
            );
            let (name, email) = match mapped {
                Some((n, e)) => (Text::from(n), Text::from(e)),
                None => (Text::from_bytes(&id.name), Text::from_bytes(&id.email)),
            };
            let (time, tz) = match window {
                Some(w) => (new_times[i], w.tz_offset_minutes(new_times[i])),
                None => (id.time, id.tz),
            };
            Ok(PIdent {
                name,
                email,
                time,
                tz,
            })
        };
        let parents = resolve_parents(&c.parents, &index, &dropped);
        let (tree, gitignore) = match outcome.trees.get(oid) {
            Some((t, g)) => (Some(t.clone()), *g),
            None => (None, false),
        };
        let e = Entry {
            old_oid: oid.clone(),
            parents,
            author: mapped(&c.author)?,
            committer: mapped(&c.committer)?,
            message: cfg.rewrite_message(&c.message),
            tree,
            gitignore,
        };
        entries.push(e);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::history::commit::Header;
    use crate::domain::history::plan::Parent;

    fn commit(oid: &str, signed: bool) -> Commit {
        let id = RawIdent {
            name: b"Jane".to_vec(),
            email: b"jane@x".to_vec(),
            time: 1_600_000_000,
            tz: 60,
        };
        Commit {
            oid: oid.into(),
            tree: "t".into(),
            parents: vec!["p".into()],
            author: id.clone(),
            committer: id,
            extra: signed
                .then(|| Header {
                    key: "gpgsig".into(),
                    raw: b"sig".to_vec(),
                })
                .into_iter()
                .collect(),
            message: b"msg\n".to_vec(),
        }
    }

    /// The entry that only moves `c` onto a new parent.
    fn moved(c: &Commit) -> Entry {
        let id = |r: &RawIdent| PIdent {
            name: Text::from_bytes(&r.name),
            email: Text::from_bytes(&r.email),
            time: r.time,
            tz: r.tz,
        };
        Entry {
            old_oid: c.oid.clone(),
            parents: vec![Parent::In(0)],
            author: id(&c.author),
            committer: id(&c.committer),
            message: c.message.clone(),
            tree: None,
            gitignore: false,
        }
    }

    #[test]
    fn only_a_new_parent_is_no_change_of_its_own() {
        let c = commit("c", false);
        assert!(!changes_on_its_own(Signing::Strip, &moved(&c), &c));
        let mut e = moved(&c);
        e.author.email = "jane@work".into();
        assert!(changes_on_its_own(Signing::Strip, &e, &c), "identity");
        let mut e = moved(&c);
        e.committer.time += 1;
        assert!(changes_on_its_own(Signing::Strip, &e, &c), "time");
        let mut e = moved(&c);
        e.message = b"msg\n\nAssisted-By: A <a@x>\n".to_vec();
        assert!(changes_on_its_own(Signing::Strip, &e, &c), "message");
        let mut e = moved(&c);
        e.tree = Some("t".into());
        assert!(!changes_on_its_own(Signing::Strip, &e, &c), "same tree");
        e.tree = Some("u".into());
        assert!(changes_on_its_own(Signing::Strip, &e, &c), "tree");
        let signed = commit("s", true);
        assert!(changes_on_its_own(Signing::Strip, &moved(&signed), &signed));
        assert!(!changes_on_its_own(
            Signing::Resign,
            &moved(&signed),
            &signed
        ));
        assert!(changes_on_its_own(Signing::Resign, &moved(&c), &c));
    }

    #[test]
    fn dropped_commits_change_on_their_own() {
        let (a, b) = (commit("a", false), commit("b", false));
        let mut e = moved(&a);
        e.message = b"other\n".to_vec();
        let mut plan = Plan::new(
            "refs/heads/main".into(),
            "b".into(),
            Signing::Strip,
            vec![e, moved(&b)],
        );
        plan.dropped = vec!["d".into()];
        let commits = HashMap::from([("a".to_string(), a), ("b".to_string(), b)]);
        assert_eq!(own_changes(Signing::Strip, &plan, &commits), ["a", "d"]);
    }
}
