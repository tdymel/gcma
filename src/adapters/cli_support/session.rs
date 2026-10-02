//! What every command shares: where to run, which config and backend to use, and the clock.

use std::path::PathBuf;

use super::args::RangeArgs;
use super::backend::{load_config, open, open_repo};
use crate::adapters::config_file::CONFIG_FILE;
use crate::adapters::git_cli::GitCli;
use crate::application::planning::PlanOptions;
use crate::domain::error::Result;
use crate::domain::settings::{Backend, Config};

pub struct Session {
    pub start: PathBuf,
    pub config: Option<PathBuf>,
    pub backend: Option<Backend>,
}

impl Session {
    /// The repository (with the selected backend) and the config.
    pub fn open(&self) -> Result<(GitCli, Config)> {
        open(&self.start, self.config.as_deref(), self.backend)
    }

    /// The repository alone: the config is never read, so a broken one cannot block commands
    /// that undo or clean up (`restore`, `hook install`, `hook uninstall`).
    pub fn open_repo(&self) -> Result<GitCli> {
        open_repo(&self.start, self.backend)
    }

    /// The config alone, without opening an object backend (an explicit `--config` needs no
    /// repository at all).
    pub fn load_config(&self) -> Result<Config> {
        load_config(self.config.as_deref(), || {
            Ok(GitCli::open(&self.start)?.dir().to_path_buf())
        })
    }
}

/// Planning options from the command line. `strict` also refuses a dirty index or a running
/// operation; read-only commands skip that (`apply` enforces it again before writing).
pub fn plan_options(r: &RangeArgs, strict: bool) -> PlanOptions {
    PlanOptions {
        from_rev: r.from.clone(),
        rewrite_pushed: r.rewrite_pushed,
        all: r.all,
        strict,
        retag_hint: true,
        ..PlanOptions::new(now())
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn warn_if_inert(cfg: &Config) {
    if cfg.is_inert() {
        eprintln!(
            "warning: no rules are configured, so nothing will change \
             (write {CONFIG_FILE} with `gcma init` and enable what you need)"
        );
    }
}
