//! `gcma hook`: install, uninstall, and the entry point of the installed shim.

use std::io::Read;

use crate::adapters::cli_support::args::HookCmd;
use crate::adapters::cli_support::session::{Session, now};
use crate::adapters::hook_installer;
use crate::application::push_guard::{self, PushedRef};
use crate::domain::error::{Error, Result};

const PRE_PUSH: &str = "hooks/pre-push";

pub fn run(s: &Session, cmd: HookCmd) -> Result<()> {
    match cmd {
        HookCmd::Install { force } => {
            let (repo, _) = s.open()?;
            let path = hook_installer::install(&repo.git_path(PRE_PUSH)?, force)?;
            println!("installed {}", path.display());
        }
        HookCmd::Uninstall => {
            let (repo, _) = s.open()?;
            let removed = hook_installer::uninstall(&repo.git_path(PRE_PUSH)?)?;
            println!(
                "{}",
                if removed {
                    "hook removed"
                } else {
                    "no hook installed"
                }
            );
        }
        HookCmd::Run { name, args } => {
            if name != "pre-push" {
                return Err(Error::Usage(format!("unsupported hook {name:?}")));
            }
            let (repo, cfg) = s.open()?;
            let mut stdin = String::new();
            std::io::stdin().read_to_string(&mut stdin)?;
            let remote = args.first().map(String::as_str).unwrap_or("");
            push_guard::run_pre_push(&repo, &cfg, remote, &parse_pushed_refs(&stdin), now())?;
        }
    }
    Ok(())
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
