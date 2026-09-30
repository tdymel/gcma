//! The planning use case: decide which commits are rewritten and what they become.

use std::collections::{HashMap, HashSet};

use super::entries::{build_entries, dropped_parents};
use super::pathplan::{self, PathOutcome};
use super::range::{RangeInfo, resolve};
use super::timing::{Schedule, load_external_parents, window_for};
use super::types::{Built, PlanOptions};
use super::warnings::rewrite_warnings;
use crate::application::pathrules::TreeRewriter;
use crate::application::ports::Repository;
use crate::application::preconditions::{check_preconditions, refuse_pushed};
use crate::application::retag;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::Commit;
use crate::domain::history::conform::{self, Ctx};
use crate::domain::history::linearize::linearize;
use crate::domain::history::parents::resolve_parents;
use crate::domain::history::plan::{Parent, PathRules, Plan};
use crate::domain::paths::PathFilter;
use crate::domain::scheduling::Window;
use crate::domain::settings::Config;

/// What planning reads once and every later step needs.
struct Loaded<'a> {
    /// The commits of the range, plus (with a schedule or path rules) their outside parents.
    commits: HashMap<String, Commit>,
    /// Schedule mode: the window and the resolved end of it.
    window: Option<(Window, i64)>,
    rewriter: Option<TreeRewriter<'a>>,
    /// Range commits whose tree contains an excluded path.
    excluded: HashSet<String>,
    /// Committer time of every loaded commit.
    times: HashMap<String, i64>,
}

impl Loaded<'_> {
    fn window(&self) -> Option<&Window> {
        self.window.as_ref().map(|(w, _)| w)
    }
}

pub fn build_plan(repo: &dyn Repository, cfg: &Config, opts: &PlanOptions) -> Result<Built> {
    check_preconditions(repo, opts.strict)?;
    let range = resolve(repo, opts)?;
    if range.order.is_empty() {
        return Ok(nothing_to_do(cfg, &range));
    }
    let filter = cfg.path_filter()?;
    let loaded = load(repo, cfg, opts.now, &range, filter.as_ref())?;
    let suffix = choose_suffix(repo, cfg, opts, &range, &loaded)?;
    if suffix.is_empty() {
        return Ok(nothing_to_do(cfg, &range));
    }
    let (plan, warnings) = assemble(
        repo,
        cfg,
        &range,
        &loaded,
        filter.as_ref(),
        &suffix,
        opts.retag,
    )?;
    let retag = opts
        .retag
        .then(|| retag::preview(repo, &plan))
        .transpose()?;
    Ok(Built {
        plan,
        range_len: range.order.len(),
        frozen: range.order.len() - suffix.len(),
        warnings,
        retag,
    })
}

/// The result when every commit of the range is left alone.
fn nothing_to_do(cfg: &Config, range: &RangeInfo) -> Built {
    Built {
        plan: Plan::new(
            range.branch_ref.clone(),
            range.tip.clone(),
            cfg.signing,
            Vec::new(),
        ),
        range_len: range.order.len(),
        frozen: range.order.len(),
        warnings: Vec::new(),
        retag: None,
    }
}

/// Reads the commits of the range, the schedule window, and which commits touch excluded paths.
fn load<'a>(
    repo: &'a dyn Repository,
    cfg: &Config,
    now: i64,
    range: &RangeInfo,
    filter: Option<&'a PathFilter>,
) -> Result<Loaded<'a>> {
    let order = &range.order;
    let mut commits: HashMap<String, Commit> = repo
        .read_commits(order)?
        .into_iter()
        .map(|c| (c.oid.clone(), c))
        .collect();
    let window = window_for(cfg, now)?;
    let rewriter = filter.map(|f| TreeRewriter::new(repo, f));
    // The committer times of parents outside the range matter to the schedule and to path rules.
    if window.is_some() || rewriter.is_some() {
        let range_set: HashSet<&String> = order.iter().collect();
        load_external_parents(repo, &mut commits, &range_set)?;
    }
    let mut excluded = HashSet::new();
    if let Some(rw) = &rewriter {
        for oid in order {
            if rw.has_excluded(&commits[oid].tree)? {
                excluded.insert(oid.clone());
            }
        }
    }
    let times = commits
        .iter()
        .map(|(o, c)| (o.clone(), c.committer.time))
        .collect();
    Ok(Loaded {
        commits,
        window,
        rewriter,
        excluded,
        times,
    })
}

/// The commits of the range that are rewritten (parents first): everything that is not frozen as
/// conforming. Pushed commits among them need an explicit flag.
fn choose_suffix(
    repo: &dyn Repository,
    cfg: &Config,
    opts: &PlanOptions,
    range: &RangeInfo,
    loaded: &Loaded,
) -> Result<Vec<String>> {
    let ctx = Ctx {
        cfg,
        window: loaded.window(),
        times: &loaded.times,
        excluded: loaded.rewriter.as_ref().map(|_| &loaded.excluded),
    };
    let frozen = if opts.all {
        HashSet::new()
    } else {
        conform::frozen_set(&range.order, &loaded.commits, &ctx)
    };
    let suffix: Vec<String> = range
        .order
        .iter()
        .filter(|o| !frozen.contains(*o))
        .cloned()
        .collect();
    if !suffix.is_empty() {
        refuse_pushed(
            repo,
            &suffix,
            range.upstream.as_deref(),
            opts.rewrite_pushed,
        )?;
    }
    Ok(suffix)
}

/// The plan for the (non-empty) suffix, and the warnings about rewriting it.
fn assemble(
    repo: &dyn Repository,
    cfg: &Config,
    range: &RangeInfo,
    loaded: &Loaded,
    filter: Option<&PathFilter>,
    suffix: &[String],
    retag: bool,
) -> Result<(Plan, Vec<String>)> {
    let commits = &loaded.commits;
    let linear = linearize(suffix, commits);
    let outcome = match &loaded.rewriter {
        Some(rw) => pathplan::apply_rules(rw, cfg, &linear, commits, &loaded.excluded, &range.tip)?,
        None => PathOutcome::untouched(&linear),
    };
    let new_tip = new_tip_when_dropped(&range.tip, &outcome, commits)?;
    let new_times = match &loaded.window {
        Some((w, to)) => Schedule {
            cfg,
            window: w,
            to: *to,
            base: range.base.as_deref(),
            commits,
            times: &loaded.times,
        }
        .instants(&linear, outcome.kept.len())?,
        None => Vec::new(),
    };
    let entries = build_entries(cfg, loaded.window(), &new_times, &outcome, commits)?;

    let mut plan = Plan::new(
        range.branch_ref.clone(),
        range.tip.clone(),
        cfg.signing,
        entries,
    );
    if let Some(f) = filter {
        plan.paths = Some(PathRules {
            exclude: f.patterns().to_vec(),
        });
        plan.dropped = outcome.dropped;
        plan.new_tip = new_tip;
    }
    let touched: Vec<String> = plan.touched_oids().cloned().collect();
    let warnings = rewrite_warnings(repo, cfg, &touched, commits, retag)?;
    Ok((plan, warnings))
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
