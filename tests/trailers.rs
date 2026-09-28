//! `strip_trailers` and `add_trailers` end to end: exactly which bytes of a message change, and
//! that a second run changes nothing. (`rewrite_trailers` and `title_only` are in
//! `message_rewrites.rs`.)

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
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let first = r.message_bytes("HEAD");
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
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
    r.gcma_ok(&["apply", "--from", "root"]);
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
    r.gcma_ok(&["apply", "--from", "root"]);
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
            r.gcma_ok(&["apply", "--from", "root"])
                .contains("Nothing to do"),
            "{cfg}"
        );
        assert_eq!(r.message_bytes("HEAD"), b"");
    }
}

#[test]
fn trailer_stripping_is_applied_and_idempotent() {
    let r = Repo::new();
    r.commit_at(
        "a.txt",
        "first\n\nbody\n\nSigned-off-by: Old Me <me@home.org>",
        1_600_000_000,
    );
    r.commit_at("b.txt", "second", 1_600_100_000);
    r.config("version: 1\nmessages:\n  strip_trailers: [Signed-off-by]\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    let msg = r.git(&["log", "-1", "--format=%B", "HEAD~1"]);
    assert!(!msg.contains("Signed-off-by"), "{msg}");
    assert!(msg.contains("body"));
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
}

#[test]
fn trailers_can_be_swapped_and_the_result_is_stable() {
    let r = Repo::new();
    r.commit_at(
        "a.txt",
        "first\n\nbody\n\nCo-Authored-By: Bot <bot@x>",
        1_600_000_000,
    );
    r.commit_at("b.txt", "second", 1_600_100_000);
    r.config(
        "version: 1\nmessages:\n  strip_trailers: [Co-Authored-By]\n  add_trailers: [\"Assisted-By: Bot <bot@x>\"]\n",
    );
    r.gcma_ok(&["apply", "--from", "root"]);
    let first = r.git(&["log", "-1", "--format=%B", "HEAD~1"]);
    assert!(!first.contains("Co-Authored-By"), "{first}");
    assert!(
        first.contains("body\n\nAssisted-By: Bot <bot@x>"),
        "{first}"
    );
    let second = r.git(&["log", "-1", "--format=%B", "HEAD"]);
    assert!(
        second.trim_end().ends_with("Assisted-By: Bot <bot@x>"),
        "{second}"
    );
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
    r.fsck();
}

#[test]
fn a_trailer_both_stripped_and_added_is_a_config_error() {
    let r = Repo::new();
    r.linear(1, 1_600_000_000);
    r.config("version: 1\nmessages:\n  strip_trailers: [Assisted-By]\n  add_trailers: [\"assisted-by: x\"]\n");
    assert_eq!(Repo::code(&r.gcma(&["plan", "--from", "root"])), 2);
    r.config("version: 1\nmessages:\n  add_trailers: [\"not a trailer\"]\n");
    assert_eq!(Repo::code(&r.gcma(&["plan", "--from", "root"])), 2);
}
