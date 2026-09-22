//! The command line grammar.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::domain::settings::Backend;

#[derive(Parser)]
#[command(
    name = "ghma",
    version,
    about = "git hide my ass: safely reshape the history of the current branch"
)]
pub struct Cli {
    /// Config file (default: .git-hide-my-ass.yml in the repository root).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Run as if started in this directory.
    #[arg(short = 'C', global = true)]
    pub dir: Option<PathBuf>,
    /// Object backend: `gix` (default) or `git`.
    /// Also settable with GHMA_BACKEND or `backend:` in the config.
    #[arg(long, global = true)]
    pub backend: Option<Backend>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Args, Clone)]
pub struct RangeArgs {
    /// Exclusive lower bound of the range (`root` = no bound). Required without an upstream.
    #[arg(long)]
    pub from: Option<String>,
    /// Allow rewriting commits that are already on the upstream.
    #[arg(long)]
    pub rewrite_pushed: bool,
    /// Rewrite every commit in the range, even ones that already conform.
    #[arg(long)]
    pub all: bool,
}

#[derive(Subcommand)]
pub enum Cmd {
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
pub enum HookCmd {
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
