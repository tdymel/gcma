//! The Plan: the seam between planning stages and `apply`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};

use crate::conform::{self, Ctx};
use crate::domain::error::{Error, Result};
use crate::domain::scheduling::{Window, derive_seed, rng_from_seed, schedule};
use crate::domain::settings::{Config, Signing, parse_days, parse_hours};
use crate::domain::text::messages;
use crate::git::{Commit, Git, RawIdent};

pub const PLAN_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PIdent {
    pub name: String,
    pub email: String,
    pub time: i64,
    /// UTC offset in minutes.
    pub tz: i32,
}

impl PIdent {
    pub fn to_raw(&self) -> RawIdent {
        RawIdent {
            name: self.name.clone().into_bytes(),
            email: self.email.clone().into_bytes(),
            time: self.time,
            tz: self.tz,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Parent {
    /// Index into `entries` (a rewritten commit).
    In(usize),
    /// A commit that keeps its OID (frozen, or outside the range).
    Base(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub old_oid: String,
    pub parents: Vec<Parent>,
    pub author: PIdent,
    pub committer: PIdent,
    pub message_b64: String,
}

impl Entry {
    pub fn message(&self) -> Result<Vec<u8>> {
        Ok(B64.decode(&self.message_b64)?)
    }

    pub fn set_message(&mut self, m: &[u8]) {
        self.message_b64 = B64.encode(m);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub branch_ref: String,
    pub tip_oid: String,
    pub signing: Signing,
    /// Suffix commits only, in write order (parents before children).
    pub entries: Vec<Entry>,
}

impl Plan {
    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Plan> {
        let data = std::fs::read(path)
            .map_err(|e| Error::Usage(format!("cannot read plan {}: {e}", path.display())))?;
        let p: Plan = serde_json::from_slice(&data)?;
        if p.version != PLAN_VERSION {
            return Err(Error::Usage(format!(
                "unsupported plan version {}",
                p.version
            )));
        }
        for (i, e) in p.entries.iter().enumerate() {
            for par in &e.parents {
                if let Parent::In(j) = par
                    && *j >= i
                {
                    return Err(Error::Usage(format!(
                        "plan entry {i} has a forward parent reference"
                    )));
                }
            }
        }
        Ok(p)
    }

    pub fn branch_name(&self) -> &str {
        self.branch_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&self.branch_ref)
    }
}

/// An explicit range (used by the pre-push hook): commits reachable from `tip` but not from `exclude`.
#[derive(Debug, Clone)]
pub struct RangeSpec {
    pub tip: String,
    pub branch_ref: String,
    /// Extra `rev-list` arguments, e.g. `["^<sha>"]` or `["--not", "--remotes=origin"]`.
    pub exclude: Vec<String>,
    pub base: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PlanOptions {
    pub from_rev: Option<String>,
    pub rewrite_pushed: bool,
    /// Treat every commit in the range as nonconforming (rewrite everything).
    pub all: bool,
    pub range: Option<RangeSpec>,
    /// Override "now" (tests).
    pub now: Option<i64>,
    /// Refuse on dirty index / operations in progress (apply and plan); the hook verifier relaxes this.
    pub strict: bool,
}

#[derive(Debug)]
pub struct Built {
    pub plan: Plan,
    pub range_len: usize,
    pub frozen: usize,
    pub warnings: Vec<String>,
}

pub fn check_preconditions(git: &Git, strict: bool) -> Result<()> {
    let refuse = |m: &str| Err(Error::Precondition(m.to_string()));
    if git.is_shallow()? {
        return refuse("shallow clones are not supported (parents are missing)");
    }
    if git.has_replace_refs()? || git.has_grafts()? {
        return refuse("replace refs / grafts are present; refusing to rewrite");
    }
    if strict {
        if let Some(op) = git.operation_in_progress()? {
            return Err(Error::Precondition(format!("a {op} is in progress")));
        }
        if git.index_dirty()? {
            return refuse("the index has staged changes; commit or stash them first");
        }
    }
    Ok(())
}

/// Deterministic parents-first order of the suffix, independent of dates:
/// ready commits are taken by (first-parent depth, oid).
fn linearize(suffix: &[String], commits: &HashMap<String, Commit>) -> Vec<String> {
    let set: HashSet<&String> = suffix.iter().collect();
    // First-parent depth within the suffix, computed independently of the input order.
    let mut depth: HashMap<&String, usize> = HashMap::new();
    for oid in suffix {
        let mut chain: Vec<&String> = Vec::new();
        let mut cur = oid;
        let mut base = 0;
        loop {
            if let Some(d) = depth.get(cur) {
                base = *d + 1;
                break;
            }
            chain.push(cur);
            match commits[cur].parents.first() {
                Some(p) if set.contains(p) => cur = p,
                _ => break,
            }
        }
        for (k, c) in chain.iter().rev().enumerate() {
            depth.insert(c, base + k);
        }
    }
    let mut indeg: HashMap<&String, usize> = HashMap::new();
    let mut children: HashMap<&String, Vec<&String>> = HashMap::new();
    for oid in suffix {
        let ps: BTreeSet<&String> = commits[oid]
            .parents
            .iter()
            .filter(|p| set.contains(p))
            .collect();
        indeg.insert(oid, ps.len());
        for p in ps {
            children.entry(p).or_default().push(oid);
        }
    }
    let mut ready: BTreeSet<(usize, &String)> = suffix
        .iter()
        .filter(|o| indeg[o] == 0)
        .map(|o| (depth[o], o))
        .collect();
    let mut out = Vec::with_capacity(suffix.len());
    while let Some(&(d, o)) = ready.iter().next() {
        ready.remove(&(d, o));
        out.push(o.clone());
        if let Some(ch) = children.get(o) {
            for c in ch {
                let n = indeg.get_mut(c).unwrap();
                *n -= 1;
                if *n == 0 {
                    ready.insert((depth[c], c));
                }
            }
        }
    }
    out
}

fn utf8(b: &[u8], what: &str, oid: &str) -> Result<String> {
    String::from_utf8(b.to_vec()).map_err(|_| {
        Error::Precondition(format!(
            "commit {oid}: non-UTF-8 {what} in a commit that must be rewritten"
        ))
    })
}

pub fn build_plan(git: &Git, cfg: &Config, opts: &PlanOptions) -> Result<Built> {
    check_preconditions(git, opts.strict)?;

    let (branch_ref, tip, explicit_base, mut range_args) = if let Some(r) = &opts.range {
        (
            r.branch_ref.clone(),
            r.tip.clone(),
            r.base.clone(),
            r.exclude.clone(),
        )
    } else {
        let branch_ref = git
            .current_branch_ref()?
            .ok_or_else(|| Error::Precondition("detached HEAD; check out a branch first".into()))?;
        let tip = git
            .rev_parse(&branch_ref)?
            .ok_or_else(|| Error::Usage("the current branch has no commits".into()))?;
        (branch_ref, tip, None, Vec::new())
    };
    let upstream_oid = if opts.range.is_some() {
        None
    } else {
        git.upstream_oid(&branch_ref)?
    };

    let mut base_oid = explicit_base;
    if opts.range.is_none() {
        base_oid = match (&opts.from_rev, &upstream_oid) {
            (Some(f), _) if f == "root" => None,
            (Some(f), _) => {
                let b = git
                    .rev_parse(f)?
                    .ok_or_else(|| Error::Usage(format!("--from: cannot resolve {f:?}")))?;
                if !git.is_ancestor(&b, &tip)? {
                    return Err(Error::Usage(format!(
                        "--from {f} is not an ancestor of the branch tip"
                    )));
                }
                Some(b)
            }
            (None, Some(up)) => git.merge_base(&tip, up)?,
            (None, None) => {
                return Err(Error::Usage(
                    "the branch has no upstream: pass --from <rev> (exclusive) or --from root"
                        .into(),
                ));
            }
        };
        if let Some(b) = &base_oid {
            range_args.push(format!("^{b}"));
        }
    }

    // The range, parents first.
    let mut args: Vec<&str> = vec!["--topo-order", "--reverse", &tip];
    args.extend(range_args.iter().map(|s| s.as_str()));
    let listed = git.rev_list_parents(&args)?;
    let order: Vec<String> = listed.iter().map(|(o, _)| o.clone()).collect();
    let empty = |warnings| Built {
        plan: Plan {
            version: PLAN_VERSION,
            branch_ref: branch_ref.clone(),
            tip_oid: tip.clone(),
            signing: cfg.signing,
            entries: Vec::new(),
        },
        range_len: order.len(),
        frozen: order.len(),
        warnings,
    };
    if order.is_empty() {
        return Ok(empty(Vec::new()));
    }

    let range_set: HashSet<&String> = order.iter().collect();
    let mut commits: HashMap<String, Commit> = git
        .read_commits(&order)?
        .into_iter()
        .map(|c| (c.oid.clone(), c))
        .collect();

    // Schedule mode: the Window, and the committer times of external parents.
    let now = opts.now.unwrap_or_else(|| chrono::Utc::now().timestamp());
    let mut window = None;
    let mut resolved_to = None;
    if let Some(s) = &cfg.schedule {
        let to = cfg.resolve_to(now)?;
        let (days, hours) = (parse_days(&s.days)?, parse_hours(&s.hours)?);
        window = Some(Window::build(cfg.tz()?, &days, hours, cfg.from_utc()?, to));
        resolved_to = Some(to);
        let external: Vec<String> = commits
            .values()
            .flat_map(|c| c.parents.iter())
            .filter(|p| !range_set.contains(p))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for c in git.read_commits(&external)? {
            commits.insert(c.oid.clone(), c);
        }
    }
    let times: HashMap<String, i64> = commits
        .iter()
        .map(|(o, c)| (o.clone(), c.committer.time))
        .collect();
    let ctx = Ctx {
        cfg,
        window: window.as_ref(),
        times: &times,
    };

    let frozen = if opts.all {
        HashSet::new()
    } else {
        conform::frozen_set(&order, &commits, &ctx)
    };
    let suffix: Vec<String> = order
        .iter()
        .filter(|o| !frozen.contains(*o))
        .cloned()
        .collect();
    if suffix.is_empty() {
        return Ok(empty(Vec::new()));
    }

    // Pushed commits in the suffix need an explicit flag.
    if let Some(up) = &upstream_oid {
        let unpushed = git.unpushed_among(&suffix, up)?;
        let pushed: Vec<&String> = suffix.iter().filter(|o| !unpushed.contains(*o)).collect();
        if !pushed.is_empty() && !opts.rewrite_pushed {
            return Err(Error::Pushed(format!(
                "{} commit(s) to rewrite are already on the upstream (e.g. {}); pass --rewrite-pushed to proceed",
                pushed.len(),
                &pushed[0][..pushed[0].len().min(10)]
            )));
        }
    }

    let linear = linearize(&suffix, &commits);
    let suffix_set: HashSet<&String> = linear.iter().collect();

    // Floor and scheduled times.
    let mut new_times: Vec<i64> = Vec::new();
    if let (Some(w), Some(s)) = (&window, &cfg.schedule) {
        let from = cfg.from_utc()?;
        let outside: i64 = linear
            .iter()
            .flat_map(|o| commits[o].parents.iter())
            .filter(|p| !suffix_set.contains(p))
            .filter_map(|p| times.get(p).copied())
            .max()
            .unwrap_or(from)
            .max(from);
        let to = resolved_to.unwrap();
        if to <= outside {
            return Err(Error::Precondition(format!(
                "`to` ({to}) is not after the floor ({outside}, the latest commit kept as-is); nothing can be scheduled"
            )));
        }
        let seed = derive_seed(&[
            s.seed.to_string().as_bytes(),
            base_oid.as_deref().unwrap_or("").as_bytes(),
        ]);
        new_times = schedule(
            linear.len(),
            w,
            outside,
            s.distribution,
            &mut rng_from_seed(seed),
        )?;
    }

    let index: HashMap<&String, usize> = linear.iter().enumerate().map(|(i, o)| (o, i)).collect();
    let mut entries = Vec::with_capacity(linear.len());
    for (i, oid) in linear.iter().enumerate() {
        let c = &commits[oid];
        let mapped = |id: &RawIdent| -> Result<PIdent> {
            let name = utf8(&id.name, "identity name", oid)?;
            let email = utf8(&id.email, "identity email", oid)?;
            let (name, email) = cfg.map_identity(&name, &email).unwrap_or((name, email));
            let (time, tz) = match &window {
                Some(w) => (new_times[i], w.tz_offset_minutes(new_times[i])),
                None => (id.time, id.tz),
            };
            Ok(PIdent {
                name,
                email,
                time,
                tz,
            })
        };
        let parents = c
            .parents
            .iter()
            .map(|p| match index.get(p) {
                Some(j) => Parent::In(*j),
                None => Parent::Base(p.clone()),
            })
            .collect();
        let mut e = Entry {
            old_oid: oid.clone(),
            parents,
            author: mapped(&c.author)?,
            committer: mapped(&c.committer)?,
            message_b64: String::new(),
        };
        e.set_message(&messages::strip_trailers(
            &c.message,
            &cfg.messages.strip_trailers,
        ));
        entries.push(e);
    }

    let mut warnings = Vec::new();
    let labels = git.labels_pointing_at(&linear.iter().cloned().collect())?;
    if !labels.is_empty() {
        warnings.push(format!(
            "tags/notes point at commits that will be rewritten and will keep pointing at the old ones: {}",
            labels.join(", ")
        ));
    }
    if cfg.signing == Signing::Resign {
        let lossy = linear
            .iter()
            .filter(|o| {
                commits[*o]
                    .extra
                    .iter()
                    .any(|h| h.key != "gpgsig" && h.key != "mergetag" && h.key != "gpgsig-sha256")
            })
            .count();
        if lossy > 0 {
            warnings.push(format!("{lossy} commit(s) carry extra headers (e.g. encoding) that `signing: resign` cannot preserve"));
        }
    }

    Ok(Built {
        plan: Plan {
            version: PLAN_VERSION,
            branch_ref,
            tip_oid: tip,
            signing: cfg.signing,
            entries,
        },
        range_len: order.len(),
        frozen: frozen.len(),
        warnings,
    })
}

/// Human-readable plan table.
pub fn render(plan: &Plan, old: &[Commit]) -> String {
    use chrono::TimeZone;
    let fmt = |t: i64, off: i32| -> String {
        match chrono::FixedOffset::east_opt(off * 60).and_then(|o| o.timestamp_opt(t, 0).single()) {
            Some(d) => d.format("%Y-%m-%d %H:%M %z").to_string(),
            None => t.to_string(),
        }
    };
    let mut s = String::new();
    for (e, o) in plan.entries.iter().zip(old) {
        let title = messages::title(&e.message().unwrap_or_default());
        let who_old = format!(
            "{} <{}>",
            String::from_utf8_lossy(&o.author.name),
            String::from_utf8_lossy(&o.author.email)
        );
        let who_new = format!("{} <{}>", e.author.name, e.author.email);
        s.push_str(&format!(
            "{}  {} -> {}  {}{}\n",
            &e.old_oid[..e.old_oid.len().min(8)],
            fmt(o.committer.time, o.committer.tz),
            fmt(e.committer.time, e.committer.tz),
            if who_old == who_new {
                who_new
            } else {
                format!("{who_old} -> {who_new}")
            },
            if title.is_empty() {
                String::new()
            } else {
                format!("  \"{title}\"")
            }
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(oid: &str, parents: &[&str]) -> Commit {
        let id = RawIdent {
            name: b"n".to_vec(),
            email: b"e".to_vec(),
            time: 0,
            tz: 0,
        };
        Commit {
            oid: oid.into(),
            tree: "t".into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            author: id.clone(),
            committer: id,
            extra: Vec::new(),
            message: Vec::new(),
        }
    }

    #[test]
    fn linearize_is_parents_first_and_deterministic() {
        // a <- b <- d (merge of b and c) ; c <- a
        let commits: HashMap<String, Commit> = [
            c("a", &[]),
            c("b", &["a"]),
            c("c", &["a"]),
            c("d", &["b", "c"]),
        ]
        .into_iter()
        .map(|x| (x.oid.clone(), x))
        .collect();
        let s: Vec<String> = ["d", "c", "b", "a"].iter().map(|x| x.to_string()).collect();
        let lin = linearize(&s, &commits);
        let pos = |o: &str| lin.iter().position(|x| x == o).unwrap();
        assert!(
            pos("a") < pos("b")
                && pos("a") < pos("c")
                && pos("b") < pos("d")
                && pos("c") < pos("d")
        );
        assert_eq!(lin, linearize(&s, &commits));
        let s2: Vec<String> = ["a", "b", "c", "d"].iter().map(|x| x.to_string()).collect();
        assert_eq!(lin, linearize(&s2, &commits), "independent of input order");
    }

    #[test]
    fn plan_roundtrip_and_forward_reference_rejected() {
        let id = PIdent {
            name: "N".into(),
            email: "e@x".into(),
            time: 1,
            tz: 60,
        };
        let mut e = Entry {
            old_oid: "o".into(),
            parents: vec![Parent::Base("b".into())],
            author: id.clone(),
            committer: id,
            message_b64: String::new(),
        };
        e.set_message(b"hi \xff\n");
        let mut p = Plan {
            version: PLAN_VERSION,
            branch_ref: "refs/heads/main".into(),
            tip_oid: "o".into(),
            signing: Signing::Strip,
            entries: vec![e.clone()],
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.json");
        p.save(&path).unwrap();
        let q = Plan::load(&path).unwrap();
        assert_eq!(q.entries[0].message().unwrap(), b"hi \xff\n");
        assert_eq!(q.branch_name(), "main");
        p.entries[0].parents = vec![Parent::In(0)];
        p.save(&path).unwrap();
        assert!(Plan::load(&path).is_err());
    }
}
