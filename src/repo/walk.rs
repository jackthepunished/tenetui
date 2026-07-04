//! History walk: collect every commit that touched the target file, oldest → newest.

use super::CommitMeta;
use anyhow::Result;
use git2::{DiffOptions, Oid, Repository, Tree};
use std::path::Path;

/// Walk history for `path` (repo-relative, forward slashes) and return the
/// commits that changed it, in chronological order (index 0 = oldest).
///
/// A commit "touched" the file when the file's blob oid differs from its first
/// parent's (added, modified, or deleted). Churn is a diff limited to that path.
pub fn timeline(repo: &Repository, path: &str) -> Result<Vec<CommitMeta>> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    // TOPOLOGICAL guarantees a child never precedes its parent even when commit
    // times tie (rebases, same-second commits); TIME breaks ties among unrelated
    // commits. We collect newest→oldest, then reverse to past → future.
    revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

    let mut out = Vec::new();
    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let tree = commit.tree()?;

        let this_blob = entry_oid(&tree, path);

        // Compare against the first parent only (linear view of history).
        let parent = commit.parent(0).ok();
        let parent_tree = match &parent {
            Some(p) => Some(p.tree()?),
            None => None,
        };
        let parent_blob = parent_tree.as_ref().and_then(|t| entry_oid(t, path));

        // Unchanged relative to parent → this commit didn't touch the file.
        if this_blob == parent_blob {
            continue;
        }

        let (insertions, deletions) = churn(repo, parent_tree.as_ref(), &tree, path)?;
        let author = commit.author();

        out.push(CommitMeta {
            oid,
            time: commit.time().seconds(),
            author: author.name().unwrap_or("unknown").to_string(),
            summary: commit.summary().unwrap_or("").to_string(),
            insertions,
            deletions,
        });
    }

    // revwalk TIME order is newest-first; the timeline reads past → future.
    out.reverse();
    Ok(out)
}

/// The blob oid for `path` in `tree`, or `None` if the file isn't present.
fn entry_oid(tree: &Tree, path: &str) -> Option<Oid> {
    tree.get_path(Path::new(path)).ok().map(|e| e.id())
}

/// Insertions/deletions for `path` between `old_tree` and `new_tree`.
fn churn(
    repo: &Repository,
    old_tree: Option<&Tree>,
    new_tree: &Tree,
    path: &str,
) -> Result<(usize, usize)> {
    let mut opts = DiffOptions::new();
    opts.pathspec(path);
    let diff = repo.diff_tree_to_tree(old_tree, Some(new_tree), Some(&mut opts))?;
    let stats = diff.stats()?;
    Ok((stats.insertions(), stats.deletions()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::head_snapshot;
    use git2::{Repository, Signature};
    use std::fs;
    use std::path::Path;

    /// Stage everything in the workdir and commit it, returning the new commit.
    fn commit(repo: &Repository, message: &str) -> Oid {
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

    fn write(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    /// Build a repo whose `foo.txt` has a known history, plus noise commits that
    /// touch only `other.txt`, and assert the walk sees exactly foo's history.
    #[test]
    fn timeline_tracks_only_the_target_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let repo = Repository::init(dir).unwrap();

        write(dir, "foo.txt", "a\n");
        write(dir, "other.txt", "x\n");
        commit(&repo, "A: add foo + other"); // touches foo

        write(dir, "foo.txt", "a\nb\n");
        commit(&repo, "B: grow foo"); // touches foo

        write(dir, "other.txt", "x\ny\n");
        commit(&repo, "C: only other"); // does NOT touch foo

        write(dir, "foo.txt", "a\nB\nc\n");
        commit(&repo, "D: edit + grow foo"); // touches foo

        let tl = timeline(&repo, "foo.txt").unwrap();

        // Exactly the three foo-touching commits, oldest → newest.
        let summaries: Vec<&str> = tl.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(
            summaries,
            ["A: add foo + other", "B: grow foo", "D: edit + grow foo"]
        );

        // Churn is populated: the first commit adds one line.
        assert_eq!(tl[0].insertions, 1);
        // D both edits a line and adds one → at least one insertion and one deletion.
        assert!(tl[2].insertions >= 1 && tl[2].deletions >= 1, "{:?}", tl[2]);

        // HEAD snapshot reflects the latest content and existence.
        let snap = head_snapshot(&repo, "foo.txt").unwrap();
        assert!(snap.existed);
        assert_eq!(&*snap.content, "a\nB\nc\n");

        // A file that never existed is a state, not an error.
        let missing = head_snapshot(&repo, "nope.txt").unwrap();
        assert!(!missing.existed);
        assert_eq!(missing.line_count(), 0);
    }
}
