//! The planning use case: decide which commits are rewritten and what they become.

use std::collections::{HashMap, HashSet};

use super::entries::{build_entries, dropped_parents};
use super::pathplan::{self, PathOutcome};
use super::range::resolve;
use super::timing::{Schedule, load_external_parents, window_for};
use super::types::{Built, PlanOptions};
use crate::application::pathrules::TreeRewriter;
use crate::application::ports::Repository;
use crate::application::preconditions::check_preconditions;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::Commit;
use crate::domain::history::conform::{self, Ctx};
use crate::domain::history::linearize::linearize;
use crate::domain::history::parents::resolve_parents;
use crate::domain::history::plan::{Parent, PathRules, Plan};
use crate::domain::settings::{Config, Signing};

pub fn build_plan(repo: &dyn Repository, cfg: &Config, opts: &PlanOptions) -> Result<Built> {
    check_preconditions(repo, opts.strict)?;
    let range = resolve(repo, opts)?;
    let order = &range.order;
    let empty = || Built {
        plan: Plan::new(
            range.branch_ref.clone(),
            range.tip.clone(),
            cfg.signing,
            Vec::new(),
        ),
        range_len: order.len(),
        frozen: order.len(),
        warnings: Vec::new(),
    };
    if order.is_empty() {
        return Ok(empty());
    }

    let range_set: HashSet<&String> = order.iter().collect();
    let mut commits: HashMap<String, Commit> = repo
        .read_commits(order)?
        .into_iter()
        .map(|c| (c.oid.clone(), c))
        .collect();

    // Schedule mode: the window, and the committer times of parents outside the range.
    let now = opts.now;
    let window = window_for(cfg, now)?;
    let filter = cfg.path_filter()?;
    let rewriter = filter.as_ref().map(|f| TreeRewriter::new(repo, f));
    if window.is_some() || rewriter.is_some() {
        load_external_parents(repo, &mut commits, &range_set)?;
    }
    let excluded: HashSet<String> = match &rewriter {
        Some(rw) => {
            let mut set = HashSet::new();
            for oid in order {
                if rw.has_excluded(&commits[oid].tree)? {
                    set.insert(oid.clone());
                }
            }
            set
        }
        None => HashSet::new(),
    };
    let times: HashMap<String, i64> = commits
        .iter()
        .map(|(o, c)| (o.clone(), c.committer.time))
        .collect();
    let ctx = Ctx {
        cfg,
        window: window.as_ref().map(|(w, _)| w),
        times: &times,
        excluded: rewriter.as_ref().map(|_| &excluded),
    };

    let frozen = if opts.all {
        HashSet::new()
    } else {
        conform::frozen_set(order, &commits, &ctx)
    };
    let suffix: Vec<String> = order
        .iter()
        .filter(|o| !frozen.contains(*o))
        .cloned()
        .collect();
    if suffix.is_empty() {
        return Ok(empty());
    }
    refuse_pushed(repo, &suffix, range.upstream.as_deref(), opts)?;

    let linear = linearize(&suffix, &commits);
    let outcome = match &rewriter {
        Some(rw) => pathplan::apply_rules(rw, cfg, &linear, &commits, &excluded, &range.tip)?,
        None => PathOutcome::untouched(&linear),
    };
    let new_tip = new_tip_when_dropped(&range.tip, &outcome, &commits)?;
    let new_times = match &window {
        Some((w, to)) => Schedule {
            cfg,
            window: w,
            to: *to,
            base: range.base.as_deref(),
            commits: &commits,
            times: &times,
        }
        .instants(&linear, outcome.kept.len())?,
        None => Vec::new(),
    };
    let entries = build_entries(
        cfg,
        window.as_ref().map(|(w, _)| w),
        &new_times,
        &outcome,
        &commits,
    )?;
    let touched: Vec<String> = outcome
        .kept
        .iter()
        .chain(&outcome.dropped)
        .cloned()
        .collect();
    refuse_unsignable(cfg, &outcome.kept, &commits)?;
    let warnings = rewrite_warnings(repo, cfg, &touched, &commits)?;

    let mut plan = Plan::new(
        range.branch_ref.clone(),
        range.tip.clone(),
        cfg.signing,
        entries,
    );
    if let Some(f) = &filter {
        plan.paths = Some(PathRules {
            exclude: f.patterns().to_vec(),
        });
        plan.dropped = outcome.dropped;
        plan.new_tip = new_tip;
    }
    Ok(Built {
        plan,
        range_len: order.len(),
        frozen: frozen.len(),
        warnings,
    })
}

/// When the old tip itself is dropped, the branch moves to what its parent became.
fn new_tip_when_dropped(
    tip: &str,
    outcome: &PathOutcome,
    commits: &HashMap<String, Commit>,
) -> Result<Option<Parent>> {
    if !outcome.dropped.iter().any(|o| o == tip) {
        return Ok(None);
    }
    let index: HashMap<&str, usize> = outcome
        .kept
        .iter()
        .enumerate()
        .map(|(i, o)| (o.as_str(), i))
        .collect();
    let dropped = dropped_parents(outcome, commits);
    resolve_parents(&commits[tip].parents, &index, &dropped)
        .into_iter()
        .next()
        .map(Some)
        .ok_or_else(|| {
            Error::Precondition(
                "every commit of the branch only touches excluded paths; nothing would be left"
                    .into(),
            )
        })
}

/// Pushed commits in the suffix need an explicit flag.
fn refuse_pushed(
    repo: &dyn Repository,
    suffix: &[String],
    upstream: Option<&str>,
    opts: &PlanOptions,
) -> Result<()> {
    let Some(up) = upstream else { return Ok(()) };
    let unpushed = repo.unpushed_among(suffix, up)?;
    let pushed: Vec<&String> = suffix.iter().filter(|o| !unpushed.contains(*o)).collect();
    if !pushed.is_empty() && !opts.rewrite_pushed {
        return Err(Error::Pushed(format!(
            "{} commit(s) to rewrite are already on the upstream (e.g. {}); pass --rewrite-pushed to proceed",
            pushed.len(),
            &pushed[0][..pushed[0].len().min(10)]
        )));
    }
    Ok(())
}

/// `git commit-tree -S` recodes a message that is not valid UTF-8, so such a commit cannot be
/// re-signed byte for byte; better to say so now than to fail verification at apply time.
fn refuse_unsignable(
    cfg: &Config,
    linear: &[String],
    commits: &HashMap<String, Commit>,
) -> Result<()> {
    if cfg.signing != Signing::Resign {
        return Ok(());
    }
    let binary = linear
        .iter()
        .filter(|o| std::str::from_utf8(&commits[*o].message).is_err())
        .count();
    if binary > 0 {
        return Err(Error::Precondition(format!(
            "{binary} commit(s) have messages that are not valid UTF-8, which git recodes when it signs, \
             so `signing: resign` cannot keep them as they are; use `signing: strip` for this branch"
        )));
    }
    Ok(())
}

/// Non-fatal consequences of rewriting these commits.
fn rewrite_warnings(
    repo: &dyn Repository,
    cfg: &Config,
    linear: &[String],
    commits: &HashMap<String, Commit>,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let labels = repo.labels_pointing_at(&linear.iter().cloned().collect())?;
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
                    .any(|h| !h.is_invalidated_by_rewrite())
            })
            .count();
        if lossy > 0 {
            warnings.push(format!("{lossy} commit(s) carry extra headers (e.g. encoding) that `signing: resign` cannot preserve"));
        }
    }
    Ok(warnings)
}
