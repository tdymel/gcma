//! A scratch repository (with its own `HOME`) and running `git` and `gcma` in it. Building history
//! is in `commits`, reading it back in `read`.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

pub struct Repo {
    pub dir: TempDir,
    pub home: TempDir,
    /// Forces the object backend of every `gcma` run (`git` or `gix`); `None` keeps the default.
    pub backend: Option<&'static str>,
}

/// The object backends this build has: `git`, and `gix` when its feature is built.
pub const BACKENDS: &[&str] = if cfg!(feature = "gix") {
    &["git", "gix"]
} else {
    &["git"]
};

/// Runs `scenario` once per backend in `BACKENDS`, each time on a fresh repository whose `gcma`
/// runs use that backend.
pub fn on_both_backends(scenario: impl Fn(Repo)) {
    for backend in BACKENDS {
        scenario(Repo::with_backend(backend));
    }
}

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_gcma")
}

pub fn base_cmd(prog: &str, dir: &Path, home: &Path) -> Command {
    let mut c = Command::new(prog);
    c.current_dir(dir);
    c.env("HOME", home);
    c.env("GIT_CONFIG_GLOBAL", "/dev/null");
    c.env("GIT_CONFIG_SYSTEM", "/dev/null");
    c.env("GIT_CONFIG_NOSYSTEM", "1");
    c.env("GIT_TERMINAL_PROMPT", "0");
    c.env_remove("GIT_DIR");
    c.env_remove("GIT_WORK_TREE");
    c
}

impl Repo {
    pub fn new() -> Repo {
        let r = Repo {
            dir: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
            backend: None,
        };
        r.git(&["init", "-q", "-b", "main"]);
        r.git(&["config", "user.name", "Old Me"]);
        r.git(&["config", "user.email", "me@home.org"]);
        r.git(&["config", "commit.gpgsign", "false"]);
        r
    }

    /// A repository whose `gcma` runs use the backend `seed` selects: the two alternate, so a
    /// series of seeds covers both (only `git` when the `gix` feature is not built).
    pub fn for_seed(seed: u64) -> Repo {
        Repo::with_backend(BACKENDS[seed as usize % BACKENDS.len()])
    }

    /// A repository whose `gcma` runs all use `backend`.
    pub fn with_backend(backend: &'static str) -> Repo {
        Repo {
            backend: Some(backend),
            ..Repo::new()
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn cmd(&self, prog: &str) -> Command {
        base_cmd(prog, self.path(), self.home.path())
    }

    // ---------- running commands ----------

    pub fn git_out(&self, args: &[&str]) -> Output {
        self.cmd("git").args(args).output().unwrap()
    }

    pub fn git(&self, args: &[&str]) -> String {
        let o = self.git_out(args);
        assert!(
            o.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout)
            .trim_end_matches('\n')
            .to_string()
    }

    pub fn gcma(&self, args: &[&str]) -> Output {
        let mut c = self.cmd(bin());
        if let Some(b) = self.backend {
            c.env("GCMA_BACKEND", b);
        }
        c.args(args).output().unwrap()
    }

    pub fn gcma_ok(&self, args: &[&str]) -> String {
        let o = self.gcma(args);
        assert!(
            o.status.success(),
            "gcma {:?} failed ({:?}): {}{}",
            args,
            o.status.code(),
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    }

    pub fn code(o: &Output) -> i32 {
        o.status.code().unwrap_or(-1)
    }

    // ---------- files and config ----------

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.path().join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    pub fn config(&self, yaml: &str) {
        self.write("gcma.yml", yaml);
    }
}
