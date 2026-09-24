//! Trailer rules end to end: exactly which bytes of a message change, and that a second run
//! changes nothing.

mod common;

use common::*;

fn run(strip: &[&str], add: &[&str], message: &str) -> String {
    let r = Repo::new();
    r.commit_msg("a.txt", message.as_bytes(), 1_600_000_000);
    let mut cfg = String::from("version: 1\nmessages:\n");
    if !strip.is_empty() {
        cfg.push_str(&format!("  strip_trailers: [{}]\n", strip.join(", ")));
    }
    if !add.is_empty() {
        cfg.push_str("  add_trailers:\n");
        for a in add {
            cfg.push_str(&format!("    - \"{a}\"\n"));
        }
    }
    r.config(&cfg);
    let o = r.ghma(&["apply", "--from", "root"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let first = r.message_bytes("HEAD");
    assert!(
        r.ghma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do"),
        "a second run must be a no-op for {message:?}"
    );
    assert_eq!(r.message_bytes("HEAD"), first);
    r.fsck();
    String::from_utf8(first).unwrap()
}

#[test]
fn stripping_removes_only_the_named_trailers_of_the_final_block() {
    assert_eq!(
        run(
            &["Signed-off-by", "Co-authored-by"],
            &[],
            "feat: x\n\nbody\n\nSigned-off-by: A <a@x>\nCo-authored-by: B <b@x>\n"
        ),
        "feat: x\n\nbody\n"
    );
    assert_eq!(
        run(
            &["Co-authored-by"],
            &[],
            "feat: x\n\nReviewed-by: R <r@x>\nCo-authored-by: B <b@x>\nSigned-off-by: A <a@x>\n"
        ),
        "feat: x\n\nReviewed-by: R <r@x>\nSigned-off-by: A <a@x>\n"
    );
}

#[test]
fn keys_match_case_insensitively() {
    assert_eq!(
        run(&["signed-OFF-by"], &[], "x\n\nSIGNED-OFF-BY: A <a@x>\n"),
        "x\n"
    );
}

#[test]
fn lookalikes_outside_the_trailer_block_are_left_alone() {
    // In the body, in the subject paragraph, or not followed by a colon.
    let in_body = "x\n\nCo-authored-by: mentioned in prose\nmore prose here\n\nFixes #1\n";
    assert_eq!(run(&["Co-authored-by"], &[], in_body), in_body);
    let subject_only = "Signed-off-by: this is the subject\n";
    assert_eq!(run(&["Signed-off-by"], &[], subject_only), subject_only);
    let no_colon = "x\n\nSigned-off-by A <a@x>\n";
    assert_eq!(run(&["Signed-off-by"], &[], no_colon), no_colon);
}

#[test]
fn stripping_everything_leaves_a_clean_message_without_a_dangling_blank_line() {
    assert_eq!(
        run(
            &["Signed-off-by"],
            &[],
            "only subject\n\nSigned-off-by: A <a@x>\n"
        ),
        "only subject\n"
    );
}

#[test]
fn adding_creates_a_block_or_extends_the_existing_one() {
    let t = "Assisted-By: Claude <noreply@anthropic.com>";
    assert_eq!(run(&[], &[t], "x\n"), format!("x\n\n{t}\n"));
    assert_eq!(
        run(&[], &[t], "x\n\nbody text\n"),
        format!("x\n\nbody text\n\n{t}\n")
    );
    assert_eq!(
        run(&[], &[t], "x\n\nbody\n\nSigned-off-by: A <a@x>\n"),
        format!("x\n\nbody\n\nSigned-off-by: A <a@x>\n{t}\n")
    );
}

#[test]
fn adding_is_skipped_when_the_exact_trailer_is_present_but_not_for_a_different_value() {
    let t = "Assisted-By: Claude <noreply@anthropic.com>";
    let have = format!("x\n\n{t}\n");
    assert_eq!(run(&[], &[t], &have), have);
    assert_eq!(
        run(&[], &[t], "x\n\nAssisted-By: Someone Else <s@x>\n"),
        format!("x\n\nAssisted-By: Someone Else <s@x>\n{t}\n")
    );
}

#[test]
fn several_added_trailers_keep_their_order() {
    let (a, b) = ("Assisted-By: A <a@x>", "Reviewed-by: B <b@x>");
    assert_eq!(run(&[], &[a, b], "x\n"), format!("x\n\n{a}\n{b}\n"));
}

#[test]
fn a_missing_final_newline_is_added_only_when_the_message_changes() {
    let t = "Assisted-By: A <a@x>";
    assert_eq!(run(&[], &[t], "x"), format!("x\n\n{t}\n"));
}

#[test]
fn swapping_one_trailer_for_another_in_a_single_run() {
    assert_eq!(
        run(
            &["Co-authored-by"],
            &["Assisted-By: Claude <noreply@anthropic.com>"],
            "x\n\nSigned-off-by: A <a@x>\nCo-authored-by: Bot <bot@x>\n"
        ),
        "x\n\nSigned-off-by: A <a@x>\nAssisted-By: Claude <noreply@anthropic.com>\n"
    );
}

#[test]
fn merge_commits_get_the_same_treatment_and_keep_their_parents() {
    let r = Repo::new();
    r.commit_msg("base.txt", b"base\n", 1_600_000_000);
    r.git(&["checkout", "-q", "-b", "side"]);
    r.commit_msg("s.txt", b"side\n\nSigned-off-by: A <a@x>\n", 1_600_100_000);
    r.git(&["checkout", "-q", "main"]);
    r.commit_msg("m.txt", b"main\n", 1_600_200_000);
    let date = "1600300000 +0000";
    let o = r
        .cmd("git")
        .args([
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "merge side\n\nSigned-off-by: A <a@x>",
            "side",
        ])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(o.status.success());
    r.config("version: 1\nmessages:\n  strip_trailers: [Signed-off-by]\n  add_trailers: [\"Assisted-By: A <a@x>\"]\n");
    let old = r.log();
    r.ghma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    let merge = new.iter().find(|x| x.parents.len() == 2).unwrap();
    assert_eq!(
        r.messages(&merge.oid)[&merge.oid],
        "merge side\n\nAssisted-By: A <a@x>\n"
    );
    r.fsck();
}

#[test]
fn messages_that_are_not_utf8_survive_trailer_rules_byte_for_byte() {
    let r = Repo::new();
    r.commit_msg(
        "a.txt",
        b"caf\xe9\n\nbody \xe9\n\nSigned-off-by: J\xf6rg <j@x>\n",
        1_600_000_000,
    );
    r.config("version: 1\nmessages:\n  strip_trailers: [Signed-off-by]\n  add_trailers: [\"Assisted-By: A <a@x>\"]\n");
    r.ghma_ok(&["apply", "--from", "root"]);
    assert_eq!(
        r.message_bytes("HEAD"),
        b"caf\xe9\n\nbody \xe9\n\nAssisted-By: A <a@x>\n".to_vec()
    );
}

#[test]
fn an_empty_message_is_left_alone_by_every_rule() {
    let r = Repo::new();
    r.commit_msg("a.txt", b"", 1_600_000_000);
    for cfg in [
        "version: 1\nmessages:\n  strip_trailers: [Signed-off-by]\n",
        "version: 1\nmessages:\n  add_trailers: [\"Assisted-By: A <a@x>\"]\n",
        "version: 1\nmessages:\n  title_only: true\n  rewrite_trailers: [{match: \"^A\", replace: \"B\"}]\n",
    ] {
        r.config(cfg);
        assert!(
            r.ghma_ok(&["apply", "--from", "root"])
                .contains("Nothing to do"),
            "{cfg}"
        );
        assert_eq!(r.message_bytes("HEAD"), b"");
    }
}

// ---------- rewrite_trailers and title_only ----------

fn run_with(messages_cfg: &str, message: &str) -> String {
    let r = Repo::new();
    r.commit_msg("a.txt", message.as_bytes(), 1_600_000_000);
    r.config(&format!("version: 1\nmessages:\n{messages_cfg}"));
    let o = r.ghma(&["apply", "--from", "root"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let first = r.message_bytes("HEAD");
    assert!(
        r.ghma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do"),
        "a second run must be a no-op for {message:?}"
    );
    assert!(
        r.ghma(&["plan", "--check", "--from", "root"])
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
        let o = r.ghma(&["plan", "--from", "root"]);
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
    let o = r.ghma(&["plan", "--from", "root"]);
    assert!(o.status.success());
    assert!(!String::from_utf8_lossy(&o.stderr).contains("no rules are configured"));
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("1 to rewrite"),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
}
