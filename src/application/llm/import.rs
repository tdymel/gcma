//! Strict, all-or-nothing validation and application of an LLM's JSONL reply.

use std::collections::HashSet;

use serde::Deserialize;

use crate::domain::error::{Error, Result};
use crate::domain::history::plan::Plan;
use crate::domain::settings::Config;
use crate::domain::text::messages;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    i: usize,
    t: String,
    b: Option<String>,
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
        if msg.iter().any(|&b| b < 0x20 && !matches!(b, b'\n' | b'\t')) {
            bad("the message contains control characters".into());
            continue;
        }
        let old = &plan.entries[row.i].message;
        if let Some(missing) = protected_trailers(old, &cfg.messages.strip_trailers)
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
        let had = messages::trailers(old);
        if let Some(forged) = protected_trailers(&msg, &[])
            .into_iter()
            .find(|t| !had.contains(t))
        {
            bad(format!(
                "the trailer {:?} is new; a reply may not add Signed-off-by or Co-authored-by lines",
                String::from_utf8_lossy(&forged)
            ));
            continue;
        }
        // The reply cannot know about trailers the rules append; they are put back here.
        let msg = cfg.rewrite_message(&msg);
        if msg == *old {
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
        plan.entries[i].message = m;
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
        assert!(matches!(e, Error::LlmInvalid(_)));
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
    fn replies_cannot_forge_sign_offs_or_smuggle_control_characters() {
        let cfg = Config::default();
        let mut p = plan_with(&["a\n"]);
        let e = import(
            &mut p,
            "{\"i\":0,\"t\":\"x\",\"b\":\"Signed-off-by: Mallory <m@x>\"}",
            &cfg,
        )
        .unwrap_err();
        assert!(e.to_string().contains("is new"), "{e}");
        assert!(import(&mut p, "{\"i\":0,\"t\":\"x\\u0000y\"}", &cfg).is_err());
        assert!(import(&mut p, "{\"i\":0,\"t\":\"x\",\"b\":\"\\u001b[2J\"}", &cfg).is_err());
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
