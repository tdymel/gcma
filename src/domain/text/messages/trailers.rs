//! Trailer edits: strip by key, rewrite lines with regexes, append wanted ones.

use regex::bytes::Regex;

use super::block::{map_final_block, trailer_key, trailers};

/// Removes trailers whose key is in `keys` (case-insensitive) from the trailing trailer block(s),
/// repeating until nothing changes (so the function is idempotent).
pub fn strip_trailers(msg: &[u8], keys: &[String]) -> Vec<u8> {
    if keys.is_empty() {
        return msg.to_vec();
    }
    let keep = |block: &[Vec<u8>]| -> Vec<Vec<u8>> {
        block
            .iter()
            .filter(|l| {
                let k = trailer_key(l).unwrap_or(b"");
                !keys.iter().any(|x| x.as_bytes().eq_ignore_ascii_case(k))
            })
            .cloned()
            .collect()
    };
    let mut cur = msg.to_vec();
    loop {
        let next = map_final_block(&cur, &keep);
        if next == cur {
            return cur;
        }
        cur = next;
    }
}

/// A sed-like rule for the lines of the trailer block: every match of `pattern` in a line is
/// replaced by `replacement` (`$1` refers to groups); a line that ends up empty is dropped.
#[derive(Debug, Clone)]
pub struct TrailerRewrite {
    pub pattern: Regex,
    pub replacement: String,
}

/// Rewrites the lines of the final trailer block with `rules`, in order. Lines that become
/// identical collapse into the first one.
pub fn rewrite_trailers(msg: &[u8], rules: &[TrailerRewrite]) -> Vec<u8> {
    if rules.is_empty() {
        return msg.to_vec();
    }
    let rewrite = |block: &[Vec<u8>]| -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = Vec::new();
        for line in block {
            let mut line = line.clone();
            for r in rules {
                line = r
                    .pattern
                    .replace_all(&line, r.replacement.as_bytes())
                    .into_owned();
            }
            if !line.is_empty() && !out.contains(&line) {
                out.push(line);
            }
        }
        out
    };
    map_final_block(msg, &rewrite)
}

/// Appends each `Key: value` line of `add` that is not already present as a trailer (compared
/// byte-exact on the value, case-insensitive on the key). New lines join the final trailer block,
/// or start one after a blank line. Blank or empty messages are left alone.
pub fn add_trailers(msg: &[u8], add: &[String]) -> Vec<u8> {
    if add.is_empty() || msg.iter().all(u8::is_ascii_whitespace) {
        return msg.to_vec();
    }
    let present = trailers(msg);
    let missing: Vec<&String> = add
        .iter()
        .filter(|line| {
            let want = line.as_bytes();
            let key = trailer_key(want).unwrap_or(b"");
            let value = &want[(key.len() + 1).min(want.len())..];
            !present.iter().any(|p| {
                trailer_key(p).is_some_and(|k| k.eq_ignore_ascii_case(key))
                    && p[k_len(p) + 1..] == *value
            })
        })
        .collect();
    if missing.is_empty() {
        return msg.to_vec();
    }
    let mut out = msg.to_vec();
    if out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    if present.is_empty() {
        // A message that ends in a blank line keeps just one separator.
        while out.len() >= 2 && out[out.len() - 2] == b'\n' {
            out.pop();
        }
        out.push(b'\n');
    }
    for line in missing {
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    out
}

fn k_len(trailer: &[u8]) -> usize {
    trailer_key(trailer).map_or(0, <[u8]>::len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    fn rule(pattern: &str, replacement: &str) -> TrailerRewrite {
        TrailerRewrite {
            pattern: Regex::new(pattern).unwrap(),
            replacement: replacement.to_string(),
        }
    }

    #[test]
    fn strips_listed_trailers() {
        let m = b"Subject\n\nBody\n\nSigned-off-by: a <a@b>\nCo-authored-by: c <c@d>\n";
        let out = strip_trailers(m, &s(&["signed-off-by"]));
        assert_eq!(out, b"Subject\n\nBody\n\nCo-authored-by: c <c@d>\n");
    }

    #[test]
    fn removes_empty_block_and_separator() {
        let m = b"Subject\n\nBody\n\nSigned-off-by: a\n";
        assert_eq!(
            strip_trailers(m, &s(&["Signed-off-by"])),
            b"Subject\n\nBody\n"
        );
    }

    #[test]
    fn repeats_until_fixed_point() {
        // After removing the last block, the previous paragraph is also a trailer block.
        let m = b"Subject\n\nSigned-off-by: a\n\nCo-authored-by: b\n";
        let once = strip_trailers(m, &s(&["Signed-off-by", "Co-authored-by"]));
        assert_eq!(once, b"Subject\n");
        assert_eq!(
            strip_trailers(&once, &s(&["Signed-off-by", "Co-authored-by"])),
            once
        );
    }

    #[test]
    fn idempotent_on_awkward_inputs() {
        let keys = s(&["Signed-off-by"]);
        for m in [
            &b"Signed-off-by: x"[..],
            b"Signed-off-by: x\n",
            b"Subject\n\nSigned-off-by: a\n\n\n",
            b"Subject\n\nSigned-off-by: a\nnot a trailer\n",
            b"",
            b"\n",
            b"S\n\nSigned-off-by: a\n\nBody after\n",
        ] {
            let once = strip_trailers(m, &keys);
            assert_eq!(
                strip_trailers(&once, &keys),
                once,
                "input {:?}",
                String::from_utf8_lossy(m)
            );
        }
    }

    #[test]
    fn no_keys_is_identity() {
        let m = b"S\r\n\nSigned-off-by: a\xff\n";
        assert_eq!(strip_trailers(m, &[]), m);
    }

    #[test]
    fn adds_a_trailer_block_after_the_body() {
        let add = s(&["Assisted-By: Bot <b@x>"]);
        assert_eq!(
            add_trailers(b"Subject\n", &add),
            b"Subject\n\nAssisted-By: Bot <b@x>\n"
        );
        assert_eq!(
            add_trailers(b"Subject\n\nBody\n\n\n", &add),
            b"Subject\n\nBody\n\nAssisted-By: Bot <b@x>\n"
        );
        assert_eq!(
            add_trailers(b"Subject", &add),
            b"Subject\n\nAssisted-By: Bot <b@x>\n"
        );
    }

    #[test]
    fn joins_an_existing_trailer_block_and_is_idempotent() {
        let add = s(&["Assisted-By: Bot <b@x>"]);
        let once = add_trailers(b"S\n\nBody\n\nSigned-off-by: a\n", &add);
        assert_eq!(
            once,
            b"S\n\nBody\n\nSigned-off-by: a\nAssisted-By: Bot <b@x>\n"
        );
        assert_eq!(add_trailers(&once, &add), once);
        // Same key, different value: both are kept.
        let other = add_trailers(&once, &s(&["assisted-by: Other"]));
        assert!(other.ends_with(b"assisted-by: Other\n"));
    }

    #[test]
    fn empty_messages_are_left_alone() {
        assert_eq!(add_trailers(b"", &s(&["A: b"])), b"");
        assert_eq!(add_trailers(b"\n", &s(&["A: b"])), b"\n");
    }

    #[test]
    fn a_rewrite_turns_a_trailer_into_another_and_leaves_the_rest() {
        let rules = [rule(
            r"^Co-Authored-By: Claude (Opus|Sonnet)\b.*<noreply@anthropic\.com>$",
            "Assisted-By: Claude $1",
        )];
        let m = b"S\n\nSigned-off-by: A <a@x>\nCo-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>\n";
        let out = rewrite_trailers(m, &rules);
        assert_eq!(
            out,
            b"S\n\nSigned-off-by: A <a@x>\nAssisted-By: Claude Opus\n"
        );
        assert_eq!(rewrite_trailers(&out, &rules), out, "idempotent");
    }

    #[test]
    fn a_rewrite_can_cut_the_version_out_of_the_middle_of_a_line() {
        let rules = [rule(
            r"(?i)^(Assisted-by:.*?\b(?:opus|sonnet|haiku|fable))[ -]\d+(?:[.-]\d+)*",
            "$1",
        )];
        for (from, to) in [
            (
                "Assisted-by: Claude Code:claude-opus-5-5",
                "Assisted-by: Claude Code:claude-opus",
            ),
            (
                "Assisted-by: Claude Code:Opus 5",
                "Assisted-by: Claude Code:Opus",
            ),
            (
                "Assisted-by: Claude Sonnet 5 [ted-gate]",
                "Assisted-by: Claude Sonnet [ted-gate]",
            ),
            (
                "Assisted-by: Claude Code:claude-fable-5-1 [a] [b]",
                "Assisted-by: Claude Code:claude-fable [a] [b]",
            ),
            (
                "Assisted-by: Claude Code:claude-opus",
                "Assisted-by: Claude Code:claude-opus",
            ),
            ("Reviewed-by: Opus 5 <o@x>", "Reviewed-by: Opus 5 <o@x>"),
        ] {
            let m = format!("S\n\n{from}\n");
            assert_eq!(
                rewrite_trailers(m.as_bytes(), &rules),
                format!("S\n\n{to}\n").into_bytes(),
                "{from}"
            );
        }
    }

    #[test]
    fn rewrites_run_in_order_and_an_empty_result_drops_the_line() {
        let rules = [
            rule("^A: x$", "B: y"),
            rule("^B: y$", ""),
            rule("^Keep: .*", "$0"),
        ];
        let m = b"S\n\nA: x\nKeep: me\n";
        assert_eq!(rewrite_trailers(m, &rules), b"S\n\nKeep: me\n");
        let only = b"S\n\nA: x\n";
        assert_eq!(
            rewrite_trailers(only, &rules),
            b"S\n",
            "an emptied block vanishes with its separator"
        );
    }

    #[test]
    fn lines_that_become_identical_collapse() {
        let rules = [rule("^Co-Authored-By: (.*)$", "Assisted-By: Claude")];
        let m = b"S\n\nCo-Authored-By: a\nCo-Authored-By: b\n";
        assert_eq!(rewrite_trailers(m, &rules), b"S\n\nAssisted-By: Claude\n");
    }

    #[test]
    fn rewrites_only_touch_a_real_trailer_block() {
        let rules = [rule("^Co-Authored-By: .*$", "")];
        for m in [
            &b"Co-Authored-By: subject\n"[..],
            b"S\n\nCo-Authored-By: x\nnot a trailer\n",
            b"S\n\nCo-Authored-By: x\n\nafter\n",
        ] {
            assert_eq!(rewrite_trailers(m, &rules), m);
        }
    }

    #[test]
    fn rewrites_work_on_bytes_that_are_not_utf8_with_a_byte_pattern() {
        let m = b"S\n\nSigned-off-by: J\xf6rg <j@x>\n";
        let bytes = [rule(r"(?-u)^Signed-off-by: (.*)$", "Reviewed-by: $1")];
        assert_eq!(
            rewrite_trailers(m, &bytes),
            b"S\n\nReviewed-by: J\xf6rg <j@x>\n"
        );
        // A Unicode pattern simply does not match such a line, so it is left alone.
        let unicode = [rule(r"^Signed-off-by: (.*)$", "Reviewed-by: $1")];
        assert_eq!(rewrite_trailers(m, &unicode), m);
    }

    #[test]
    fn subject_is_never_a_trailer_block() {
        assert_eq!(
            strip_trailers(b"Signed-off-by: x\n", &s(&["Signed-off-by"])),
            b"Signed-off-by: x\n"
        );
    }

    #[test]
    fn preserves_missing_trailing_newline() {
        let m = b"S\n\nBody\n\nSigned-off-by: a";
        assert_eq!(strip_trailers(m, &s(&["Signed-off-by"])), b"S\n\nBody");
    }
}
