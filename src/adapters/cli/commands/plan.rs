//! `gcma plan`: the dry run.

use std::path::PathBuf;

use crate::adapters::cli_support::args::RangeArgs;
use crate::adapters::cli_support::report::print_plan;
use crate::adapters::cli_support::session::{Session, plan_options, warn_if_inert};
use crate::adapters::plan_file;
use crate::application::planning::{PlanOptions, build_plan};
use crate::domain::error::{Error, Result};

pub fn run(
    s: &Session,
    range: &RangeArgs,
    out: Option<PathBuf>,
    check: bool,
    retag: bool,
) -> Result<()> {
    let (repo, cfg) = s.open()?;
    warn_if_inert(&cfg);
    let opts = PlanOptions {
        retag,
        ..plan_options(range, false)
    };
    let built = build_plan(&repo, &cfg, &opts)?;
    print_plan(&repo, &built)?;
    if let Some(out) = out {
        plan_file::save(&built.plan, &out)?;
        println!("plan written to {}", out.display());
    }
    if check && !built.plan.is_empty() {
        return Err(Error::Nonconforming(format!(
            "{} commit(s) do not follow the rules",
            built.plan.entries.len() + built.plan.dropped.len()
        )));
    }
    Ok(())
}
