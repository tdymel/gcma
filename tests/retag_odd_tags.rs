//! `gcma apply --retag` with tags that need care: tags of tags, legacy tags without a tagger, and
//! names that are not UTF-8. Every scenario runs with both object backends (only `git` when `gix`
//! is not built).

mod common;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use common::*;

/// Runs `scenario` on a fresh repository for each backend.
fn on_both_backends(scenario: impl Fn(Repo)) {
    for seed in [0, 1] {
        scenario(Repo::for_seed(seed));
    }
}

/// Three commits by the old identity, which the config rewrites; every one of them is replaced.
fn three_to_rewrite(r: &Repo) -> Vec<String> {
    let old = r.linear(3, T0);
    r.config(IDENTITY_CFG);
    old
}

/// The id of the commit a ref finally points at.
fn peeled(r: &Repo, name: &str) -> String {
    r.git(&["rev-parse", &format!("{name}^{{commit}}")])
}

/// The id of what a tag ref holds (a tag object for an annotated tag).
fn held(r: &Repo, name: &str) -> String {
    r.git(&["rev-parse", &format!("refs/tags/{name}")])
}

#[test]
fn a_tag_of_a_tag_stays_and_is_warned_about_while_the_inner_tag_moves() {
    on_both_backends(|r| {
        three_to_rewrite(&r);
        r.git(&["tag", "-a", "-m", "y", "old", "HEAD~2"]);
        r.git(&["tag", "-a", "-m", "nest", "nest", "refs/tags/old"]);
        let (outer, inner, tags) = (
            held(&r, "nest"),
            held(&r, "old"),
            r.git(&["for-each-ref", "refs/tags"]),
        );
        let plan = r.gcma_ok(&["plan", "--retag", "--from", "root"]);
        assert!(
            plan.contains("1 tag(s) would be moved (--retag): old"),
            "{plan}"
        );
        assert!(
            plan.contains("warning: tag(s) of tags") && plan.contains("nest"),
            "{plan}"
        );
        let o = r.gcma(&["apply", "--retag", "--from", "root"]);
        assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
        assert!(stderr(&o).contains("nest"), "{}", stderr(&o));
        assert_eq!(held(&r, "nest"), outer, "the tag of the tag stays");
        assert_ne!(held(&r, "old"), inner, "the inner tag is recreated");
        assert_eq!(peeled(&r, "old"), r.log()[0].oid);
        r.fsck();
        r.gcma_ok(&["restore", &r.backup_id()]);
        assert_eq!(r.git(&["for-each-ref", "refs/tags"]), tags);
    });
}

/// Writes a tag object that `git fsck` finds fault with, and a ref to it.
fn tag_without_tagger(r: &Repo, name: &str, commit: &str) -> String {
    let raw = format!("object {commit}\ntype commit\ntag {name}\n\nlegacy tag\n");
    let mut child = r
        .cmd("git")
        .args(["hash-object", "-t", "tag", "--literally", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), raw.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let oid = String::from_utf8(out.stdout).unwrap().trim().to_string();
    r.git(&["update-ref", &format!("refs/tags/{name}"), &oid]);
    oid
}

#[test]
fn a_legacy_annotated_tag_without_a_tagger_follows_its_commit() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        let before = tag_without_tagger(&r, "legacy", &old[1]);
        let o = r.gcma(&["apply", "--retag", "--from", "root"]);
        assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
        assert_ne!(held(&r, "legacy"), before);
        assert_eq!(peeled(&r, "legacy"), r.log()[1].oid);
        let body = r.git(&["cat-file", "tag", "refs/tags/legacy"]);
        assert!(!body.contains("tagger"), "{body}");
        r.gcma_ok(&["restore", &r.backup_id()]);
        assert_eq!(held(&r, "legacy"), before);
    });
}

#[test]
fn a_tag_whose_name_is_not_utf8_stays_and_is_warned_about() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        // Some file systems (APFS on macOS) refuse names that are not valid UTF-8 (EILSEQ).
        let probe = r.path().join(OsStr::from_bytes(b".git/caf\xe9-probe"));
        match std::fs::write(&probe, "") {
            Ok(()) => std::fs::remove_file(&probe).unwrap(),
            Err(e) if matches!(e.raw_os_error(), Some(84 | 92)) => {
                eprintln!("skipping: this file system rejects non-UTF-8 names ({e})");
                return;
            }
            Err(e) => panic!("cannot create the probe file: {e}"),
        }
        let name = OsStr::from_bytes(b"caf\xe9");
        let made = r
            .cmd("git")
            .arg("tag")
            .arg(name)
            .arg(&old[1])
            .output()
            .unwrap();
        assert!(made.status.success(), "{}", stderr(&made));
        r.git(&["tag", "v1", &old[2]]);
        let plan = r.gcma_ok(&["plan", "--retag", "--from", "root"]);
        assert!(plan.contains("name is not valid UTF-8"), "{plan}");
        assert!(
            plan.contains("1 tag(s) would be moved (--retag): v1"),
            "{plan}"
        );
        let o = r.gcma(&["apply", "--retag", "--from", "root"]);
        assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
        assert!(
            stderr(&o).contains("name is not valid UTF-8"),
            "{}",
            stderr(&o)
        );
        let full = OsStr::from_bytes(b"refs/tags/caf\xe9");
        let kept = r.cmd("git").arg("rev-parse").arg(full).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&kept.stdout).trim(), old[1]);
        assert_eq!(held(&r, "v1"), r.log()[2].oid, "the others still move");
        r.fsck();
    });
}

#[test]
fn a_symbolic_tag_follows_its_target_and_is_not_moved_itself() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "light", &old[1]]);
        r.git(&["symbolic-ref", "refs/tags/latest", "refs/tags/light"]);
        let plan = r.gcma_ok(&["plan", "--retag", "--from", "root"]);
        assert!(
            plan.contains("1 tag(s) would be moved (--retag): light\n"),
            "{plan}"
        );
        assert!(!plan.contains("latest"), "{plan}");
        let o = r.gcma(&["apply", "--retag", "--from", "root"]);
        assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
        assert!(!stderr(&o).contains("latest"), "{}", stderr(&o));
        assert_eq!(held(&r, "light"), r.log()[1].oid);
        let target = |r: &Repo| r.git(&["symbolic-ref", "refs/tags/latest"]);
        assert_eq!(target(&r), "refs/tags/light");
        r.fsck();
        r.gcma_ok(&["restore", &r.backup_id()]);
        assert_eq!(held(&r, "light"), old[1]);
        assert_eq!(target(&r), "refs/tags/light");
    });
}

#[test]
fn a_symbolic_tag_to_the_branch_does_not_clash_with_the_branch_update() {
    on_both_backends(|r| {
        three_to_rewrite(&r);
        r.git(&["symbolic-ref", "refs/tags/x", "refs/heads/main"]);
        let plan = r.gcma_ok(&["plan", "--retag", "--from", "root"]);
        assert!(!plan.contains("tag(s) would be moved"), "{plan}");
        let o = r.gcma(&["apply", "--retag", "--from", "root"]);
        assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
        assert_eq!(r.git(&["symbolic-ref", "refs/tags/x"]), "refs/heads/main");
        assert_eq!(held(&r, "x"), r.log()[2].oid);
        r.fsck();
    });
}
