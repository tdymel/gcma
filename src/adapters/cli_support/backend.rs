//! Choosing the object backend and opening the repository with it.

use std::path::{Path, PathBuf};

use crate::adapters::config_file::{self, CONFIG_FILE};
use crate::adapters::git_cli::GitCli;
use crate::adapters::repository;
use crate::domain::error::Result;
use crate::domain::settings::{Backend, Config};

/// `--backend`, then `GCMA_BACKEND`, then the config's `backend:`, then the build's default
/// (`gix` when compiled in, otherwise `git`).
pub(super) fn resolve_backend(
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

/// Opens the repository with the backend from `--backend` or `GCMA_BACKEND` (or the build's
/// default), without reading any config. This is what the undo path (`restore`, `hook install`
/// and `uninstall`) uses, so a broken `gcma.yml` can never block it.
pub(super) fn open_repo(dir: &Path, flag: Option<Backend>) -> Result<GitCli> {
    attach(GitCli::open(dir)?, flag, None)
}

/// Opens the repository and loads the config; the config's `backend:` is the third choice.
pub(super) fn open(
    dir: &Path,
    config: Option<&Path>,
    flag: Option<Backend>,
) -> Result<(GitCli, Config)> {
    let cli = GitCli::open(dir)?;
    let cfg = load_config(config, || Ok(cli.dir().to_path_buf()))?;
    let repo = attach(cli, flag, cfg.backend)?;
    Ok((repo, cfg))
}

/// Loads the config: the explicit file, else `gcma.yml` in the repository root (`root` is only
/// asked for in that case), else the defaults.
pub(super) fn load_config(
    explicit: Option<&Path>,
    root: impl FnOnce() -> Result<PathBuf>,
) -> Result<Config> {
    match explicit {
        Some(p) => config_file::load(p),
        None => {
            let p = root()?.join(CONFIG_FILE);
            if p.exists() {
                config_file::load(&p)
            } else {
                Ok(Config::default())
            }
        }
    }
}

fn attach(cli: GitCli, flag: Option<Backend>, cfg: Option<Backend>) -> Result<GitCli> {
    let env = std::env::var("GCMA_BACKEND").ok();
    repository::with_backend(cli, resolve_backend(flag, env.as_deref(), cfg)?)
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
