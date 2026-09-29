//! The command line as a user meets it: help, usage errors, `-C` and `--config`, `init`, and what
//! happens when the output pipe closes.

mod common;

use common::*;

#[test]
fn help_and_version_work_everywhere_and_list_every_command() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    for flag in ["--help", "-h"] {
        let o = base_cmd(bin(), dir.path(), home.path())
            .arg(flag)
            .output()
            .unwrap();
        assert_eq!(Repo::code(&o), 0, "{}", stderr(&o));
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
                stdout(&o).contains(word),
                "{word} missing from help:\n{}",
                stdout(&o)
            );
        }
    }
    let o = base_cmd(bin(), dir.path(), home.path())
        .arg("--version")
        .output()
        .unwrap();
    assert!(stdout(&o).starts_with("gcma "), "{}", stdout(&o));
    for sub in [
        "plan", "apply", "restore", "export", "import", "hook", "init",
    ] {
        let o = base_cmd(bin(), dir.path(), home.path())
            .args([sub, "--help"])
            .output()
            .unwrap();
        assert_eq!(Repo::code(&o), 0, "{sub}: {}", stderr(&o));
    }
}

#[test]
fn usage_mistakes_exit_2_and_say_what_was_wrong() {
    let r = Repo::new();
    r.linear(2, T0);
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
        assert_eq!(Repo::code(&o), 2, "{args:?}: {}{}", stdout(&o), stderr(&o));
        assert!(
            stderr(&o).contains(why),
            "{args:?}: expected {why:?} in {}",
            stderr(&o)
        );
    }
}

#[test]
fn the_dir_option_works_from_anywhere() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    let elsewhere = tempfile::tempdir().unwrap();
    let o = base_cmd(bin(), elsewhere.path(), r.home.path())
        .args(["-C"])
        .arg(r.path())
        .args(["apply", "--from", "root"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
}

#[test]
fn a_config_can_live_anywhere_when_named() {
    let r = Repo::new();
    r.linear(2, T0);
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
    r.linear(1, T0);
    let o = r.gcma(&["init"]);
    assert_eq!(Repo::code(&o), 0, "{}", stderr(&o));
    let first = std::fs::read_to_string(r.path().join("gcma.yml")).unwrap();
    assert!(first.contains("version: 1"));
    let exclude = std::fs::read_to_string(r.path().join(".git/info/exclude")).unwrap();
    assert_eq!(exclude.matches("/gcma.yml").count(), 1);

    std::fs::write(r.path().join("gcma.yml"), "version: 1\n# mine\n").unwrap();
    let o = r.gcma(&["init"]);
    assert_eq!(Repo::code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("--force"), "{}", stderr(&o));
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
fn init_keeps_the_config_out_of_commits_and_starts_inert() {
    let r = Repo::new();
    r.linear(2, T0);
    r.gcma_ok(&["init"]);
    assert!(
        r.git(&["status", "--porcelain"]).is_empty(),
        "the config is excluded"
    );
    let o = r.gcma(&["plan", "--from", "root"]);
    assert!(o.status.success());
    assert!(
        stderr(&o).contains("no rules are configured"),
        "{}",
        stderr(&o)
    );
    assert!(stdout(&o).contains("Nothing to do"));
}

#[test]
fn a_closed_output_pipe_is_not_a_crash() {
    let r = Repo::new();
    r.linear(300, T0);
    r.config(IDENTITY_CFG);
    let script = format!(
        "{} plan --from root | head -n 1 > /dev/null; echo ${{PIPESTATUS[0]}}",
        bin()
    );
    let o = r.cmd("bash").args(["-c", &script]).output().unwrap();
    assert!(!stderr(&o).contains("panicked"), "{}", stderr(&o));
    assert!(!stderr(&o).contains("Broken pipe"), "{}", stderr(&o));
    let code: i32 = stdout(&o).trim().parse().unwrap();
    assert!(code == 0 || code == 141, "unexpected exit code {code}");
}
