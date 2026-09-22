//! In-process object access through gitoxide: no process is spawned per object.

use crate::application::ports::CommitStore;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{Commit, NewCommit, build_commit_buffer};
use crate::domain::settings::Signing;

/// Reads and writes commit objects of one repository in-process.
#[derive(Clone)]
pub struct GixStore {
    repo: gix::Repository,
}

impl GixStore {
    pub fn open(dir: &std::path::Path) -> Result<GixStore> {
        let repo = gix::open(dir)
            .map_err(|e| Error::Git(format!("gix cannot open the repository: {e}")))?;
        Ok(GixStore { repo })
    }

    fn oid(oid: &str) -> Result<gix::ObjectId> {
        gix::ObjectId::from_hex(oid.as_bytes())
            .map_err(|e| Error::Git(format!("bad object id {oid:?}: {e}")))
    }
}

impl CommitStore for GixStore {
    fn read_commits(&self, oids: &[String]) -> Result<Vec<Commit>> {
        oids.iter()
            .map(|oid| {
                let obj = self
                    .repo
                    .find_object(Self::oid(oid)?)
                    .map_err(|e| Error::Git(format!("cannot read {oid}: {e}")))?;
                if obj.kind != gix::objs::Kind::Commit {
                    return Err(Error::Git(format!("object {oid} is not a commit")));
                }
                Commit::parse(oid, &obj.data)
            })
            .collect()
    }

    fn objects_exist(&self, oids: &[String]) -> Result<bool> {
        for o in oids {
            if !self.repo.has_object(Self::oid(o)?) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Only unsigned commits: signing needs the user's gpg/ssh setup, which stays with the CLI.
    fn write_commit(&self, commit: &NewCommit, signing: Signing) -> Result<String> {
        if signing == Signing::Resign {
            return Err(Error::Internal(
                "the gix store cannot sign commits; use the git CLI adapter".into(),
            ));
        }
        let buf = build_commit_buffer(commit);
        let id = gix::objs::Write::write_buf(&self.repo.objects, gix::objs::Kind::Commit, &buf)
            .map_err(|e| Error::Git(format!("gix cannot write a commit: {e}")))?;
        Ok(id.to_string())
    }
}
