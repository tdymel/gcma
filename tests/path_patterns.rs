//! What each kind of gitignore pattern removes from history, checked against the files that
//! remain in the rewritten commit.

mod common;

use common::*;

fn remaining(r: &Repo) -> Vec<String> {
    let mut files: Vec<String> = r
        .git(&["ls-tree", "-r", "--name-only", "-z", "HEAD"])
        .split('\0')
        .filter(|f| !f.is_empty() && *f != ".gitignore")
        .map(String::from)
        .collect();
    files.sort();
    files
}

fn quote(p: &str) -> String {
    format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\""))
}

/// One commit with `files` (plus `keep.txt`, so it is never dropped as "only excluded").
fn case(patterns: &[&str], files: &[&str], expect: &[&str]) {
    let r = Repo::new();
    let mut all: Vec<(&str, &str)> = files.iter().map(|f| (*f, "x\n")).collect();
    all.push(("keep.txt", "k\n"));
    r.commit_files(&all, "one", 1_600_000_000);
    let list: Vec<String> = patterns.iter().map(|p| quote(p)).collect();
    r.config(&format!(
        "version: 1\npaths:\n  exclude: [{}]\n",
        list.join(", ")
    ));
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(
        o.status.success(),
        "{patterns:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let mut want: Vec<String> = expect.iter().map(|s| s.to_string()).collect();
    want.push("keep.txt".into());
    want.sort();
    assert_eq!(remaining(&r), want, "patterns {patterns:?}");
    // Every pattern is in .gitignore, and the working copy still has all the files.
    let ignore = r.git(&["show", "HEAD:.gitignore"]);
    for p in patterns {
        assert!(
            ignore.lines().any(|l| l == *p),
            "{p:?} missing from {ignore:?}"
        );
    }
    for f in files {
        assert!(r.path().join(f).exists(), "{f} must stay in the project");
    }
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do"),
        "{patterns:?}: second run"
    );
    r.fsck();
}

#[test]
fn a_bare_name_matches_at_any_depth() {
    case(
        &["CLAUDE.md"],
        &[
            "CLAUDE.md",
            "docs/CLAUDE.md",
            "docs/deep/er/CLAUDE.md",
            "claude.md",
            "CLAUDE.md.bak",
        ],
        &["claude.md", "CLAUDE.md.bak"],
    );
}

#[test]
fn a_leading_slash_anchors_to_the_root() {
    case(
        &["/CLAUDE.md"],
        &["CLAUDE.md", "docs/CLAUDE.md"],
        &["docs/CLAUDE.md"],
    );
    case(
        &["/build"],
        &["build/x.o", "src/build/y.o"],
        &["src/build/y.o"],
    );
}

#[test]
fn a_slash_in_the_middle_anchors_too() {
    case(
        &["memory/*"],
        &[
            "memory/a.md",
            "memory/sub/b.md",
            "src/memory/c.md",
            "memory.md",
        ],
        &["src/memory/c.md", "memory.md"],
    );
}

#[test]
fn a_trailing_slash_matches_only_directories() {
    case(
        &["build/"],
        &["build", "x/build/y.o", "z/build.txt"],
        &["build", "z/build.txt"],
    );
}

#[test]
fn star_stops_at_a_slash_and_double_star_does_not() {
    case(
        &["a/*.tmp"],
        &["a/x.tmp", "a/b/y.tmp", "c/a/z.tmp"],
        &["a/b/y.tmp", "c/a/z.tmp"],
    );
    case(
        &["a/**/*.tmp"],
        &["a/x.tmp", "a/b/y.tmp", "a/b/c/z.tmp", "other/q.tmp"],
        &["other/q.tmp"],
    );
    case(
        &["**/node_modules"],
        &[
            "node_modules/p/i.js",
            "web/node_modules/p/i.js",
            "web/src/app.js",
        ],
        &["web/src/app.js"],
    );
    case(
        &["logs/**"],
        &["logs/a.log", "logs/b/c.log", "other/logs/d.log"],
        &["other/logs/d.log"],
    );
}

#[test]
fn question_mark_and_character_classes() {
    case(
        &["?.txt", "[xy].md"],
        &["a.txt", "ab.txt", "x.md", "z.md", "dir/b.txt"],
        &["ab.txt", "z.md"],
    );
}

#[test]
fn a_negation_brings_a_file_back_unless_its_directory_is_excluded() {
    case(
        &["*.log", "!keep.log"],
        &["a.log", "keep.log", "d/b.log", "d/keep.log"],
        &["keep.log", "d/keep.log"],
    );
    // gitignore rule: a file cannot be re-included when a parent directory is excluded.
    case(
        &["logs/", "!logs/keep.txt"],
        &["logs/keep.txt", "logs/other.txt"],
        &[],
    );
}

#[test]
fn the_last_matching_pattern_wins() {
    // `*.txt` comes last and also removes keep.txt: nothing but the added .gitignore would remain.
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n"), ("b.md", "b\n")], "one", 1_600_000_000);
    r.config("version: 1\npaths:\n  exclude: [\"!a.txt\", \"*.txt\"]\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(
        remaining(&r),
        ["b.md"],
        "the later *.txt overrides the earlier negation"
    );
    // Reversed order: the negation wins for a.txt.
    let r = Repo::new();
    r.commit_files(
        &[("a.txt", "a\n"), ("b.txt", "b\n"), ("c.md", "c\n")],
        "one",
        1_600_000_000,
    );
    r.config("version: 1\npaths:\n  exclude: [\"*.txt\", \"!a.txt\"]\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(
        remaining(&r),
        ["a.txt", "c.md"],
        "the later negation wins for a.txt"
    );
}

#[test]
fn escaped_hash_and_bang_match_literally() {
    case(
        &["\\#notes.txt", "\\!bang.txt"],
        &["#notes.txt", "!bang.txt", "notes.txt"],
        &["notes.txt"],
    );
}

#[test]
fn names_with_spaces_and_unicode_are_handled() {
    case(
        &["my secret.txt", "café/", "*.ü"],
        &["my secret.txt", "café/ü.txt", "a.ü", "plain.txt"],
        &["plain.txt"],
    );
}

#[test]
fn symlinks_and_submodule_paths_are_excluded_like_files_and_directories() {
    let r = Repo::new();
    r.commit_files(
        &[("keep.txt", "k\n"), ("real.txt", "r\n")],
        "one",
        1_600_000_000,
    );
    std::os::unix::fs::symlink("real.txt", r.path().join("secret-link")).unwrap();
    r.git(&["add", "secret-link"]);
    let fake_commit = "1".repeat(40);
    r.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{fake_commit},vendor/lib"),
    ]);
    r.git(&["commit", "-q", "-m", "link and submodule"]);
    r.config("version: 1\npaths:\n  exclude: [\"secret-link\", \"vendor/\"]\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(remaining(&r), ["keep.txt", "real.txt"]);
    r.fsck();
}

#[test]
fn deeply_nested_files_vanish_with_their_empty_directories() {
    let r = Repo::new();
    r.commit_files(
        &[
            ("keep.txt", "k\n"),
            ("a/b/c/d/e/secret.txt", "s\n"),
            ("a/b/other.txt", "o\n"),
        ],
        "one",
        1_600_000_000,
    );
    r.config("version: 1\npaths:\n  exclude: [\"secret.txt\"]\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(remaining(&r), ["a/b/other.txt", "keep.txt"]);
    assert!(
        !r.git(&["ls-tree", "-r", "-d", "--name-only", "HEAD"])
            .contains("a/b/c"),
        "no empty directory is left behind"
    );
}

#[test]
fn unusable_patterns_are_config_errors() {
    for (pattern, why) in [
        ("\\", "dangling"),
        ("# comment", "paths.exclude"),
        ("", "paths.exclude"),
        (" padded", "paths.exclude"),
        ("trailing ", "paths.exclude"),
    ] {
        let r = Repo::new();
        r.linear(2, 1_600_000_000);
        r.config(&format!(
            "version: 1\npaths:\n  exclude: [{}]\n",
            quote(pattern)
        ));
        let o = r.gcma(&["plan", "--from", "root"]);
        assert_eq!(
            Repo::code(&o),
            2,
            "{pattern:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(
            String::from_utf8_lossy(&o.stderr).contains(why),
            "{pattern:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}
