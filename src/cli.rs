use std::io::Read;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::adapters::config_file::{self, CONFIG_FILE, starter_config};
use crate::apply::{apply, list_backups, prune, restore};
use crate::domain::error::{Error, Result};
use crate::domain::settings::{Backend, Config};
use crate::git::Git;
use crate::hook;
use crate::llm;
use crate::plan::{Built, Plan, PlanOptions, build_plan, render};

#[derive(Parser)]
#[command(
    name = "ghma",
    version,
    about = "git hide my ass: safely reshape the history of the current branch"
)]
struct Cli {
    /// Config file (default: .git-hide-my-ass.yml in the repository root).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Run as if started in this directory.
    #[arg(short = 'C', global = true)]
    dir: Option<PathBuf>,
    /// Object backend: `git` (default) or `gix` (needs the `gix` cargo feature).
    /// Also settable with GHMA_BACKEND or `backend:` in the config.
    #[arg(long, global = true)]
    backend: Option<Backend>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct RangeArgs {
    /// Exclusive lower bound of the range (`root` = no bound). Required without an upstream.
    #[arg(long)]
    from: Option<String>,
    /// Allow rewriting commits that are already on the upstream.
    #[arg(long)]
    rewrite_pushed: bool,
    /// Rewrite every commit in the range, even ones that already conform.
    #[arg(long)]
    all: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a starter config file.
    Init {
        #[arg(long)]
        force: bool,
    },
    /// Show what would change (dry run).
    Plan {
        #[command(flatten)]
        range: RangeArgs,
        /// Save the plan as JSON.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Exit with code 6 if any commit is nonconforming.
        #[arg(long)]
        check: bool,
    },
    /// Rewrite the branch (verifies first, keeps a backup).
    Apply {
        #[command(flatten)]
        range: RangeArgs,
        /// Apply a saved plan instead of planning now.
        #[arg(long)]
        plan: Option<PathBuf>,
    },
    /// Export commits as compact JSONL for an LLM.
    Export {
        #[command(flatten)]
        range: RangeArgs,
        #[arg(long)]
        plan: Option<PathBuf>,
        #[arg(long)]
        batch: Option<usize>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Import an LLM reply (JSONL) into a saved plan.
    Import {
        #[arg(long)]
        plan: PathBuf,
        /// Write the updated plan here (default: overwrite --plan).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Reply file, or `-` for stdin.
        reply: String,
    },
    /// List backups, restore one, or prune one.
    Restore {
        id: Option<String>,
        /// Restore even if the branch has moved on since.
        #[arg(long)]
        force: bool,
        /// Delete the backup refs instead of restoring.
        #[arg(long)]
        prune: bool,
    },
    /// Manage the git hook.
    Hook {
        #[command(subcommand)]
        cmd: HookCmd,
    },
}

#[derive(Subcommand)]
enum HookCmd {
    Install {
        #[arg(long)]
        force: bool,
    },
    Uninstall,
    /// Entry point used by the installed shim.
    Run {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

/// `--backend`, then `GHMA_BACKEND`, then the config's `backend:`, then the build's default
/// (`gix` when compiled in, otherwise `git`).
fn resolve_backend(
    flag: Option<Backend>,
    env: Option<&str>,
    cfg: Option<Backend>,
) -> Result<Backend> {
    let env = match env.map(str::trim) {
        Some(v) if !v.is_empty() => Some(v.parse::<Backend>()?),
        _ => None,
    };
    Ok(flag.or(env).or(cfg).unwrap_or(if cfg!(feature = "gix") {
        Backend::Gix
    } else {
        Backend::Git
    }))
}

/// Loads the config and switches `git` to the selected backend.
fn setup(git: &mut Git, config: &Option<PathBuf>, backend: Option<Backend>) -> Result<Config> {
    let cfg = load_config(git, config)?;
    let env = std::env::var("GHMA_BACKEND").ok();
    let want = resolve_backend(backend, env.as_deref(), cfg.backend)?;
    if want != git.backend() {
        *git = git.clone().with_backend(want)?;
    }
    Ok(cfg)
}

fn load_config(git: &Git, explicit: &Option<PathBuf>) -> Result<Config> {
    match explicit {
        Some(p) => config_file::load(p),
        None => {
            let p = git.dir().join(CONFIG_FILE);
            if p.exists() {
                config_file::load(&p)
            } else {
                Ok(Config::default())
            }
        }
    }
}

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

fn print_plan(git: &Git, b: &Built) -> Result<()> {
    let p = &b.plan;
    if p.entries.is_empty() {
        if b.range_len == 0 {
            println!("Nothing to do: the range contains no commits.");
        } else {
            println!(
                "Nothing to do: all {} commit(s) in the range already conform.",
                b.range_len
            );
        }
        return Ok(());
    }
    println!(
        "{} commit(s) in range, {} kept as-is, {} to rewrite (branch {}):",
        b.range_len,
        b.frozen,
        p.entries.len(),
        p.branch_name()
    );
    let old: Vec<_> = git.read_commits(
        &p.entries
            .iter()
            .map(|e| e.old_oid.clone())
            .collect::<Vec<_>>(),
    )?;
    print!("{}", render(p, &old));
    for w in &b.warnings {
        println!("warning: {w}");
    }
    Ok(())
}

fn run(cli: Cli) -> Result<()> {
    let start = match &cli.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir()?,
    };
    let mut git = Git::open(&start)?;
    match cli.cmd {
        Cmd::Init { force } => {
            let path = git.dir().join(CONFIG_FILE);
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
            let cfg = setup(&mut git, &cli.config, cli.backend)?;
            let built = build_plan(&git, &cfg, &opts(&range, false))?;
            print_plan(&git, &built)?;
            if let Some(out) = out {
                built.plan.save(&out)?;
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
            let cfg = setup(&mut git, &cli.config, cli.backend)?;
            let plan = match plan {
                Some(p) => Plan::load(&p)?,
                None => {
                    let built = build_plan(&git, &cfg, &opts(&range, true))?;
                    for w in &built.warnings {
                        eprintln!("warning: {w}");
                    }
                    built.plan
                }
            };
            let report = apply(&git, &plan, range.rewrite_pushed)?;
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
            let cfg = setup(&mut git, &cli.config, cli.backend)?;
            let plan = match plan {
                Some(p) => Plan::load(&p)?,
                None => build_plan(&git, &cfg, &opts(&range, false))?.plan,
            };
            let (prelude, rows) = llm::export(&git, &plan, batch, offset)?;
            eprintln!("{prelude}");
            for r in rows {
                println!("{r}");
            }
        }
        Cmd::Import { plan, out, reply } => {
            let cfg = setup(&mut git, &cli.config, cli.backend)?;
            let mut p = Plan::load(&plan)?;
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
            p.save(&dest)?;
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
                let all = list_backups(&git)?;
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
                let b = prune(&git, &id)?;
                println!(
                    "pruned backup {} (the original commits may now be garbage-collected)",
                    b.id
                );
            }
            Some(id) => {
                let (b, parked) = restore(&git, &id, force)?;
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
                println!("installed {}", hook::install(&git, force)?.display())
            }
            HookCmd::Uninstall => {
                println!(
                    "{}",
                    if hook::uninstall(&git)? {
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
                let cfg = setup(&mut git, &cli.config, cli.backend)?;
                let mut stdin = String::new();
                std::io::stdin().read_to_string(&mut stdin)?;
                hook::run_pre_push(
                    &git,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_precedence_is_flag_env_config_default() {
        let default = if cfg!(feature = "gix") {
            Backend::Gix
        } else {
            Backend::Git
        };
        assert_eq!(resolve_backend(None, None, None).unwrap(), default);
        // Opting out of the default: via config, via env, and the flag beats both.
        assert_eq!(
            resolve_backend(None, None, Some(Backend::Git)).unwrap(),
            Backend::Git
        );
        assert_eq!(
            resolve_backend(None, Some("git"), Some(Backend::Gix)).unwrap(),
            Backend::Git
        );
        assert_eq!(
            resolve_backend(Some(Backend::Gix), Some("git"), Some(Backend::Git)).unwrap(),
            Backend::Gix
        );
        assert_eq!(resolve_backend(None, Some("  "), None).unwrap(), default);
        assert!(resolve_backend(None, Some("bogus"), None).is_err());
    }
}
