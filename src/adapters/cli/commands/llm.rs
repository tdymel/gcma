//! `ghma export` and `ghma import`: the round trip through an LLM.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::adapters::cli_support::args::RangeArgs;
use crate::adapters::cli_support::report::sanitize;
use crate::adapters::cli_support::session::{Session, plan_options};
use crate::adapters::{llm_jsonl, plan_file};
use crate::application::llm;
use crate::application::planning::build_plan;
use crate::domain::error::{Error, Result};

pub fn export(
    s: &Session,
    range: &RangeArgs,
    saved: Option<PathBuf>,
    batch: Option<usize>,
    offset: usize,
) -> Result<()> {
    let (repo, cfg) = s.open()?;
    let plan = match saved {
        Some(p) => plan_file::load(&p)?,
        None => build_plan(&repo, &cfg, &plan_options(range, false))?.plan,
    };
    let (prelude, rows) = llm_jsonl::render_export(&llm::export(&repo, &plan, batch, offset)?)?;
    eprintln!("{}", sanitize(&prelude));
    for r in rows {
        println!("{r}");
    }
    Ok(())
}

pub fn import(s: &Session, plan: PathBuf, out: Option<PathBuf>, reply: &str) -> Result<()> {
    let (_, cfg) = s.open()?;
    let mut p = plan_file::load(&plan)?;
    let text = if reply == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        std::fs::read_to_string(Path::new(reply))
            .map_err(|e| Error::Usage(format!("cannot read {reply}: {e}")))?
    };
    let rep = llm::import(&mut p, llm_jsonl::parse_reply(&text), &cfg)?;
    let dest = out.unwrap_or(plan);
    plan_file::save(&p, &dest)?;
    println!(
        "imported {} message(s) ({} unchanged); plan saved to {}",
        rep.changed,
        rep.unchanged_rows,
        dest.display()
    );
    Ok(())
}
