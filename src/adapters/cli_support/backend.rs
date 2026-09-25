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

/// Opens the repository with the selected backend and loads the config.
pub(super) fn open(
    dir: &Path,
    config: &Option<PathBuf>,
    flag: Option<Backend>,
) -> Result<(GitCli, Config)> {
    let cli = GitCli::open(dir)?;
    let cfg = load_config(&cli, config)?;
    let env = std::env::var("GCMA_BACKEND").ok();
    let want = resolve_backend(flag, env.as_deref(), cfg.backend)?;
    Ok((repository::with_backend(cli, want)?, cfg))
}

fn load_config(cli: &GitCli, explicit: &Option<PathBuf>) -> Result<Config> {
    match explicit {
        Some(p) => config_file::load(p),
        None => {
            let p = cli.dir().join(CONFIG_FILE);
            if p.exists() {
                config_file::load(&p)
            } else {
                Ok(Config::default())
            }
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
