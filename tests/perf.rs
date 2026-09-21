//! Opt-in performance test comparing the object backends on a large generated history.
//!
//! ```text
//! cargo test --release --features gix --test perf -- --ignored --nocapture
//! GHMA_PERF_COMMITS=50000 cargo test --release --features gix --test perf -- --ignored --nocapture
//! ```
//!
//! Without `--features gix` only the `git` backend runs. With it, both run on identical copies of
//! the history and must produce the very same commits (same ids), proving the backends are
//! interchangeable. `GHMA_PERF_BUDGET_SECS` optionally fails the test when a backend's `apply`
//! exceeds the budget.

mod common;

use std::io::Write;
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::*;

const CFG: &str = "version: 1
from: 2024-01-01
to: 2026-06-30
timezone: Europe/Berlin
schedule: {days: [mon, tue, wed, thu, fri], hours: \"09:30-18:00\", distribution: bursty, seed: 3}
identity:
  - match: {email: me@home.org}
    set: {name: Jane Doe, email: jane@work.com}
messages: {strip_trailers: [Signed-off-by]}
";

/// A linear history of `n` commits (every 25th has a side branch merged back), via fast-import.
fn generate(r: &Repo, n: usize) {
    let mut s = Vec::new();
    let mut mark = 0usize;
    let mut prev: Option<usize> = None;
    let commit = |s: &mut Vec<u8>, mark: &mut usize, parents: &[usize], i: usize| -> usize {
        *mark += 1;
        let blob = *mark;
        let body = format!("content {i}\n");
        write!(s, "blob\nmark :{blob}\ndata {}\n{body}\n", body.len()).unwrap();
        *mark += 1;
        let m = *mark;
        let msg = format!("commit {i}\n\nSigned-off-by: Old Me <me@home.org>\n");
        write!(
            s,
            "commit refs/heads/main\nmark :{m}\nauthor Old Me <me@home.org> {t} +0000\ncommitter Old Me <me@home.org> {t} +0000\ndata {}\n{msg}\n",
            msg.len(),
            t = 1_500_000_000 + i as i64 * 600
        )
        .unwrap();
        if let Some((first, rest)) = parents.split_first() {
            writeln!(s, "from :{first}").unwrap();
            for p in rest {
                writeln!(s, "merge :{p}").unwrap();
            }
        }
        writeln!(s, "M 100644 :{blob} f{i}.txt\n").unwrap();
        m
    };
    let mut i = 0;
    while i < n {
        if i % 25 == 24 && i + 2 < n {
            // side commit whose parent is the current tip, then a merge of tip + side
            let side = commit(&mut s, &mut mark, &[prev.unwrap()], i);
            let tipc = commit(&mut s, &mut mark, &[prev.unwrap(), side], i + 1);
            prev = Some(tipc);
            i += 2;
        } else {
            let m = commit(&mut s, &mut mark, prev.as_slice(), i);
            prev = Some(m);
            i += 1;
        }
    }
    s.extend_from_slice(b"done\n");
    let mut child = r
        .cmd("git")
        .args(["fast-import", "--quiet", "--done"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&s).unwrap();
    assert!(child.wait().unwrap().success());
    r.git(&["reset", "-q", "--hard", "main"]);
}

struct Timing {
    backend: &'static str,
    plan: Duration,
    apply: Duration,
    rerun: Duration,
    export: Duration,
    restore: Duration,
    tip: String,
    commits: usize,
}

fn run_backend(backend: &'static str, src: &Repo, n: usize) -> Timing {
    let r = Repo::new();
    // Clone the object store only; the generated history is the same for every backend.
    r.git(&[
        "fetch",
        "-q",
        src.path().to_str().unwrap(),
        "main:refs/seed",
    ]);
    r.git(&["reset", "-q", "--hard", "refs/seed"]);
    r.git(&["update-ref", "-d", "refs/seed"]);
    r.config(CFG);
    let be = ["--backend", backend];
    let old_tip = r.git(&["rev-parse", "HEAD"]);
    let timed = |args: &[&str]| -> (Duration, String) {
        let t = Instant::now();
        let mut a = be.to_vec();
        a.extend_from_slice(args);
        let out = r.ghma_ok(&a);
        (t.elapsed(), out)
    };

    let (plan, out) = timed(&["plan", "--from", "root"]);
    assert!(out.contains("to rewrite"), "{out}");
    let (export, _) = timed(&["export", "--from", "root"]);
    let (apply, out) = timed(&["apply", "--from", "root"]);
    assert!(out.contains("Rewrote"), "{out}");
    let tip = r.git(&["rev-parse", "HEAD"]);
    let commits: usize = r.git(&["rev-list", "--count", "HEAD"]).parse().unwrap();
    assert_eq!(commits, n);
    let (rerun, out) = timed(&["apply", "--from", "root"]);
    assert!(
        out.contains("Nothing to do"),
        "rerun must be a no-op: {out}"
    );
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    r.fsck();
    // Content untouched: `git diff` of trees is empty and every tree id is preserved.
    r.git(&["diff", "--quiet", &old_tip, &tip]);
    let trees = |rev: &str| r.git(&["log", "--format=%T", rev]);
    let new_trees = trees(&tip);
    let old_trees = trees(&old_tip);
    assert_eq!(
        new_trees, old_trees,
        "every commit must keep its tree, in order"
    );
    let id = r.ghma_ok(&["restore"]);
    let id = id.split_whitespace().next().unwrap().to_string();
    let t = Instant::now();
    r.ghma_ok(&["restore", &id]);
    let restore = t.elapsed();
    assert_eq!(r.git(&["rev-parse", "HEAD"]), old_tip);
    Timing {
        backend,
        plan,
        apply,
        rerun,
        export,
        restore,
        tip,
        commits,
    }
}

#[test]
#[ignore = "performance test; run with --release --ignored --nocapture"]
fn backends_perf() {
    let n: usize = std::env::var("GHMA_PERF_COMMITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000);
    let src = Repo::new();
    let t = Instant::now();
    generate(&src, n);
    eprintln!("generated {n} commits in {:?}", t.elapsed());

    let mut backends = vec!["git"];
    if cfg!(feature = "gix") {
        backends.push("gix");
    } else {
        eprintln!("(built without --features gix: only the git backend is measured)");
    }
    let results: Vec<Timing> = backends.iter().map(|b| run_backend(b, &src, n)).collect();

    eprintln!(
        "\n{:<6} {:>8} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "", "commits", "plan", "export", "apply", "rerun", "restore"
    );
    for t in &results {
        eprintln!(
            "{:<6} {:>8} {:>9.3?} {:>9.3?} {:>9.3?} {:>9.3?} {:>9.3?}",
            t.backend, t.commits, t.plan, t.export, t.apply, t.rerun, t.restore
        );
    }
    if results.len() == 2 {
        eprintln!(
            "apply speed-up gix vs git: {:.1}x",
            results[0].apply.as_secs_f64() / results[1].apply.as_secs_f64()
        );
    }
    for t in &results[1..] {
        assert_eq!(
            t.tip, results[0].tip,
            "backend {} produced different commits than {}",
            t.backend, results[0].backend
        );
    }
    if let Some(budget) = std::env::var("GHMA_PERF_BUDGET_SECS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        for t in &results {
            assert!(
                t.apply.as_secs_f64() <= budget,
                "{} apply took {:?}, over the {budget}s budget",
                t.backend,
                t.apply
            );
        }
    }
}
