//! The driving adapter: parses the command line and calls the use cases.

mod args;
mod backend;
mod render;
mod report;

use std::io::Read;
use std::path::Path;

use clap::Parser;

use self::args::{Cli, Cmd, HookCmd, RangeArgs};
use self::backend::open;
use self::report::{print_plan, sanitize};
use crate::adapters::config_file::{CONFIG_FILE, starter_config};
use crate::adapters::fsutil;
use crate::adapters::git_cli::GitCli;
use crate::adapters::{hook_installer, plan_file};
use crate::application::llm;
use crate::application::planning::{PlanOptions, build_plan};
use crate::application::push_guard::{self, PushedRef};
use crate::application::rewrite::{
    apply, ensure_plan_matches_config, list_backups, prune, restore,
};
use crate::domain::error::{Error, Result};
use crate::domain::settings::Config;

/// `strict` also refuses a dirty index or a running operation; read-only commands skip that
/// (`apply` enforces it again before writing).
fn opts(r: &RangeArgs, strict: bool) -> PlanOptions {
    PlanOptions {
        from_rev: r.from.clone(),
        rewrite_pushed: r.rewrite_pushed,
        all: r.all,
        strict,
        ..PlanOptions::new(now())
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// git's pre-push input: `<local ref> <local sha> <remote ref> <remote sha>` per line.
fn parse_pushed_refs(stdin: &str) -> Vec<PushedRef> {
    stdin
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            (parts.len() == 4).then(|| PushedRef {
                local_ref: parts[0].to_string(),
                local_sha: parts[1].to_string(),
                remote_sha: parts[3].to_string(),
            })
        })
        .collect()
}

fn warn_if_inert(cfg: &Config) {
    if cfg.is_inert() {
        eprintln!(
            "warning: no rules are configured, so nothing will change \
             (write {CONFIG_FILE} with `ghma init` and enable what you need)"
        );
    }
}

/// The process exit code of an error.
fn exit_code(e: &Error) -> i32 {
    match e {
        Error::Internal(_) | Error::Git(_) | Error::Io(_) => 1,
        Error::Usage(_) => 2,
        Error::Precondition(_) => 3,
        Error::TipMoved(_) => 4,
        Error::Pushed(_) => 5,
        Error::Nonconforming(_) => 6,
        Error::LlmInvalid(_) => 7,
    }
}

fn run(cli: Cli) -> Result<()> {
    let start = match &cli.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir()?,
    };
    match cli.cmd {
        Cmd::Init { force } => {
            let cli_repo = GitCli::open(&start)?;
            let path = cli_repo.dir().join(CONFIG_FILE);
            if path.exists() && !force {
                return Err(Error::Precondition(format!(
                    "{} already exists (use --force)",
                    path.display()
                )));
            }
            fsutil::write_regular(&path, starter_config().as_bytes())?;
            println!("wrote {}", path.display());
            // The config names identities and paths you want hidden: keep it out of commits.
            let exclude = cli_repo.git_path("info/exclude")?;
            let line = format!("/{CONFIG_FILE}");
            let current = fsutil::read_regular(&exclude)?.unwrap_or_default();
            if !String::from_utf8_lossy(&current).lines().any(|l| l == line) {
                let mut text = String::from_utf8_lossy(&current).to_string();
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&line);
                text.push('\n');
                if let Some(dir) = exclude.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                fsutil::write_regular(&exclude, text.as_bytes())?;
                println!("added {line} to .git/info/exclude so it is not committed by accident");
            }
        }
        Cmd::Plan { range, out, check } => {
            let (repo, cfg) = open(&start, &cli.config, cli.backend)?;
            warn_if_inert(&cfg);
            let built = build_plan(&repo, &cfg, &opts(&range, false))?;
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
        }
        Cmd::Apply { range, plan } => {
            let (repo, cfg) = open(&start, &cli.config, cli.backend)?;
            warn_if_inert(&cfg);
            let plan = match plan {
                Some(p) => {
                    let plan = plan_file::load(&p)?;
                    ensure_plan_matches_config(&plan, &cfg)?;
                    plan
                }
                None => {
                    let built = build_plan(&repo, &cfg, &opts(&range, true))?;
                    for w in &built.warnings {
                        eprintln!("warning: {w}");
                    }
                    built.plan
                }
            };
            let report = apply(&repo, &plan, range.rewrite_pushed, now())?;
            if report.noop {
                println!("Nothing to do.");
            } else {
                let dropped = if report.dropped > 0 {
                    format!(" and dropped {}", report.dropped)
                } else {
                    String::new()
                };
                println!(
                    "Rewrote {} commit(s){dropped}; {} is now at {}.\nBackup: {} (undo with `ghma restore {}`)",
                    report.rewritten,
                    plan.branch_name(),
                    report.new_tip.as_deref().unwrap_or("?"),
                    report.backup_id.as_deref().unwrap_or("?"),
                    report.backup_id.as_deref().unwrap_or("?")
                );
                for n in &report.notes {
                    eprintln!("warning: {n}");
                }
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
            eprintln!("{}", sanitize(&prelude));
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
                    "pruned backup {}. The original commits are only collected once nothing else (reflog, tags, \
other branches) refers to them; see the README on purging history.",
                    b.id
                );
            }
            Some(id) => {
                let (repo, _) = open(&start, &cli.config, cli.backend)?;
                let report = restore(&repo, &id, force)?;
                println!("{} restored to {}", report.backup.branch, report.backup.old);
                for n in &report.notes {
                    eprintln!("warning: {n}");
                }
                if let Some(r) = report.parked {
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
                    &parse_pushed_refs(&stdin),
                    now(),
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
            exit_code(&e)
        }
    }
}
