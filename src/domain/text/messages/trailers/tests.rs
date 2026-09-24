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

#[test]
fn adding_to_a_block_followed_by_blank_lines_joins_the_block_and_settles() {
    let want = s(&["Assisted-By: Bot", "Reviewed-By: R"]);
    for tail in ["\n", "\n\n", "\n \n\t\n"] {
        let m = format!("s\n\nAssisted-By: Bot\nKey: v\n{tail}");
        let once = add_trailers(m.as_bytes(), &want);
        assert_eq!(
            once, b"s\n\nAssisted-By: Bot\nKey: v\nReviewed-By: R\n",
            "{tail:?}"
        );
        assert_eq!(add_trailers(&once, &want), once, "{tail:?}: second pass");
    }
}

#[test]
fn a_body_followed_by_blank_lines_gets_exactly_one_separator() {
    let want = s(&["Assisted-By: Bot"]);
    for m in ["s\n\nbody\n\n\n", "s\n\nbody\n \n", "s\n\nbody\n"] {
        let once = add_trailers(m.as_bytes(), &want);
        assert_eq!(once, b"s\n\nbody\n\nAssisted-By: Bot\n", "{m:?}");
        assert_eq!(add_trailers(&once, &want), once);
    }
}
