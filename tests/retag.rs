//! `gcma apply --retag`: tags (and notes) follow the commits a rewrite replaces, in the same ref
//! transaction as the branch, and `gcma restore` puts them back. Every scenario runs with both object
//! backends (only `git` when `gix` is not built).

mod common;

use common::*;

const SIGNED_MESSAGE: &str = "signed release\n-----BEGIN PGP SIGNATURE-----\nnot a real signature\n-----END PGP SIGNATURE-----";

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

/// The tag object without its first line, which names the target.
fn tag_body(r: &Repo, name: &str) -> String {
    let raw = r.git(&["cat-file", "tag", &format!("refs/tags/{name}")]);
    raw.split_once('\n').unwrap().1.to_string()
}

#[test]
fn a_lightweight_tag_follows_its_commit() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "v1", &old[1]]);
        let out = r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        let rows = r.log();
        assert_eq!(held(&r, "v1"), rows[1].oid);
        assert_ne!(rows[1].oid, old[1]);
        assert!(out.contains("Moved 1 tag(s): v1"), "{out}");
        r.fsck();
    });
}

#[test]
fn an_unsigned_annotated_tag_is_recreated_with_its_message_and_tagger() {
    on_both_backends(|r| {
        three_to_rewrite(&r);
        r.git(&[
            "tag",
            "-a",
            "-m",
            "release 2\n\nwith a body",
            "v2",
            "HEAD~1",
        ]);
        let (old_object, old_body) = (held(&r, "v2"), tag_body(&r, "v2"));
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        let rows = r.log();
        assert_ne!(held(&r, "v2"), old_object, "a new tag object");
        assert_eq!(r.git(&["cat-file", "-t", "refs/tags/v2"]), "tag");
        assert_eq!(peeled(&r, "v2"), rows[1].oid);
        assert_eq!(tag_body(&r, "v2"), old_body, "name, tagger, date, message");
        assert!(old_body.contains("tagger Old Me <me@home.org>"));
        r.fsck();
    });
}

#[test]
fn a_signed_annotated_tag_stays_and_is_warned_about() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "-a", "-m", SIGNED_MESSAGE, "v3", &old[1]]);
        r.git(&["tag", "v1", &old[2]]);
        let before = held(&r, "v3");
        let o = r.gcma(&["apply", "--retag", "--from", "root"]);
        assert!(o.status.success(), "{}", stderr(&o));
        assert!(
            stderr(&o).contains("warning: signed tag(s)") && stderr(&o).contains("v3"),
            "{}",
            stderr(&o)
        );
        assert_eq!(held(&r, "v3"), before, "the signed tag is left alone");
        assert_eq!(peeled(&r, "v3"), old[1]);
        assert_eq!(held(&r, "v1"), r.log()[2].oid, "the others still move");
    });
}

#[test]
fn a_tag_on_a_dropped_commit_is_warned_about_and_stays() {
    on_both_backends(|r| {
        r.commit_at("a.txt", "add a", T0);
        let secret = r.commit_files(&[("secrets/key.pem", "k\n")], "add key", T0 + 3600);
        r.commit_at("c.txt", "add c", T0 + 7200);
        r.config(SECRETS_CFG);
        r.git(&["tag", "gone", &secret]);
        r.git(&["tag", "tip", "HEAD"]);
        let o = r.gcma(&["apply", "--retag", "--from", "root"]);
        assert!(o.status.success(), "{}", stderr(&o));
        assert!(
            stderr(&o).contains("warning: tag(s) point at commits that are dropped")
                && stderr(&o).contains("gone"),
            "{}",
            stderr(&o)
        );
        assert_eq!(held(&r, "gone"), secret);
        assert_eq!(held(&r, "tip"), r.log()[1].oid);
        r.fsck();
    });
}

#[test]
fn tags_on_commits_that_conform_are_untouched() {
    on_both_backends(|r| {
        // The first commit already conforms (and so does everything below it), the second does not.
        let kept = r.commit_as("k.txt", "kept", T0, "Jane Doe", "jane@work.com");
        let old = r.commit_at("o.txt", "old identity", T0 + 3600);
        r.config(IDENTITY_CFG);
        r.git(&["tag", "kept", &kept]);
        r.git(&["tag", "moved", &old]);
        let out = r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        assert_eq!(held(&r, "kept"), kept);
        assert_eq!(held(&r, "moved"), r.log()[1].oid);
        assert!(out.contains("Moved 1 tag(s): moved"), "{out}");
    });
}

#[test]
fn a_commit_that_is_rewritten_into_itself_keeps_its_tag() {
    on_both_backends(|r| {
        let same = r.commit_as("k.txt", "kept", T0, "Jane Doe", "jane@work.com");
        r.commit_at("o.txt", "old identity", T0 + 3600);
        r.config(IDENTITY_CFG);
        r.git(&["tag", "same", &same]);
        let out = r.gcma_ok(&["apply", "--all", "--retag", "--from", "root"]);
        assert_eq!(r.log()[0].oid, same, "the same commit comes out");
        assert_eq!(held(&r, "same"), same);
        assert!(!out.contains("Moved"), "{out}");
    });
}

#[test]
fn tags_below_the_range_are_untouched() {
    on_both_backends(|r| {
        let old = r.linear(4, T0);
        r.config(IDENTITY_CFG);
        r.git(&["tag", "below", &old[0]]);
        r.gcma_ok(&["apply", "--retag", "--from", &old[1]]);
        assert_eq!(held(&r, "below"), old[0]);
    });
}

#[test]
fn without_retag_tags_stay_and_the_warning_points_at_the_flag() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "v1", &old[1]]);
        r.git(&["tag", "-a", "-m", "release", "v2", &old[2]]);
        let before = r.git(&["for-each-ref", "refs/tags"]);
        let o = r.gcma(&["apply", "--from", "root"]);
        assert!(o.status.success(), "{}", stderr(&o));
        let err = stderr(&o);
        assert!(err.contains("warning: tags/notes point at"), "{err}");
        assert!(
            err.contains("refs/tags/v1") && err.contains("refs/tags/v2"),
            "{err}"
        );
        assert!(err.contains("pass --retag to move them"), "{err}");
        assert_eq!(r.git(&["for-each-ref", "refs/tags"]), before);
        assert_eq!(r.backup_count(), 1);
        assert!(!r.refs().contains("/tags "), "a backup without tag records");
    });
}

#[test]
fn plan_shows_the_tags_it_would_move_and_those_it_would_not() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "v1", &old[0]]);
        r.git(&["tag", "-a", "-m", "release", "v2", &old[1]]);
        r.git(&["tag", "-a", "-m", SIGNED_MESSAGE, "v3", &old[2]]);
        let refs = r.refs();
        let out = r.gcma_ok(&["plan", "--retag", "--from", "root"]);
        assert!(
            out.contains("2 tag(s) would be moved (--retag): v1, v2"),
            "{out}"
        );
        assert!(out.contains("warning: signed tag(s)") && out.contains("v3"));
        assert!(!out.contains("pass --retag"), "{out}");
        assert_eq!(r.refs(), refs, "a plan changes nothing");
        let plain = r.gcma_ok(&["plan", "--from", "root"]);
        assert!(
            plain.contains("refs/tags/v1") && plain.contains("refs/tags/v2"),
            "{plain}"
        );
        assert!(plain.contains("pass --retag to move them"), "{plain}");
    });
}

#[test]
fn a_saved_plan_can_be_applied_with_retag() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "v1", &old[2]]);
        let plan = r.path().join("plan.json");
        r.gcma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
        r.gcma_ok(&["apply", "--retag", "--plan", plan.to_str().unwrap()]);
        assert_eq!(held(&r, "v1"), r.git(&["rev-parse", "HEAD"]));
    });
}

#[test]
fn a_run_that_changes_nothing_moves_no_tag_and_a_rerun_is_a_noop() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "v1", &old[1]]);
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        let refs = r.refs();
        let out = r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        assert!(out.contains("Nothing to do"), "{out}");
        assert_eq!(r.refs(), refs);
    });
}

#[test]
fn restore_puts_every_tag_back() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "v1", &old[0]]);
        r.git(&["tag", "-a", "-m", "release", "v2", &old[1]]);
        let (tags, tip) = (r.git(&["for-each-ref", "refs/tags"]), old[2].clone());
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        assert_ne!(r.git(&["for-each-ref", "refs/tags"]), tags);
        let id = r.backup_id();
        r.gcma_ok(&["restore", &id]);
        assert_eq!(r.git(&["for-each-ref", "refs/tags"]), tags);
        assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
        r.fsck();
    });
}

#[test]
fn the_old_annotated_tag_object_stays_reachable_through_the_backup() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "-a", "-m", "release", "v2", &old[1]]);
        let old_object = held(&r, "v2");
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        r.git(&["gc", "-q", "--prune=now"]);
        assert_eq!(r.git(&["cat-file", "-t", &old_object]), "tag");
        let id = r.backup_id();
        r.gcma_ok(&["restore", &id]);
        assert_eq!(held(&r, "v2"), old_object);
    });
}

#[test]
fn tags_named_like_the_backup_refs_do_not_confuse_the_listing() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        for name in ["old", "new", "tags", "tag-0", "x/old"] {
            r.git(&["tag", name, &old[1]]);
        }
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        let list = r.gcma_ok(&["restore"]);
        assert_eq!(list.lines().count(), 1, "{list}");
        let id = r.backup_id();
        let new_tip = r.git(&["rev-parse", "HEAD"]);
        assert!(list.contains(&new_tip), "{list}");
        r.gcma_ok(&["restore", &id]);
        for name in ["old", "new", "tags", "tag-0", "x/old"] {
            assert_eq!(held(&r, name), old[1], "{name}");
        }
        assert_eq!(r.git(&["rev-parse", "HEAD"]), old[2]);
    });
}

#[test]
fn restore_refuses_when_a_tag_moved_meanwhile_and_changes_nothing() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "v1", &old[0]]);
        r.git(&["tag", "v2", &old[1]]);
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        let id = r.backup_id();
        r.git(&["tag", "-f", "v1", "HEAD"]);
        let refs = r.refs();
        let o = r.gcma(&["restore", &id]);
        assert_eq!(Repo::code(&o), 4, "{}", stderr(&o));
        assert!(stderr(&o).contains("refs/tags/v1"), "{}", stderr(&o));
        assert_eq!(r.refs(), refs, "the branch and the other tag stay put");
    });
}

#[test]
fn a_forced_restore_leaves_a_tag_that_moved_meanwhile() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "v1", &old[0]]);
        r.git(&["tag", "v2", &old[1]]);
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        let id = r.backup_id();
        r.git(&["tag", "-f", "v1", "HEAD"]);
        let moved = held(&r, "v1");
        let o = r.gcma(&["restore", &id, "--force"]);
        assert!(o.status.success(), "{}", stderr(&o));
        assert!(stderr(&o).contains("refs/tags/v1"), "{}", stderr(&o));
        assert_eq!(held(&r, "v1"), moved);
        assert_eq!(held(&r, "v2"), old[1]);
        assert_eq!(r.git(&["rev-parse", "HEAD"]), old[2]);
    });
}

#[test]
fn prune_forgets_the_tag_records_too() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["tag", "-a", "-m", "release", "v2", &old[1]]);
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        assert!(r.refs().contains("/tags "), "the backup records the tags");
        let id = r.backup_id();
        r.gcma_ok(&["restore", &id, "--prune"]);
        assert_eq!(r.git(&["for-each-ref", "refs/gcma/backup"]), "");
    });
}

#[test]
fn notes_are_copied_to_the_new_commits_and_the_old_ones_stay() {
    on_both_backends(|r| {
        let old = three_to_rewrite(&r);
        r.git(&["notes", "add", "-m", "reviewed", &old[1]]);
        r.git(&["notes", "--ref", "extra", "add", "-m", "other", &old[2]]);
        r.gcma_ok(&["apply", "--retag", "--from", "root"]);
        let rows = r.log();
        assert_eq!(r.git(&["notes", "show", &rows[1].oid]), "reviewed");
        assert_eq!(
            r.git(&["notes", "--ref", "extra", "show", &rows[2].oid]),
            "other"
        );
        assert_eq!(r.git(&["notes", "show", &old[1]]), "reviewed");
    });
}
