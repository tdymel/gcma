//! `rewrite_trailers` and `title_only` end to end: a trailer rewritten by a regex (a co-author
//! becomes an `Assisted-By` without the model version), a message cut down to its title and
//! trailers, rules that are not valid, and that a second run changes nothing.

mod common;

use common::*;

fn run_with(messages_cfg: &str, message: &str) -> String {
    let r = Repo::new();
    r.commit_msg("a.txt", message.as_bytes(), 1_600_000_000);
    r.config(&format!("version: 1\nmessages:\n{messages_cfg}"));
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let first = r.message_bytes("HEAD");
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do"),
        "a second run must be a no-op for {message:?}"
    );
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
    assert_eq!(r.message_bytes("HEAD"), first);
    r.fsck();
    String::from_utf8(first).unwrap()
}

const CO_TO_ASSISTED: &str = "  rewrite_trailers:\n    - match: '^Co-Authored-By: Claude (Opus|Sonnet|Haiku|Fable)\\b.*<noreply@anthropic\\.com>$'\n      replace: 'Assisted-By: Claude $1 <noreply@anthropic.com>'\n";
const CUT_VERSION: &str = "    - match: '(?i)^(Assisted-by:.*?\\b(?:opus|sonnet|haiku|fable))[ -]\\d+(?:[.-]\\d+)*'\n      replace: '$1'\n";

#[test]
fn a_co_author_trailer_becomes_an_assisted_by_trailer_without_the_model_version() {
    for (from, to) in [
        (
            "Claude Opus 5.5 <noreply@anthropic.com>",
            "Claude Opus <noreply@anthropic.com>",
        ),
        (
            "Claude Sonnet 5 <noreply@anthropic.com>",
            "Claude Sonnet <noreply@anthropic.com>",
        ),
        (
            "Claude Opus 5.5 (1M context) <noreply@anthropic.com>",
            "Claude Opus <noreply@anthropic.com>",
        ),
    ] {
        let out = run_with(
            &format!("{CO_TO_ASSISTED}  strip_trailers: [Co-Authored-By]\n"),
            &format!("feat: x\n\nCo-Authored-By: {from}\n"),
        );
        assert_eq!(out, format!("feat: x\n\nAssisted-By: {to}\n"));
    }
}

#[test]
fn existing_assisted_by_trailers_lose_their_version_and_keep_their_tools() {
    for (from, to) in [
        (
            "Assisted-by: Claude Code:claude-opus-5-5",
            "Assisted-by: Claude Code:claude-opus",
        ),
        (
            "Assisted-by: Claude:claude-opus-5",
            "Assisted-by: Claude:claude-opus",
        ),
        (
            "Assisted-by: Claude Code:claude-fable-5-1 [puppeteer-core] [chromium]",
            "Assisted-by: Claude Code:claude-fable [puppeteer-core] [chromium]",
        ),
        (
            "Assisted-by: Claude Sonnet 5 [ted-gate]",
            "Assisted-by: Claude Sonnet [ted-gate]",
        ),
        (
            "Assisted-by: Claude Code:claude-opus",
            "Assisted-by: Claude Code:claude-opus",
        ),
    ] {
        let out = run_with(
            &format!("{CO_TO_ASSISTED}{CUT_VERSION}"),
            &format!("fix: y\n\n{from}\n"),
        );
        assert_eq!(out, format!("fix: y\n\n{to}\n"), "{from}");
    }
}

#[test]
fn title_only_drops_the_body_but_keeps_the_subject_and_trailers() {
    let out = run_with(
        "  title_only: true\n",
        "docs: record it\n\nA long explanation.\n\nSecond paragraph.\n\nClaude-Session: https://example.org/s/1\n",
    );
    assert_eq!(
        out,
        "docs: record it\n\nClaude-Session: https://example.org/s/1\n"
    );
    assert_eq!(
        run_with("  title_only: true\n", "docs: record it\n\nBody only.\n"),
        "docs: record it\n"
    );
    assert_eq!(
        run_with("  title_only: true\n", "docs: just a title\n"),
        "docs: just a title\n"
    );
}

#[test]
fn the_libero_style_pipeline_on_each_kind_of_commit() {
    let cfg = format!(
        "  title_only: true\n{CO_TO_ASSISTED}{CUT_VERSION}  strip_trailers: [Co-Authored-By]\n"
    );
    let with_co_author = "docs(brain): note\n\nWhy it matters.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_1\n";
    assert_eq!(
        run_with(&cfg, with_co_author),
        "docs(brain): note\n\nAssisted-By: Claude Sonnet <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_1\n"
    );
    let with_assisted =
        "fix(table): grip\n\nDetails.\n\nAssisted-by: Claude Code:claude-sonnet-5\n";
    assert_eq!(
        run_with(&cfg, with_assisted),
        "fix(table): grip\n\nAssisted-by: Claude Code:claude-sonnet\n"
    );
    assert_eq!(
        run_with(&cfg, "chore: nothing special\n\nSome body.\n"),
        "chore: nothing special\n"
    );
    let human = "feat: pair\n\nCo-Authored-By: Jane <jane@example.org>\n";
    assert_eq!(
        run_with(&cfg, human),
        "feat: pair\n",
        "other co-authors are stripped, not converted"
    );
}

#[test]
fn rewrite_rules_that_are_not_valid_are_config_errors() {
    for (messages, why) in [
        (
            "  rewrite_trailers: [{match: \"(\", replace: x}]\n",
            "rewrite_trailers[0].match",
        ),
        (
            "  rewrite_trailers: [{match: a, replace: \"x\\ny\"}]\n",
            "single line",
        ),
        ("  rewrite_trailers: [{match: a}]\n", "replace"),
        (
            "  rewrite_trailers: [{match: a, replace: b, extra: 1}]\n",
            "extra",
        ),
        ("  rewrite_trailers: nonsense\n", "invalid type"),
    ] {
        let r = Repo::new();
        r.linear(2, 1_600_000_000);
        r.config(&format!("version: 1\nmessages:\n{messages}"));
        let o = r.gcma(&["plan", "--from", "root"]);
        assert_eq!(
            Repo::code(&o),
            2,
            "{messages}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(
            String::from_utf8_lossy(&o.stderr).contains(why),
            "{why} in {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[test]
fn title_only_and_rewrites_count_as_rules_so_the_config_is_not_inert() {
    let r = Repo::new();
    r.commit_msg("a.txt", b"s\n\nbody\n", 1_600_000_000);
    r.config("version: 1\nmessages:\n  title_only: true\n");
    let o = r.gcma(&["plan", "--from", "root"]);
    assert!(o.status.success());
    assert!(!String::from_utf8_lossy(&o.stderr).contains("no rules are configured"));
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("1 to rewrite"),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
}
