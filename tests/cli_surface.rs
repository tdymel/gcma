//! The command line as a user meets it: help, usage errors, `init`, the backup commands and
//! what happens when the output pipe closes.

mod common;

use common::*;

fn out(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}
fn err(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

#[test]
fn help_and_version_work_everywhere_and_list_every_command() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    for flag in ["--help", "-h"] {
        let o = base_cmd(bin(), dir.path(), home.path())
            .arg(flag)
            .output()
            .unwrap();
        assert_eq!(Repo::code(&o), 0, "{}", err(&o));
        for word in [
            "init",
            "plan",
            "apply",
            "restore",
            "export",
            "import",
            "hook",
            "--backend",
            "--config",
        ] {
            assert!(
                out(&o).contains(word),
                "{word} missing from help:\n{}",
                out(&o)
            );
        }
    }
    let o = base_cmd(bin(), dir.path(), home.path())
        .arg("--version")
        .output()
        .unwrap();
    assert!(out(&o).starts_with("gcma "), "{}", out(&o));
    for sub in [
        "plan", "apply", "restore", "export", "import", "hook", "init",
    ] {
        let o = base_cmd(bin(), dir.path(), home.path())
            .args([sub, "--help"])
            .output()
            .unwrap();
        assert_eq!(Repo::code(&o), 0, "{sub}: {}", err(&o));
    }
}

#[test]
fn usage_mistakes_exit_2_and_say_what_was_wrong() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.config(IDENTITY_CFG);
    for (args, why) in [
        (vec!["--bogus"], "unexpected argument"),
        (vec!["bogus"], "unrecognized subcommand"),
        (vec![], "Usage"),
        (vec!["plan", "--from"], "value"),
        (vec!["plan", "--backend", "svn"], "invalid value"),
        (vec!["export", "--batch", "many"], "invalid value"),
        (vec!["hook", "run", "post-commit"], "unsupported hook"),
        (vec!["restore", "--prune"], "needs a backup id"),
        (vec!["restore", "no-such-backup"], "no backup matches"),
        (
            vec!["restore", "no-such-backup", "--prune"],
            "no backup matches",
        ),
    ] {
        let o = r.gcma(&args);
        assert_eq!(Repo::code(&o), 2, "{args:?}: {}{}", out(&o), err(&o));
        assert!(
            err(&o).contains(why),
            "{args:?}: expected {why:?} in {}",
            err(&o)
        );
    }
}

#[test]
fn the_dir_option_works_from_anywhere() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let elsewhere = tempfile::tempdir().unwrap();
    let o = base_cmd(bin(), elsewhere.path(), r.home.path())
        .args(["-C"])
        .arg(r.path())
        .args(["apply", "--from", "root"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
}

#[test]
fn a_config_can_live_anywhere_when_named() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    let cfg = r.home.path().join("elsewhere.yml");
    std::fs::write(&cfg, IDENTITY_CFG).unwrap();
    let cfg = cfg.to_str().unwrap();
    assert!(
        r.gcma_ok(&["--config", cfg, "plan", "--from", "root"])
            .contains("2 to rewrite")
    );
    r.gcma_ok(&["--config", cfg, "apply", "--from", "root"]);
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
    assert!(!r.path().join("gcma.yml").exists());
}

#[test]
fn init_writes_once_refuses_to_overwrite_and_force_replaces() {
    let r = Repo::new();
    r.linear(1, 1_600_000_000);
    let o = r.gcma(&["init"]);
    assert_eq!(Repo::code(&o), 0, "{}", err(&o));
    let first = std::fs::read_to_string(r.path().join("gcma.yml")).unwrap();
    assert!(first.contains("version: 1"));
    let exclude = std::fs::read_to_string(r.path().join(".git/info/exclude")).unwrap();
    assert_eq!(exclude.matches("/gcma.yml").count(), 1);

    std::fs::write(r.path().join("gcma.yml"), "version: 1\n# mine\n").unwrap();
    let o = r.gcma(&["init"]);
    assert_eq!(Repo::code(&o), 3, "{}", err(&o));
    assert!(err(&o).contains("--force"), "{}", err(&o));
    assert!(
        std::fs::read_to_string(r.path().join("gcma.yml"))
            .unwrap()
            .contains("# mine")
    );

    r.gcma_ok(&["init", "--force"]);
    assert_eq!(
        std::fs::read_to_string(r.path().join("gcma.yml")).unwrap(),
        first
    );
    let exclude = std::fs::read_to_string(r.path().join(".git/info/exclude")).unwrap();
    assert_eq!(
        exclude.matches("/gcma.yml").count(),
        1,
        "no duplicate entries"
    );
    assert!(
        r.git(&["status", "--porcelain"]).is_empty(),
        "the config does not show up in git status"
    );
}

#[test]
fn two_rewrites_in_one_second_keep_two_distinct_backups() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    r.config("version: 1\nmessages:\n  add_trailers: [\"Assisted-By: A <a@x>\"]\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    let listing = r.gcma_ok(&["restore"]);
    let ids: Vec<&str> = listing
        .lines()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "{listing}");
    assert_ne!(ids[0], ids[1]);
    for l in listing.lines() {
        assert!(
            l.contains("branch main") && l.contains("old ") && l.contains("new "),
            "{l}"
        );
    }
    // Both backups hold their own old tip; the chain restores step by step.
    let tip = r.git(&["rev-parse", "HEAD"]);
    let newest = listing
        .lines()
        .find(|l| l.contains(&format!("new {tip}")))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    r.gcma_ok(&["restore", &newest]);
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
    assert!(
        !r.messages("HEAD")
            .values()
            .any(|m| m.contains("Assisted-By"))
    );
}

#[test]
fn pruning_a_backup_forgets_it_and_listing_shows_none() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    let listing = r.gcma_ok(&["restore"]);
    let id = listing.split_whitespace().next().unwrap().to_string();
    let o = r.gcma(&["restore", &id, "--prune"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("pruned backup"), "{}", out(&o));
    assert_eq!(r.gcma_ok(&["restore"]).trim(), "No backups.");
    let o = r.gcma(&["restore", &id]);
    assert_eq!(Repo::code(&o), 2, "{}", err(&o));
}

#[test]
fn restoring_by_an_unambiguous_prefix_works_and_an_ambiguous_one_is_refused() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    let id = r
        .gcma_ok(&["restore"])
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let o = r.gcma(&["restore", &id[..4]]);
    assert!(o.status.success(), "a prefix is enough: {}", err(&o));
    let o = r.gcma(&["restore", ""]);
    assert!(
        !o.status.success(),
        "the empty prefix must not match everything silently"
    );
}

#[test]
fn a_closed_output_pipe_is_not_a_crash() {
    let r = Repo::new();
    r.linear(300, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let script = format!(
        "{} plan --from root | head -n 1 > /dev/null; echo ${{PIPESTATUS[0]}}",
        bin()
    );
    let o = r.cmd("bash").args(["-c", &script]).output().unwrap();
    assert!(!err(&o).contains("panicked"), "{}", err(&o));
    assert!(!err(&o).contains("Broken pipe"), "{}", err(&o));
    let code: i32 = out(&o).trim().parse().unwrap();
    assert!(code == 0 || code == 141, "unexpected exit code {code}");
}

#[test]
fn export_boundaries_are_clean() {
    let r = Repo::new();
    r.linear(5, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let rows = |args: &[&str]| {
        let mut a = vec!["export", "--from", "root"];
        a.extend_from_slice(args);
        let o = r.gcma(&a);
        assert!(o.status.success(), "{args:?}: {}", err(&o));
        out(&o).lines().count()
    };
    assert_eq!(rows(&[]), 5);
    assert_eq!(rows(&["--batch", "2"]), 2);
    assert_eq!(rows(&["--batch", "2", "--offset", "4"]), 1);
    assert_eq!(rows(&["--batch", "2", "--offset", "5"]), 0);
    assert_eq!(
        rows(&["--offset", "99"]),
        0,
        "an offset past the end is an empty batch"
    );
    assert_eq!(rows(&["--batch", "0"]), 0);
    assert_eq!(rows(&["--batch", "99999999999"]), 5);
    // The prelude (instructions) goes to stderr, rows only to stdout.
    let o = r.gcma(&["export", "--from", "root", "--batch", "1"]);
    assert!(err(&o).contains("Reply with JSONL ONLY"), "{}", err(&o));
    assert!(err(&o).contains("Rows 0..1 of 5"), "{}", err(&o));
    for l in out(&o).lines() {
        assert!(l.starts_with('{'), "{l}");
    }
}
