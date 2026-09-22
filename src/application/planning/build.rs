//! The planning use case: decide which commits are rewritten and what they become.

use std::collections::{HashMap, HashSet};

use super::entries::build_entries;
use super::preconditions::check_preconditions;
use super::range::resolve;
use super::timing::{Schedule, load_external_parents, window_for};
use super::types::{Built, PlanOptions};
use crate::application::ports::Repository;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{Commit, SIGNATURE_HEADERS};
use crate::domain::history::conform::{self, Ctx};
use crate::domain::history::linearize::linearize;
use crate::domain::history::plan::{PLAN_VERSION, Plan};
use crate::domain::settings::{Config, Signing};

pub fn build_plan(repo: &dyn Repository, cfg: &Config, opts: &PlanOptions) -> Result<Built> {
    check_preconditions(repo, opts.strict)?;
    let range = resolve(repo, opts)?;
    let order = &range.order;
    let empty = || Built {
        plan: Plan {
            version: PLAN_VERSION,
            branch_ref: range.branch_ref.clone(),
            tip_oid: range.tip.clone(),
            signing: cfg.signing,
            entries: Vec::new(),
        },
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
    let now = opts.now.unwrap_or_else(|| chrono::Utc::now().timestamp());
    let window = window_for(cfg, now)?;
    if window.is_some() {
        load_external_parents(repo, &mut commits, &range_set)?;
    }
    let times: HashMap<String, i64> = commits
        .iter()
        .map(|(o, c)| (o.clone(), c.committer.time))
        .collect();
    let ctx = Ctx {
        cfg,
        window: window.as_ref().map(|(w, _)| w),
        times: &times,
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
    let new_times = match &window {
        Some((w, to)) => Schedule {
            cfg,
            window: w,
            to: *to,
            base: range.base.as_deref(),
            commits: &commits,
            times: &times,
        }
        .instants(&linear)?,
        None => Vec::new(),
    };
    let entries = build_entries(
        cfg,
        window.as_ref().map(|(w, _)| w),
        &new_times,
        &linear,
        &commits,
    )?;
    let warnings = rewrite_warnings(repo, cfg, &linear, &commits)?;

    Ok(Built {
        plan: Plan {
            version: PLAN_VERSION,
            branch_ref: range.branch_ref.clone(),
            tip_oid: range.tip.clone(),
            signing: cfg.signing,
            entries,
        },
        range_len: order.len(),
        frozen: frozen.len(),
        warnings,
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
                commits[*o].extra.iter().any(|h| {
                    !SIGNATURE_HEADERS.contains(&h.key.as_str()) && h.key != "gpgsig-sha256"
                })
            })
            .count();
        if lossy > 0 {
            warnings.push(format!("{lossy} commit(s) carry extra headers (e.g. encoding) that `signing: resign` cannot preserve"));
        }
    }
    Ok(warnings)
}
