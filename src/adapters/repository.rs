//! Composition: the git CLI for everything, with commit reads and writes optionally served
//! in-process by the gix store.

use super::git_cli::GitCli;
#[cfg(not(feature = "gix"))]
use crate::domain::error::Error;
use crate::domain::error::Result;
use crate::domain::settings::Backend;

/// Attaches the selected object backend. `Gix` fails cleanly when it was not compiled in.
pub fn with_backend(cli: GitCli, backend: Backend) -> Result<GitCli> {
    match backend {
        Backend::Git => Ok(cli),
        #[cfg(feature = "gix")]
        Backend::Gix => {
            let store = super::gix_store::GixStore::open(cli.dir())?;
            Ok(cli.with_objects(Box::new(store)))
        }
        #[cfg(not(feature = "gix"))]
        Backend::Gix => Err(Error::Usage(
            "this build has no gix backend (rebuild with default features)".into(),
        )),
    }
}
