//! Compact export of a plan for LLMs, and strict validation of their replies.

use std::collections::{BTreeMap, HashSet};

use chrono::TimeZone;
use serde::{Deserialize, Serialize};

use crate::domain::error::{Error, Result};
use crate::domain::history::plan::Plan;
use crate::domain::settings::Config;
use crate::domain::text::messages;
use crate::git::Git;

#[derive(Debug, Serialize)]
struct Row<'a> {
    i: usize,
    /// Date (YYYY-MM-DD).
    d: String,
    /// Short stat: `+adds-dels Nf`.
    s: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    a: Option<String>,
    m: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    i: usize,
    t: String,
    b: Option<String>,
}

pub const PROMPT: &str = "\
You will receive JSONL, one commit per line: {\"i\": index, \"d\": date, \"s\": \"+adds-dels Nf\", \"m\": current message}.
(\"a\" appears only when the author differs from the common author named below.)
Write a better commit message for each commit you can improve: a concise imperative title (<= 72 chars, one line)
and an optional body explaining why. Judge from the current message and the stat; do not invent facts.
Reply with JSONL ONLY (no prose, no code fences), one object per commit you change:
{\"i\": <same index>, \"t\": \"<title>\", \"b\": \"<body, optional>\"}
Omit commits whose message is already good. Keep any Signed-off-by / Co-authored-by lines out of the body;
they are preserved automatically.";

/// Rows `[offset, offset+batch)` of the plan, as JSONL lines, plus a short prelude for stderr.
pub fn export(
    git: &Git,
    plan: &Plan,
    batch: Option<usize>,
    offset: usize,
) -> Result<(String, Vec<String>)> {
    let total = plan.entries.len();
    let offset = offset.min(total);
    let end = batch.map_or(total, |b| offset.saturating_add(b).min(total));
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &plan.entries {
        *counts
            .entry(format!("{} <{}>", e.author.name, e.author.email))
            .or_default() += 1;
    }
    let common = counts
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(k, _)| k.clone())
        .unwrap_or_default();
    let mut rows = Vec::new();
    let oids: Vec<String> = plan.entries[offset..end]
        .iter()
        .map(|e| e.old_oid.clone())
        .collect();
    let stats = git.numstats(&oids)?;
    for ((i, e), (a, d, f)) in plan
        .entries
        .iter()
        .enumerate()
        .take(end)
        .skip(offset)
        .zip(stats)
    {
        let msg = e.message()?;
        let text = String::from_utf8_lossy(&msg);
        let who = format!("{} <{}>", e.author.name, e.author.email);
        let date = chrono::FixedOffset::east_opt(e.committer.tz * 60)
            .and_then(|o| o.timestamp_opt(e.committer.time, 0).single())
            .map(|t| t.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let row = Row {
            i,
            d: date,
            s: format!("+{a}-{d} {f}f"),
            a: (who != common).then_some(who),
            m: text.trim_end_matches('\n'),
        };
        rows.push(serde_json::to_string(&row)?);
    }
    let prelude = format!(
        "{PROMPT}\n\nCommon author: {common}\nRows {offset}..{end} of {}.\n",
        plan.entries.len()
    );
    Ok((prelude, rows))
}

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
pub fn import(plan: &mut Plan, reply: &str, cfg: &Config) -> Result<ImportReport> {
    let mut errors: Vec<String> = Vec::new();
    let mut retry: Vec<usize> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut updates: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut unchanged = 0;

    for (ln, line) in reply.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: Reply = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!(
                    "line {}: not a valid reply row ({e}); prose or code fences are not allowed",
                    ln + 1
                ));
                continue;
            }
        };
        let mut bad = |m: String| {
            errors.push(format!("row i={}: {m}", row.i));
            retry.push(row.i);
        };
        if row.i >= plan.entries.len() {
            bad(format!("unknown index (valid: 0..{})", plan.entries.len()));
            continue;
        }
        if !seen.insert(row.i) {
            bad("duplicate index".into());
            continue;
        }
        let title = row.t.trim();
        if title.is_empty() {
            bad("empty title".into());
            continue;
        }
        if title.contains('\n') || title.contains('\r') {
            bad("the title must be a single line".into());
            continue;
        }
        if title.chars().count() > 200 {
            bad("title longer than 200 characters".into());
            continue;
        }
        let mut msg = title.as_bytes().to_vec();
        if let Some(b) = row.b.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            msg.extend_from_slice(b"\n\n");
            msg.extend_from_slice(b.as_bytes());
        }
        msg.push(b'\n');
        let old = plan.entries[row.i].message()?;
        if let Some(missing) = protected_trailers(&old, &cfg.messages.strip_trailers)
            .into_iter()
            .find(|t| !msg.split(|&c| c == b'\n').any(|l| l == t.as_slice()))
        {
            bad(format!(
                "the trailer {:?} was dropped",
                String::from_utf8_lossy(&missing)
            ));
            continue;
        }
        if messages::strip_trailers(&msg, &cfg.messages.strip_trailers) != msg {
            bad("the message contains a trailer that `messages.strip_trailers` removes".into());
            continue;
        }
        if msg == old {
            unchanged += 1;
        } else {
            updates.push((row.i, msg));
        }
    }
    if !errors.is_empty() {
        retry.sort_unstable();
        retry.dedup();
        return Err(Error::LlmInvalid(format!(
            "{}\nnothing was imported; retry rows: {:?}",
            errors.join("\n"),
            retry
        )));
    }
    let changed = updates.len();
    for (i, m) in updates {
        plan.entries[i].set_message(&m);
    }
    Ok(ImportReport {
        changed,
        unchanged_rows: unchanged,
    })
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
            .map(|(i, m)| {
                let mut e = Entry {
                    old_oid: format!("o{i}"),
                    parents: if i == 0 {
                        vec![]
                    } else {
                        vec![Parent::In(i - 1)]
                    },
                    author: id.clone(),
                    committer: id.clone(),
                    message_b64: String::new(),
                };
                e.set_message(m.as_bytes());
                e
            })
            .collect();
        Plan {
            version: PLAN_VERSION,
            branch_ref: "refs/heads/main".into(),
            tip_oid: "o".into(),
            signing: Signing::Strip,
            entries,
        }
    }

    fn strip_cfg() -> Config {
        let mut c = Config::default();
        c.messages.strip_trailers = vec!["Signed-off-by".into()];
        c
    }

    fn msg(p: &Plan, i: usize) -> String {
        String::from_utf8(p.entries[i].message().unwrap()).unwrap()
    }

    #[test]
    fn imports_valid_rows_and_may_omit_some() {
        let mut p = plan_with(&["wip\n", "fix\n"]);
        let r = import(
            &mut p,
            "{\"i\":0,\"t\":\"Add parser\",\"b\":\"Because.\"}\n",
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
        let reply =
            "{\"i\":0,\"t\":\"ok\"}\n{\"i\":1,\"t\":\"two\\nlines\"}\n{\"i\":9,\"t\":\"x\"}\n";
        let e = import(&mut p, reply, &Config::default()).unwrap_err();
        assert_eq!(e.exit_code(), 7);
        assert!(e.to_string().contains("retry rows: [1, 9]"), "{e}");
        assert_eq!(msg(&p, 0), "a\n", "nothing applied");
    }

    #[test]
    fn rejects_prose_dupes_empty_titles_and_unknown_keys() {
        let cfg = Config::default();
        let mut p = plan_with(&["a\n"]);
        assert!(import(&mut p, "Sure! Here you go:\n{\"i\":0,\"t\":\"x\"}", &cfg).is_err());
        assert!(import(&mut p, "{\"i\":0,\"t\":\"x\"}\n{\"i\":0,\"t\":\"y\"}", &cfg).is_err());
        assert!(import(&mut p, "{\"i\":0,\"t\":\"  \"}", &cfg).is_err());
        assert!(import(&mut p, "{\"i\":0,\"t\":\"x\",\"z\":1}", &cfg).is_err());
        assert!(import(&mut p, "```json\n{\"i\":0,\"t\":\"x\"}\n```", &cfg).is_err());
    }

    #[test]
    fn protected_trailers_must_survive() {
        let mut p = plan_with(&["wip\n\nSigned-off-by: A <a@x>\n"]);
        let cfg = Config::default();
        let e = import(&mut p, "{\"i\":0,\"t\":\"Better\"}", &cfg).unwrap_err();
        assert!(e.to_string().contains("trailer"), "{e}");
        // Including the trailer in the body keeps it.
        assert!(
            import(
                &mut p,
                "{\"i\":0,\"t\":\"Better\",\"b\":\"Signed-off-by: A <a@x>\"}",
                &cfg
            )
            .is_ok()
        );
        // A strip rule makes dropping it legitimate.
        let mut q = plan_with(&["wip\n\nSigned-off-by: A <a@x>\n"]);
        let cfg2 = strip_cfg();
        assert!(import(&mut q, "{\"i\":0,\"t\":\"Better\"}", &cfg2).is_ok());
    }

    #[test]
    fn reply_must_keep_the_message_conforming() {
        let cfg = strip_cfg();
        let mut p = plan_with(&["a\n"]);
        let e = import(
            &mut p,
            "{\"i\":0,\"t\":\"x\",\"b\":\"Signed-off-by: me\"}",
            &cfg,
        )
        .unwrap_err();
        assert!(e.to_string().contains("strip_trailers"), "{e}");
    }
}
