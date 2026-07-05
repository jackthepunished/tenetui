//! History walk: collect every commit that touched the target file, oldest →
//! newest, following the file back through renames (a `git mv` no longer stops
//! history dead — see docs/m5-plan.md "rotas").

use super::CommitMeta;
use anyhow::Result;
use git2::{DiffFindOptions, DiffOptions, Oid, Repository, Tree};
use std::collections::HashSet;
use std::path::Path;

/// Walk history for `path` (repo-relative, forward slashes) and return the
/// commits that changed it, in chronological order (index 0 = oldest).
///
/// A commit "touched" the file when its blob oid differs from the first
/// parent's. The walk goes newest→oldest tracking the file's path *at each
/// point in history*: when the file appears in a commit but not in its parent,
/// rename detection checks whether it arrived via a `git mv`, and if so the
/// walk continues under the former name. Each `CommitMeta` records the path as
/// of that commit.
pub fn timeline(repo: &Repository, path: &str) -> Result<Vec<CommitMeta>> {
    let tagged = tagged_commit_oids(repo)?;

    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    // TOPOLOGICAL guarantees a child never precedes its parent even when commit
    // times tie (rebases, same-second commits); TIME breaks ties among unrelated
    // commits. We collect newest→oldest, then reverse to past → future.
    revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

    // The path we're currently following; changes going back across a rename.
    let mut tracked = path.to_string();

    let mut out = Vec::new();
    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let tree = commit.tree()?;

        let this_blob = entry_oid(&tree, &tracked);

        // Compare against the first parent only (linear view of history).
        let parent = commit.parent(0).ok();
        let parent_tree = match &parent {
            Some(p) => Some(p.tree()?),
            None => None,
        };
        let parent_blob = parent_tree.as_ref().and_then(|t| entry_oid(t, &tracked));

        // Unchanged relative to parent → this commit didn't touch the file.
        if this_blob == parent_blob {
            continue;
        }

        // The file changed at `tracked`. If it appears here but not in the
        // parent, it may have arrived via a rename — only then is the (more
        // expensive) full-tree rename detection worth running.
        let renamed_from = if this_blob.is_some() && parent_blob.is_none() {
            detect_rename(repo, parent_tree.as_ref(), &tree, &tracked)?
        } else {
            None
        };

        let (insertions, deletions) = match &renamed_from {
            // Across a rename, diff the old blob (under its former name) against
            // the new one, so a pure move reads as zero churn and a
            // move-plus-edit reports only the real line delta.
            Some(old) => churn_blobs(repo, parent_tree.as_ref(), old, &tree, &tracked)?,
            None => churn(repo, parent_tree.as_ref(), &tree, &tracked)?,
        };

        let author = commit.author();
        out.push(CommitMeta {
            oid,
            time: commit.time().seconds(),
            author: author.name().unwrap_or("unknown").to_string(),
            summary: commit.summary().unwrap_or("").to_string(),
            insertions,
            deletions,
            path: tracked.clone(),
            is_merge: commit.parent_count() > 1,
            is_tagged: tagged.contains(&oid),
        });

        // Follow the former name for all older commits.
        if let Some(old) = renamed_from {
            tracked = old;
        }
    }

    // revwalk TIME order is newest-first; the timeline reads past → future.
    out.reverse();
    Ok(out)
}

/// If `new_path` in `tree` arrived via a rename from `parent_tree`, return its
/// former path. Runs full-tree diff with similarity detection, so it's only
/// called on the commits where the file first appears going backward.
fn detect_rename(
    repo: &Repository,
    parent_tree: Option<&Tree>,
    tree: &Tree,
    new_path: &str,
) -> Result<Option<String>> {
    let Some(parent_tree) = parent_tree else {
        return Ok(None);
    };
    let mut diff = repo.diff_tree_to_tree(Some(parent_tree), Some(tree), None)?;
    let mut find = DiffFindOptions::new();
    find.renames(true);
    diff.find_similar(Some(&mut find))?;

    for delta in diff.deltas() {
        if delta.status() != git2::Delta::Renamed {
            continue;
        }
        let new_matches = delta
            .new_file()
            .path()
            .and_then(|p| p.to_str())
            .is_some_and(|p| p == new_path);
        if new_matches && let Some(old) = delta.old_file().path().and_then(|p| p.to_str()) {
            return Ok(Some(old.to_string()));
        }
    }
    Ok(None)
}

/// Line delta between two specific blobs, addressed by (tree, path) on each
/// side — used across a rename where the path differs between commit and parent.
fn churn_blobs(
    repo: &Repository,
    old_tree: Option<&Tree>,
    old_path: &str,
    new_tree: &Tree,
    new_path: &str,
) -> Result<(usize, usize)> {
    let old_blob = old_tree
        .and_then(|t| entry_oid(t, old_path))
        .and_then(|oid| repo.find_blob(oid).ok());
    let new_blob = entry_oid(new_tree, new_path).and_then(|oid| repo.find_blob(oid).ok());

    let (Some(old_blob), Some(new_blob)) = (old_blob, new_blob) else {
        return Ok((0, 0));
    };
    let patch = git2::Patch::from_blobs(
        &old_blob,
        Some(Path::new(old_path)),
        &new_blob,
        Some(Path::new(new_path)),
        None,
    )?;
    let (_context, insertions, deletions) = patch.line_stats()?;
    Ok((insertions, deletions))
}

/// Every commit reachable through a tag, lightweight or annotated (peeled down
/// to the commit itself) — used to mark timeline landmarks.
fn tagged_commit_oids(repo: &Repository) -> Result<HashSet<Oid>> {
    let mut set = HashSet::new();
    repo.tag_foreach(|oid, _name| {
        if let Ok(obj) = repo.find_object(oid, None)
            && let Ok(commit) = obj.peel_to_commit()
        {
            set.insert(commit.id());
        }
        true
    })?;
    Ok(set)
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

    /// Merge commits (>1 parent) and tagged commits should be flagged for the
    /// timeline's landmark markers — build a small diamond history and a tag by
    /// hand rather than through the linear-only `commit` helper above.
    #[test]
    fn merge_and_tag_commits_are_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let repo = Repository::init(dir).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();

        let write_commit = |parents: &[&git2::Commit], msg: &str| -> git2::Oid {
            let mut index = repo.index().unwrap();
            index
                .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            repo.commit(None, &sig, &sig, msg, &tree, parents).unwrap()
        };

        write(dir, "foo.txt", "a\n");
        let root_oid = write_commit(&[], "root");
        let root = repo.find_commit(root_oid).unwrap();
        repo.reference("refs/heads/master", root_oid, true, "init")
            .unwrap();

        write(dir, "foo.txt", "a\nb\n");
        let side_oid = write_commit(&[&root], "side branch edits foo");
        let side = repo.find_commit(side_oid).unwrap();

        // A lightweight tag on the side commit — should resolve straight to it.
        repo.tag_lightweight("v-side", side.as_object(), false)
            .unwrap();

        write(dir, "foo.txt", "a\nb\nc\n");
        let merge_oid = write_commit(&[&root, &side], "merge side into root");
        repo.reference("refs/heads/master", merge_oid, true, "merge")
            .unwrap();
        repo.set_head("refs/heads/master").unwrap();
        repo.checkout_head(None).unwrap();

        let tl = timeline(&repo, "foo.txt").unwrap();
        let by_summary = |s: &str| tl.iter().find(|c| c.summary == s).unwrap();

        assert!(!by_summary("root").is_merge);
        assert!(by_summary("side branch edits foo").is_tagged);
        assert!(!by_summary("side branch edits foo").is_merge);
        assert!(by_summary("merge side into root").is_merge);
        assert!(!by_summary("merge side into root").is_tagged);
    }

    /// The walk should cross a `git mv`: asking for the new name still surfaces
    /// the pre-rename history, each commit carrying the path it had back then.
    #[test]
    fn timeline_follows_a_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let repo = Repository::init(dir).unwrap();

        write(dir, "old.txt", "line1\nline2\nline3\n");
        commit(&repo, "add old.txt");

        // Rename old.txt -> new.txt with a small edit (a pure move would be
        // detected too, but the edit makes the churn assertions meaningful).
        fs::remove_file(dir.join("old.txt")).unwrap();
        write(dir, "new.txt", "line1\nline2 edited\nline3\nline4\n");
        let sig = Signature::now("Test", "test@example.com").unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("old.txt")).unwrap();
        index.add_path(Path::new("new.txt")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "move old.txt to new.txt",
            &tree,
            &[&parent],
        )
        .unwrap();

        let tl = timeline(&repo, "new.txt").unwrap();

        // Both commits survive the move, oldest → newest, each under its
        // then-current name.
        assert_eq!(tl.len(), 2, "{tl:?}");
        assert_eq!(tl[0].summary, "add old.txt");
        assert_eq!(tl[0].path, "old.txt");
        assert_eq!(tl[1].summary, "move old.txt to new.txt");
        assert_eq!(tl[1].path, "new.txt");

        // The rename commit's churn reflects only the real edit (line 2 changed,
        // line 4 added), not a full add of the whole file.
        assert!(tl[1].insertions >= 1 && tl[1].deletions >= 1, "{:?}", tl[1]);
        assert!(
            tl[1].insertions <= 3,
            "move should not read as a full re-add: {:?}",
            tl[1]
        );

        // old.txt is genuinely gone at HEAD — the walk crossed a real rename,
        // it didn't just track a still-present old file.
        let snap = head_snapshot(&repo, &tl[0].path).unwrap();
        assert!(!snap.existed, "old.txt should not exist at HEAD");
    }
}
