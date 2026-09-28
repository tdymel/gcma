//! Strict, all-or-nothing validation and application of an LLM's JSONL reply.

use std::collections::HashSet;

use crate::domain::error::{Error, Result};
use crate::domain::history::plan::Plan;
use crate::domain::settings::Config;
use crate::domain::text::messages;

/// One parsed row of an LLM's reply: a new message for the entry at `index`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub index: usize,
    pub title: String,
    pub body: Option<String>,
}

/// A reply row, or why its line could not be read (already worded, with the line number).
pub type ReplyLine = std::result::Result<Reply, String>;

#[derive(Debug)]
pub struct ImportReport {
    pub changed: usize,
    pub unchanged_rows: usize,
}

fn protected_trailers(old: &[u8], strip: &[String]) -> Vec<Vec<u8>> {
    messages::trailers(old)
        .into_iter()
        .filter(|l| {
            let k = messages::trailer_key(l).unwrap_or(b"");
            let is_protected = k.eq_ignore_ascii_case(b"signed-off-by")
                || k.eq_ignore_ascii_case(b"co-authored-by");
            let stripped = strip.iter().any(|s| s.as_bytes().eq_ignore_ascii_case(k));
            is_protected && !stripped
        })
        .collect()
}

/// Validates the whole reply first; applies it only if every row is valid (all or nothing).
pub fn import(plan: &mut Plan, reply: Vec<ReplyLine>, cfg: &Config) -> Result<ImportReport> {
    let mut errors: Vec<String> = Vec::new();
    let mut retry: Vec<usize> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut updates: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut unchanged = 0;

    for line in reply {
        let row = match line {
            Ok(r) => r,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let Some(entry) = plan.entries.get(row.index) else {
            // No such entry, so there is no row to retry either.
            errors.push(format!(
                "row i={}: unknown index (valid: 0..{})",
                row.index,
                plan.entries.len()
            ));
            continue;
        };
        let checked = if seen.insert(row.index) {
            validate_row(&row, &entry.message, cfg)
        } else {
            Err("duplicate index".to_string())
        };
        match checked {
            Ok(msg) if msg == plan.entries[row.index].message => unchanged += 1,
            Ok(msg) => updates.push((row.index, msg)),
            Err(why) => {
                errors.push(format!("row i={}: {why}", row.index));
                retry.push(row.index);
            }
        }
    }
    if !errors.is_empty() {
        retry.sort_unstable();
        retry.dedup();
        let mut text = format!("{}\nnothing was imported", errors.join("\n"));
        if !retry.is_empty() {
            text.push_str(&format!("; retry rows: {retry:?}"));
        }
        return Err(Error::LlmInvalid(text));
    }
    let changed = updates.len();
    for (i, m) in updates {
        plan.entries[i].message = m;
    }
    Ok(ImportReport {
        changed,
        unchanged_rows: unchanged,
    })
}

/// The full message a reply row stands for (rules that append trailers applied), or why the row
/// is refused. `old` is the message the entry has now.
fn validate_row(row: &Reply, old: &[u8], cfg: &Config) -> std::result::Result<Vec<u8>, String> {
    let msg = compose_message(row)?;
    check_trailers(old, &msg, cfg)?;
    // The reply cannot know about trailers the rules append; they are put back here.
    Ok(cfg.rewrite_message(&msg))
}

/// The title, a blank line and the body of the row, as commit message bytes.
fn compose_message(row: &Reply) -> std::result::Result<Vec<u8>, String> {
    let title = row.title.trim();
    if title.is_empty() {
        return Err("empty title".into());
    }
    if title.contains('\n') || title.contains('\r') {
        return Err("the title must be a single line".into());
    }
    if title.chars().count() > 200 {
        return Err("title longer than 200 characters".into());
    }
    let mut msg = title.as_bytes().to_vec();
    if let Some(b) = row.body.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        msg.extend_from_slice(b"\n\n");
        msg.extend_from_slice(b.as_bytes());
    }
    msg.push(b'\n');
    if msg.iter().any(|&b| b < 0x20 && !matches!(b, b'\n' | b'\t')) {
        return Err("the message contains control characters".into());
    }
    Ok(msg)
}

/// A reply keeps the protected trailers of `old`, may not add any, and carries none that
/// `messages.strip_trailers` removes.
fn check_trailers(old: &[u8], msg: &[u8], cfg: &Config) -> std::result::Result<(), String> {
    if let Some(missing) = protected_trailers(old, &cfg.messages.strip_trailers)
        .into_iter()
        .find(|t| !msg.split(|&c| c == b'\n').any(|l| l == t.as_slice()))
    {
        return Err(format!(
            "the trailer {:?} was dropped",
            String::from_utf8_lossy(&missing)
        ));
    }
    if messages::strip_trailers(msg, &cfg.messages.strip_trailers) != msg {
        return Err("the message contains a trailer that `messages.strip_trailers` removes".into());
    }
    let had = messages::trailers(old);
    if let Some(forged) = protected_trailers(msg, &[])
        .into_iter()
        .find(|t| !had.contains(t))
    {
        return Err(format!(
            "the trailer {:?} is new; a reply may not add Signed-off-by or Co-authored-by lines",
            String::from_utf8_lossy(&forged)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::history::plan::{Entry, PIdent, PLAN_VERSION, Parent};
    use crate::domain::settings::Signing;

    fn plan_with(msgs: &[&str]) -> Plan {
        let id = PIdent {
            name: "N".into(),
            email: "e@x".into(),
            time: 0,
            tz: 0,
        };
        let entries = msgs
            .iter()
            .enumerate()
            .map(|(i, m)| Entry {
                old_oid: format!("o{i}"),
                parents: if i == 0 {
                    vec![]
                } else {
                    vec![Parent::In(i - 1)]
                },
                author: id.clone(),
                committer: id.clone(),
                message: m.as_bytes().to_vec(),
                tree: None,
                gitignore: false,
            })
            .collect();
        Plan {
            version: PLAN_VERSION,
            branch_ref: "refs/heads/main".into(),
            tip_oid: "o".into(),
            signing: Signing::Strip,
            entries,
            paths: None,
            dropped: Vec::new(),
            new_tip: None,
        }
    }

    fn strip_cfg() -> Config {
        let mut c = Config::default();
        c.messages.strip_trailers = vec!["Signed-off-by".into()];
        c
    }

    fn msg(p: &Plan, i: usize) -> String {
        String::from_utf8(p.entries[i].message.clone()).unwrap()
    }

    fn row(index: usize, title: &str, body: Option<&str>) -> ReplyLine {
        Ok(Reply {
            index,
            title: title.into(),
            body: body.map(String::from),
        })
    }

    #[test]
    fn imports_valid_rows_and_may_omit_some() {
        let mut p = plan_with(&["wip\n", "fix\n"]);
        let r = import(
            &mut p,
            vec![row(0, "Add parser", Some("Because."))],
            &Config::default(),
        )
        .unwrap();
        assert_eq!(r.changed, 1);
        assert_eq!(msg(&p, 0), "Add parser\n\nBecause.\n");
        assert_eq!(msg(&p, 1), "fix\n");
    }

    #[test]
    fn all_or_nothing_with_retry_list() {
        let mut p = plan_with(&["a\n", "b\n", "c\n"]);
        let reply = vec![
            row(0, "ok", None),
            row(1, "two\nlines", None),
            row(9, "x", None),
        ];
        let e = import(&mut p, reply, &Config::default()).unwrap_err();
        assert!(matches!(e, Error::LlmInvalid(_)));
        assert!(e.to_string().contains("retry rows: [1]"), "{e}");
        assert_eq!(msg(&p, 0), "a\n", "nothing applied");
    }

    #[test]
    fn unknown_indices_are_reported_but_not_offered_for_retry() {
        let mut p = plan_with(&["a\n"]);
        let e = import(
            &mut p,
            vec![row(0, "ok", None), row(9, "x", None)],
            &Config::default(),
        )
        .unwrap_err();
        let text = e.to_string();
        assert!(
            text.contains("row i=9: unknown index (valid: 0..1)"),
            "{text}"
        );
        assert!(!text.contains("retry rows"), "{text}");
    }

    #[test]
    fn unreadable_lines_dupes_and_empty_titles_are_rejected() {
        let cfg = Config::default();
        let mut p = plan_with(&["a\n"]);
        let unreadable = Err("line 1: not a valid reply row".to_string());
        assert!(import(&mut p, vec![unreadable, row(0, "x", None)], &cfg).is_err());
        assert!(import(&mut p, vec![row(0, "x", None), row(0, "y", None)], &cfg).is_err());
        assert!(import(&mut p, vec![row(0, "  ", None)], &cfg).is_err());
    }

    #[test]
    fn protected_trailers_must_survive() {
        let mut p = plan_with(&["wip\n\nSigned-off-by: A <a@x>\n"]);
        let cfg = Config::default();
        let e = import(&mut p, vec![row(0, "Better", None)], &cfg).unwrap_err();
        assert!(e.to_string().contains("trailer"), "{e}");
        // Including the trailer in the body keeps it.
        assert!(
            import(
                &mut p,
                vec![row(0, "Better", Some("Signed-off-by: A <a@x>"))],
                &cfg
            )
            .is_ok()
        );
        // A strip rule makes dropping it legitimate.
        let mut q = plan_with(&["wip\n\nSigned-off-by: A <a@x>\n"]);
        assert!(import(&mut q, vec![row(0, "Better", None)], &strip_cfg()).is_ok());
    }

    #[test]
    fn replies_cannot_forge_sign_offs_or_smuggle_control_characters() {
        let cfg = Config::default();
        let mut p = plan_with(&["a\n"]);
        let e = import(
            &mut p,
            vec![row(0, "x", Some("Signed-off-by: Mallory <m@x>"))],
            &cfg,
        )
        .unwrap_err();
        assert!(e.to_string().contains("is new"), "{e}");
        assert!(import(&mut p, vec![row(0, "x\0y", None)], &cfg).is_err());
        assert!(import(&mut p, vec![row(0, "x", Some("\u{1b}[2J"))], &cfg).is_err());
    }

    #[test]
    fn reply_must_keep_the_message_conforming() {
        let mut p = plan_with(&["a\n"]);
        let e = import(
            &mut p,
            vec![row(0, "x", Some("Signed-off-by: me"))],
            &strip_cfg(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("strip_trailers"), "{e}");
    }
}
