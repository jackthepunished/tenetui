//! Materialize the file's content at a given commit via a tree lookup.
//!
//! M0 reads directly; the LRU cache + background prefetch arrive in M1/M2
//! (see docs/roadmap.md). The signature already takes an `Oid` so those layers
//! slot in without touching callers.

use super::Snapshot;
use anyhow::{Context, Result};
use git2::{Oid, Repository};
use std::path::Path;

/// The file's content at `oid`. A missing file yields `existed: false` rather
/// than an error — that's a renderable state ("not yet created").
pub fn snapshot_at(repo: &Repository, oid: Oid, path: &str) -> Result<Snapshot> {
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;

    match tree.get_path(Path::new(path)) {
        Ok(entry) => {
            let object = entry.to_object(repo)?;
            let blob = object
                .as_blob()
                .with_context(|| format!("{path} is not a file at {oid}"))?;
            let content = String::from_utf8_lossy(blob.content()).into_owned();
            Ok(Snapshot {
                oid,
                content: content.into(),
                existed: true,
            })
        }
        Err(_) => Ok(Snapshot {
            oid,
            content: "".into(),
            existed: false,
        }),
    }
}

/// Convenience: the snapshot at `HEAD`.
pub fn head_snapshot(repo: &Repository, path: &str) -> Result<Snapshot> {
    let head = repo.head()?.peel_to_commit()?;
    snapshot_at(repo, head.id(), path)
}
