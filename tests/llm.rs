//! The export / import workflow: what an LLM sees, what it may answer, and what is refused.

mod common;

use common::*;
use std::io::Write;

fn out(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}
fn err(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn setup(cfg: &str) -> (Repo, String) {
    let r = Repo::new();
    r.commit_msg(
        "a.txt",
        b"wip\n\nSigned-off-by: Dev <dev@x.org>\n",
        1_600_000_000,
    );
    r.commit_msg("b.txt", b"fix stuff\n", 1_600_100_000);
    r.commit_as("c.txt", "more", 1_600_200_000, "Other Dev", "other@x.org");
    r.config(cfg);
    let plan = r.path().join("plan.json").to_str().unwrap().to_string();
    r.ghma_ok(&["plan", "--from", "root", "--all", "--out", &plan]);
    (r, plan)
}

fn rows(r: &Repo, plan: &str) -> Vec<serde_json::Value> {
    let o = r.ghma(&["export", "--plan", plan]);
    assert!(o.status.success(), "{}", err(&o));
    out(&o)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn export_describes_each_commit_and_names_the_author_only_when_it_differs() {
    let (r, plan) = setup("version: 1\n");
    let rows = rows(&r, &plan);
    assert_eq!(rows.len(), 3);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row["i"], i);
        assert!(row["d"].as_str().unwrap().starts_with("2020-09-"), "{row}");
        assert!(row["s"].as_str().unwrap().starts_with("+1-0 1f"), "{row}");
    }
    assert!(rows[0].get("a").is_none() && rows[1].get("a").is_none());
    assert_eq!(rows[2]["a"], "Other Dev <other@x.org>");
    assert_eq!(rows[0]["m"], "wip\n\nSigned-off-by: Dev <dev@x.org>");
    assert_eq!(rows[1]["m"], "fix stuff");
}

#[test]
fn a_reply_from_stdin_is_applied_and_the_protected_trailer_is_kept() {
    let (r, plan) = setup("version: 1\n");
    let reply = "{\"i\":0,\"t\":\"Add the first file\",\"b\":\"Why.\\n\\nSigned-off-by: Dev <dev@x.org>\"}\n{\"i\":1,\"t\":\"Add the second file\"}\n";
    let mut child = r
        .cmd(bin())
        .args(["import", "--plan", &plan, "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(reply.as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("imported 2 message(s) (0 unchanged)"),
        "{}",
        out(&o)
    );
    r.ghma_ok(&["apply", "--plan", &plan]);
    let subjects: Vec<String> = r.log().into_iter().map(|x| x.subject).collect();
    assert_eq!(
        subjects,
        ["Add the first file", "Add the second file", "more"]
    );
    assert!(
        r.messages("HEAD~2")
            .values()
            .next()
            .unwrap()
            .contains("Signed-off-by: Dev <dev@x.org>")
    );
    r.fsck();
}

#[test]
fn out_writes_a_new_plan_and_leaves_the_original_untouched() {
    let (r, plan) = setup("version: 1\n");
    let reply = r.path().join("reply.jsonl");
    std::fs::write(&reply, "{\"i\":1,\"t\":\"Better title\"}\n").unwrap();
    let edited = r.path().join("edited.json");
    let before = std::fs::read(&plan).unwrap();
    r.ghma_ok(&[
        "import",
        "--plan",
        &plan,
        "--out",
        edited.to_str().unwrap(),
        reply.to_str().unwrap(),
    ]);
    assert_eq!(std::fs::read(&plan).unwrap(), before);
    assert_ne!(std::fs::read(&edited).unwrap(), before);
    r.ghma_ok(&["apply", "--plan", edited.to_str().unwrap()]);
    assert_eq!(r.log()[1].subject, "Better title");
}

#[test]
fn rules_from_the_config_are_put_back_after_the_reply() {
    let (r, plan) = setup(
        "version: 1\nmessages:\n  strip_trailers: [Signed-off-by]\n  add_trailers: [\"Assisted-By: A <a@x>\"]\n",
    );
    let reply = r.path().join("reply.jsonl");
    std::fs::write(
        &reply,
        "{\"i\":0,\"t\":\"Nicer\",\"b\":\"Body text.\"}\n{\"i\":1,\"t\":\"Second\"}\n",
    )
    .unwrap();
    r.ghma_ok(&["import", "--plan", &plan, reply.to_str().unwrap()]);
    r.ghma_ok(&["apply", "--plan", &plan]);
    let msgs = r.messages("HEAD");
    let log = r.log();
    assert_eq!(
        msgs[&log[0].oid],
        "Nicer\n\nBody text.\n\nAssisted-By: A <a@x>\n"
    );
    assert_eq!(msgs[&log[1].oid], "Second\n\nAssisted-By: A <a@x>\n");
    assert!(
        r.ghma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
}

#[test]
fn a_bad_reply_changes_nothing_exits_7_and_lists_the_rows_to_retry() {
    let (r, plan) = setup("version: 1\n");
    let before = std::fs::read(&plan).unwrap();
    let reply = r.path().join("reply.jsonl");
    for (text, needle) in [
        ("Sure! Here you go:\n{\"i\":0,\"t\":\"x\"}\n", "line 1"),
        (
            "{\"i\":0,\"t\":\"ok\"}\n{\"i\":0,\"t\":\"again\"}\n",
            "duplicate index",
        ),
        (
            "{\"i\":0,\"t\":\"ok\"}\n{\"i\":7,\"t\":\"x\"}\n",
            "unknown index",
        ),
        ("{\"i\":0,\"t\":\"two\\nlines\"}\n", "single line"),
        ("{\"i\":0,\"t\":\"  \"}\n", "empty title"),
        ("{\"i\":0,\"t\":\"x\\u0000y\"}\n", "control characters"),
        (
            "{\"i\":0,\"t\":\"x\"}\n{\"i\":1,\"t\":\"y\",\"extra\":1}\n",
            "line 2",
        ),
        ("{\"i\":0,\"t\":\"x\"}\n```\n", "line 2"),
        ("{\"i\":0,\"t\":\"Better\"}\n", "was dropped"),
        (
            "{\"i\":0,\"t\":\"x\",\"b\":\"Signed-off-by: Dev <dev@x.org>\\nSigned-off-by: Mallory <m@x>\"}\n",
            "is new",
        ),
    ] {
        std::fs::write(&reply, text).unwrap();
        let o = r.ghma(&["import", "--plan", &plan, reply.to_str().unwrap()]);
        assert_eq!(Repo::code(&o), 7, "{text:?}: {}", err(&o));
        assert!(
            err(&o).contains(needle),
            "{text:?}: expected {needle:?} in {}",
            err(&o)
        );
        assert!(err(&o).contains("nothing was imported"), "{}", err(&o));
        assert_eq!(
            std::fs::read(&plan).unwrap(),
            before,
            "the plan is untouched after {text:?}"
        );
    }
}

#[test]
fn a_missing_reply_or_plan_file_is_a_usage_error() {
    let (r, plan) = setup("version: 1\n");
    let o = r.ghma(&["import", "--plan", &plan, "no-such-reply.jsonl"]);
    assert_eq!(Repo::code(&o), 2, "{}", err(&o));
    assert!(err(&o).contains("no-such-reply.jsonl"));
    let o = r.ghma(&["import", "--plan", "no-such-plan.json", "-"]);
    assert_eq!(Repo::code(&o), 2, "{}", err(&o));
}

#[test]
fn an_empty_reply_imports_nothing() {
    let (r, plan) = setup("version: 1\n");
    let reply = r.path().join("reply.jsonl");
    std::fs::write(&reply, "\n\n").unwrap();
    let o = r.ghma(&["import", "--plan", &plan, reply.to_str().unwrap()]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("imported 0 message(s)"), "{}", out(&o));
}

#[test]
fn control_characters_in_the_prelude_cannot_reach_the_terminal() {
    let (r, plan) = setup("version: 1\n");
    // The author named in the prelude comes from the plan file, which is untrusted input.
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    json["entries"][2]["author"]["name"] = serde_json::json!("Evil\u{1b}[2J");
    std::fs::write(&plan, serde_json::to_vec(&json).unwrap()).unwrap();
    let o = r.ghma(&["export", "--plan", &plan]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        !err(&o).contains('\u{1b}'),
        "escape sequences must be neutralised on stderr"
    );
}
