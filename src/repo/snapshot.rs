//! Materialize the file's content at a given commit via a tree lookup, cached.
//!
//! M1 adds the LRU cache so repeated visits to a commit are free; the background
//! prefetch thread that warms it ahead of the playhead arrives in M2 (see
//! docs/roadmap.md and docs/architecture.md "Threading").

use super::Snapshot;
use anyhow::{Context, Result};
use git2::{Oid, Repository};
use lru::LruCache;
use std::num::NonZeroUsize;
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

/// LRU cache over [`snapshot_at`], keyed by commit oid. `Snapshot` clones are
/// cheap (`content` is `Arc<str>`), so cache hits are just a clone out of the map.
///
/// Single-file per session today, so keying on oid alone is sufficient; a
/// multi-file mode (roadmap M5) would extend the key to `(Oid, path)`.
pub struct SnapshotCache {
    inner: LruCache<Oid, Snapshot>,
}

impl SnapshotCache {
    /// `capacity` bounds memory to a fixed number of snapshots, independent of
    /// repo size (see docs/architecture.md "Performance invariants").
    pub fn new(capacity: usize) -> Self {
        SnapshotCache {
            inner: LruCache::new(NonZeroUsize::new(capacity.max(1)).unwrap()),
        }
    }

    /// Return the cached snapshot for `oid`, or materialize and insert it on miss.
    pub fn get_or_fetch(&mut self, repo: &Repository, oid: Oid, path: &str) -> Result<Snapshot> {
        if let Some(hit) = self.inner.get(&oid) {
            return Ok(hit.clone());
        }
        let snapshot = snapshot_at(repo, oid, path)?;
        self.inner.put(oid, snapshot.clone());
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature};
    use std::fs;

    fn commit(repo: &Repository, file: &str, contents: &str, message: &str) -> Oid {
        fs::write(repo.workdir().unwrap().join(file), contents).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap()
    }

    #[test]
    fn cache_hit_returns_same_content_without_reinserting() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let oid = commit(&repo, "f.txt", "hello\n", "first");

        let mut cache = SnapshotCache::new(4);
        let a = cache.get_or_fetch(&repo, oid, "f.txt").unwrap();
        let b = cache.get_or_fetch(&repo, oid, "f.txt").unwrap();

        assert_eq!(&*a.content, "hello\n");
        assert_eq!(&*a.content, &*b.content);
    }

    #[test]
    fn cache_evicts_least_recently_used_beyond_capacity() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let a_oid = commit(&repo, "f.txt", "a\n", "a");
        let b_oid = commit(&repo, "f.txt", "b\n", "b");
        let c_oid = commit(&repo, "f.txt", "c\n", "c");

        // Capacity 2: fetching a, then b, then c must evict a.
        let mut cache = SnapshotCache::new(2);
        cache.get_or_fetch(&repo, a_oid, "f.txt").unwrap();
        cache.get_or_fetch(&repo, b_oid, "f.txt").unwrap();
        cache.get_or_fetch(&repo, c_oid, "f.txt").unwrap();

        assert!(!cache.inner.contains(&a_oid), "a should have been evicted");
        assert!(cache.inner.contains(&b_oid));
        assert!(cache.inner.contains(&c_oid));
    }
}
