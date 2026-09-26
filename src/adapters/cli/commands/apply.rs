//! `gcma apply`: rewrite the branch from a fresh or a saved plan.

use std::path::PathBuf;

use crate::adapters::cli_support::args::RangeArgs;
use crate::adapters::cli_support::report::print_apply;
use crate::adapters::cli_support::session::{Session, now, plan_options, warn_if_inert};
use crate::adapters::plan_file;
use crate::application::planning::build_plan;
use crate::application::rewrite::{apply, ensure_plan_matches_config};
use crate::domain::error::Result;

pub fn run(s: &Session, range: &RangeArgs, saved: Option<PathBuf>) -> Result<()> {
    let (repo, cfg) = s.open()?;
    warn_if_inert(&cfg);
    let plan = match saved {
        Some(p) => {
            let plan = plan_file::load(&p)?;
            ensure_plan_matches_config(&plan, &cfg)?;
            plan
        }
        None => {
            let built = build_plan(&repo, &cfg, &plan_options(range, true))?;
            for w in &built.warnings {
                eprintln!("warning: {w}");
            }
            built.plan
        }
    };
    let report = apply(&repo, &plan, range.rewrite_pushed, now())?;
    print_apply(&report, plan.branch_name());
    Ok(())
}
