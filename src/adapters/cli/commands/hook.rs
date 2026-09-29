//! `gcma hook`: install, uninstall, and the entry point of the installed shims.

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind, set_hook, take_hook};

use crate::adapters::cli_support::args::HookCmd;
use crate::adapters::cli_support::session::{Session, now};
use crate::adapters::git_cli::NESTED_ENV;
use crate::adapters::hook_installer::{self, Hook};
use crate::application::commit_hook;
use crate::application::push_guard::{self, PushedRef};
use crate::domain::error::{Error, Result};

pub fn run(s: &Session, cmd: HookCmd) -> Result<()> {
    match cmd {
        HookCmd::Install { post_commit, force } => {
            let hooks: &[Hook] = if post_commit {
                &Hook::ALL
            } else {
                &[Hook::PrePush]
            };
            for path in hook_installer::install(&s.open_repo()?, hooks, force)? {
                println!("installed {}", path.display());
            }
        }
        HookCmd::Uninstall => {
            let removed = hook_installer::uninstall(&s.open_repo()?)?;
            if removed.is_empty() {
                println!("no hook installed");
            } else {
                let names: Vec<&str> = removed.iter().map(|h| h.name()).collect();
                println!("hook removed ({})", names.join(", "));
            }
        }
        HookCmd::Run { name, args } => match name.as_str() {
            "pre-push" => pre_push(s, &args)?,
            "post-commit" => post_commit(s),
            _ => return Err(Error::Usage(format!("unsupported hook {name:?}"))),
        },
    }
    Ok(())
}

fn pre_push(s: &Session, args: &[String]) -> Result<()> {
    let (repo, cfg) = s.open()?;
    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin)?;
    let remote = args.first().map(String::as_str).unwrap_or("");
    push_guard::run_pre_push(&repo, &cfg, remote, &parse_pushed_refs(&stdin), now())
}

/// The commit is done and must stay done: whatever goes wrong here (even a panic) is one line on
/// stderr, and the exit code stays 0.
fn post_commit(s: &Session) {
    if std::env::var_os(NESTED_ENV).is_some() {
        return;
    }
    let quiet = take_hook();
    set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(|| {
        let (repo, cfg) = s.open()?;
        commit_hook::run_post_commit(&repo, &cfg, now())
    }));
    set_hook(quiet);
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => eprintln!("gcma: post-commit skipped: {e}"),
        Err(_) => eprintln!("gcma: post-commit skipped: internal error"),
    }
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
