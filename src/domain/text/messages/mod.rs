//! Message transforms. All transforms are pure and idempotent on the message bytes.

mod block;
mod body;
mod trailers;

pub use block::{trailer_key, trailers};
pub use body::{title, title_only};
pub use trailers::{TrailerRewrite, add_trailers, rewrite_trailers, strip_trailers};

/// Everything the message rules can do, in the order they run.
#[derive(Debug, Clone, Copy)]
pub struct Rules<'a> {
    pub rewrite: &'a [TrailerRewrite],
    pub strip: &'a [String],
    pub add: &'a [String],
    pub title_only: bool,
}

/// The message after the rules: trailers rewritten and stripped (until nothing changes, so a
/// block exposed by stripping is handled too), the body dropped when asked, wanted trailers
/// appended. Idempotent as long as no key is both stripped and added (config validation
/// guarantees it) and the rewrite rules do not undo each other.
pub fn apply_trailer_rules(msg: &[u8], rules: &Rules) -> Vec<u8> {
    let mut cur = msg.to_vec();
    for _ in 0..16 {
        let next = strip_trailers(&rewrite_trailers(&cur, rules.rewrite), rules.strip);
        if next == cur {
            break;
        }
        cur = next;
    }
    if rules.title_only {
        cur = title_only(&cur);
    }
    add_trailers(&cur, rules.add)
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::bytes::Regex;

    fn s(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    fn rule(pattern: &str, replacement: &str) -> TrailerRewrite {
        TrailerRewrite {
            pattern: Regex::new(pattern).unwrap(),
            replacement: replacement.to_string(),
        }
    }

    fn run(
        m: &[u8],
        rewrite: &[TrailerRewrite],
        strip: &[&str],
        add: &[&str],
        title_only: bool,
    ) -> Vec<u8> {
        apply_trailer_rules(
            m,
            &Rules {
                rewrite,
                strip: &s(strip),
                add: &s(add),
                title_only,
            },
        )
    }

    #[test]
    fn strip_then_add_round_trips() {
        let m = b"S\n\nCo-Authored-By: Claude <c@x>\n";
        let out = run(m, &[], &["Co-Authored-By"], &["Assisted-By: Claude"], false);
        assert_eq!(out, b"S\n\nAssisted-By: Claude\n");
        assert_eq!(
            run(
                &out,
                &[],
                &["Co-Authored-By"],
                &["Assisted-By: Claude"],
                false
            ),
            out
        );
    }

    #[test]
    fn all_rules_together_are_idempotent() {
        let rules = [rule(
            r"^Co-Authored-By: Claude (\w+) [\d.]+ <.*>$",
            "Assisted-By: Claude $1",
        )];
        let m = b"feat: x\n\nA long body.\n\nMore body.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nClaude-Session: https://x/y\n";
        let out = run(m, &rules, &["Co-Authored-By"], &[], true);
        assert_eq!(
            out,
            b"feat: x\n\nAssisted-By: Claude Opus\nClaude-Session: https://x/y\n"
        );
        assert_eq!(run(&out, &rules, &["Co-Authored-By"], &[], true), out);
    }

    #[test]
    fn stripping_can_expose_an_earlier_block_that_the_rewrites_then_handle() {
        let rules = [rule("^A: x$", "B: y")];
        let m = b"S\n\nA: x\n\nDrop: me\n";
        let out = run(m, &rules, &["Drop"], &[], false);
        assert_eq!(out, b"S\n\nB: y\n");
        assert_eq!(run(&out, &rules, &["Drop"], &[], false), out);
    }
}
