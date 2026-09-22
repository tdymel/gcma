//! The driving adapter: parses the command line and calls the use cases.

mod args;
mod backend;
mod report;

use std::io::Read;
use std::path::Path;

use clap::Parser;

use self::args::{Cli, Cmd, HookCmd, RangeArgs};
use self::backend::open;
use self::report::print_plan;
use crate::adapters::config_file::{CONFIG_FILE, starter_config};
use crate::adapters::git_cli::GitCli;
use crate::adapters::{hook_installer, plan_file};
use crate::application::planning::{PlanOptions, build_plan};
use crate::application::rewrite::{apply, list_backups, prune, restore};
use crate::application::{llm, push_guard};
use crate::domain::error::{Error, Result};

/// `strict` also refuses a dirty index or a running operation; read-only commands skip that
/// (`apply` enforces it again before writing).
fn opts(r: &RangeArgs, strict: bool) -> PlanOptions {
    PlanOptions {
        from_rev: r.from.clone(),
        rewrite_pushed: r.rewrite_pushed,
        all: r.all,
        strict,
        ..Default::default()
    }
}

fn run(cli: Cli) -> Result<()> {
    let start = match &cli.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir()?,
    };
    match cli.cmd {
        Cmd::Init { force } => {
            let path = GitCli::open(&start)?.dir().join(CONFIG_FILE);
            if path.exists() && !force {
                return Err(Error::Precondition(format!(
                    "{} already exists (use --force)",
                    path.display()
                )));
            }
            std::fs::write(&path, starter_config())?;
            println!("wrote {}", path.display());
        }
        Cmd::Plan { range, out, check } => {
            let (repo, cfg) = open(&start, &cli.config, cli.backend)?;
            let built = build_plan(&repo, &cfg, &opts(&range, false))?;
            print_plan(&repo, &built)?;
            if let Some(out) = out {
                plan_file::save(&built.plan, &out)?;
                println!("plan written to {}", out.display());
            }
            if check && !built.plan.entries.is_empty() {
                return Err(Error::Nonconforming(format!(
                    "{} commit(s) do not follow the rules",
                    built.plan.entries.len()
                )));
            }
        }
        Cmd::Apply { range, plan } => {
            let (repo, cfg) = open(&start, &cli.config, cli.backend)?;
            let plan = match plan {
                Some(p) => plan_file::load(&p)?,
                None => {
                    let built = build_plan(&repo, &cfg, &opts(&range, true))?;
                    for w in &built.warnings {
                        eprintln!("warning: {w}");
                    }
                    built.plan
                }
            };
            let report = apply(&repo, &plan, range.rewrite_pushed)?;
            if report.noop {
                println!("Nothing to do.");
            } else {
                println!(
                    "Rewrote {} commit(s); {} is now at {}.\nBackup: {} (undo with `ghma restore {}`)",
                    report.rewritten,
                    plan.branch_name(),
                    report.new_tip.as_deref().unwrap_or("?"),
                    report.backup_id.as_deref().unwrap_or("?"),
                    report.backup_id.as_deref().unwrap_or("?")
                );
            }
        }
        Cmd::Export {
            range,
            plan,
            batch,
            offset,
        } => {
            let (repo, cfg) = open(&start, &cli.config, cli.backend)?;
            let plan = match plan {
                Some(p) => plan_file::load(&p)?,
                None => build_plan(&repo, &cfg, &opts(&range, false))?.plan,
            };
            let (prelude, rows) = llm::export(&repo, &plan, batch, offset)?;
            eprintln!("{prelude}");
            for r in rows {
                println!("{r}");
            }
        }
        Cmd::Import { plan, out, reply } => {
            let (_, cfg) = open(&start, &cli.config, cli.backend)?;
            let mut p = plan_file::load(&plan)?;
            let text = if reply == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                std::fs::read_to_string(Path::new(&reply))
                    .map_err(|e| Error::Usage(format!("cannot read {reply}: {e}")))?
            };
            let rep = llm::import(&mut p, &text, &cfg)?;
            let dest = out.unwrap_or(plan);
            plan_file::save(&p, &dest)?;
            println!(
                "imported {} message(s) ({} unchanged); plan saved to {}",
                rep.changed,
                rep.unchanged_rows,
                dest.display()
            );
        }
        Cmd::Restore {
            id,
            force,
            prune: do_prune,
        } => match id {
            None if do_prune => {
                return Err(Error::Usage("--prune needs a backup id".into()));
            }
            None => {
                let (repo, _) = open(&start, &cli.config, cli.backend)?;
                let all = list_backups(&repo)?;
                if all.is_empty() {
                    println!("No backups.");
                }
                for b in all {
                    println!(
                        "{}  branch {}  old {}  new {}",
                        b.id, b.branch, b.old, b.new
                    );
                }
            }
            Some(id) if do_prune => {
                let (repo, _) = open(&start, &cli.config, cli.backend)?;
                let b = prune(&repo, &id)?;
                println!(
                    "pruned backup {} (the original commits may now be garbage-collected)",
                    b.id
                );
            }
            Some(id) => {
                let (repo, _) = open(&start, &cli.config, cli.backend)?;
                let (b, parked) = restore(&repo, &id, force)?;
                println!("{} restored to {}", b.branch, b.old);
                if let Some(r) = parked {
                    println!(
                        "The newer commits are kept at {r}. The index and working tree were not \
                         touched and may still hold their content (inspect with `git status`)."
                    );
                }
            }
        },
        Cmd::Hook { cmd } => match cmd {
            HookCmd::Install { force } => {
                let (repo, _) = open(&start, &cli.config, cli.backend)?;
                let path = repo.git_path("hooks/pre-push")?;
                println!(
                    "installed {}",
                    hook_installer::install(&path, force)?.display()
                )
            }
            HookCmd::Uninstall => {
                let (repo, _) = open(&start, &cli.config, cli.backend)?;
                println!(
                    "{}",
                    if hook_installer::uninstall(&repo.git_path("hooks/pre-push")?)? {
                        "hook removed"
                    } else {
                        "no hook installed"
                    }
                )
            }
            HookCmd::Run { name, args } => {
                if name != "pre-push" {
                    return Err(Error::Usage(format!("unsupported hook {name:?}")));
                }
                let (repo, cfg) = open(&start, &cli.config, cli.backend)?;
                let mut stdin = String::new();
                std::io::stdin().read_to_string(&mut stdin)?;
                push_guard::run_pre_push(
                    &repo,
                    &cfg,
                    args.first().map(String::as_str).unwrap_or(""),
                    &stdin,
                )?;
            }
        },
    }
    Ok(())
}

pub fn main() -> i32 {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ghma: {e}");
            e.exit_code()
        }
    }
}
