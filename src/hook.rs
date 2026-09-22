//! Git hook integration. The installed shim only calls `ghma hook run`.

use std::os::unix::fs::PermissionsExt;

use crate::apply::apply;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::is_zero_oid;
use crate::domain::settings::{Config, HookMode};
use crate::git::Git;
use crate::plan::{PlanOptions, RangeSpec, build_plan};

const MARKER: &str = "ghma-managed-hook";

pub fn install(git: &Git, force: bool) -> Result<std::path::PathBuf> {
    let path = git.git_path("hooks/pre-push")?;
    if path.exists() && !force {
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if !existing.contains(MARKER) {
            return Err(Error::Precondition(format!(
                "{} already exists and is not managed by ghma; use --force to overwrite",
                path.display()
            )));
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "ghma".into());
    let script = format!(
        "#!/bin/sh\n# {MARKER}\nGHMA='{}'\n[ -x \"$GHMA\" ] || GHMA=ghma\nexec \"$GHMA\" hook run pre-push \"$@\"\n",
        exe.replace('\'', "'\\''")
    );
    std::fs::write(&path, script)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

pub fn uninstall(git: &Git) -> Result<bool> {
    let path = git.git_path("hooks/pre-push")?;
    if !path.exists() {
        return Ok(false);
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if !existing.contains(MARKER) {
        return Err(Error::Precondition(format!(
            "{} is not managed by ghma; not removing it",
            path.display()
        )));
    }
    std::fs::remove_file(&path)?;
    Ok(true)
}

/// pre-push: judges (and in `rewrite` mode fixes) the commits about to be pushed.
/// `Ok(())` lets the push proceed; an error aborts it.
pub fn run_pre_push(git: &Git, cfg: &Config, remote: &str, stdin: &str) -> Result<()> {
    let remotes: Vec<String> = git
        .text(&["remote"])?
        .lines()
        .map(|s| s.to_string())
        .collect();
    for line in stdin.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 4 {
            continue;
        }
        let (local_ref, local_sha, _remote_ref, remote_sha) =
            (parts[0], parts[1], parts[2], parts[3]);
        if is_zero_oid(local_sha) {
            continue; // a delete push
        }
        // Only pushes of the checked-out branch are judged: by name, as `HEAD` (which is what
        // `git push origin HEAD` and `HEAD:<ref>` report), or by a ref/sha that is the branch tip.
        let Some(branch_ref) = git.current_branch_ref()? else {
            continue; // detached HEAD
        };
        let tip = git.ref_value(&branch_ref)?;
        let is_branch =
            local_ref == branch_ref || local_ref == "HEAD" || tip.as_deref() == Some(local_sha);
        if !is_branch {
            continue;
        }
        let local_ref = branch_ref.as_str();
        let known_remote = !is_zero_oid(remote_sha) && git.rev_parse(remote_sha)?.is_some();
        let (exclude, base) = if known_remote {
            (vec![format!("^{remote_sha}")], Some(remote_sha.to_string()))
        } else if remotes.iter().any(|r| r == remote) {
            (
                vec!["--not".to_string(), format!("--remotes={remote}")],
                None,
            )
        } else {
            (vec!["--not".to_string(), "--remotes".to_string()], None)
        };
        let range = RangeSpec {
            tip: local_sha.to_string(),
            branch_ref: local_ref.to_string(),
            exclude,
            base,
        };
        // Rewriting is only possible when the pushed commit is the branch tip; the plan is built
        // once, with the strict (clean index) preconditions only when we are going to write.
        let rewrite = cfg.hook.mode == HookMode::Rewrite && tip.as_deref() == Some(local_sha);
        let opts = PlanOptions {
            range: Some(range),
            strict: rewrite,
            ..Default::default()
        };
        let built = build_plan(git, cfg, &opts)?;
        let n = built.plan.entries.len();
        if n == 0 {
            continue;
        }
        if rewrite {
            let report = apply(git, &built.plan, false)?;
            if !report.noop {
                return Err(Error::Nonconforming(format!(
                    "ghma rewrote {} unpushed commit(s) of {local_ref} to follow the rules; run `git push` again",
                    report.rewritten
                )));
            }
            continue;
        }
        let hint = if git.upstream_oid(local_ref)?.is_none() {
            " (the branch has no upstream: add `--from <rev>`)"
        } else {
            ""
        };
        return Err(Error::Nonconforming(format!(
            "{n} commit(s) about to be pushed do not follow the ghma rules; \
             run `ghma apply`{hint} (it rewrites the unpushed part of the branch) and push again"
        )));
    }
    Ok(())
}
