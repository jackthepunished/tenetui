//! Repo-wide churn ranking — the "which files change the most?" scan that backs
//! the overview entry screen (docs/m5-plan.md "opera"). Walks a bounded window
//! of recent history so cold-open stays under the <1s target regardless of repo
//! age.

use anyhow::Result;
use git2::{DiffOptions, Patch, Repository};
use std::collections::HashMap;

/// How much one file has churned across the scanned window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChurn {
    pub path: String,
    /// Commits (in the window) that touched this file.
    pub touches: usize,
    /// Total insertions + deletions across those commits.
    pub churn: usize,
    /// Newest commit time (Unix seconds) that touched it.
    pub last_time: i64,
}

/// Rank files by churn across the last `max_commits` commits from HEAD, most
/// volatile first. Returns at most `top` entries.
///
/// Churn uses per-file line stats (a patch per changed file), which is the
/// expensive part; the `max_commits` bound keeps it cheap. If this ever proves
/// too slow on a very wide repo, ranking by `touches` alone (no patches) is the
/// documented fallback — see the plan.
pub fn volatility(repo: &Repository, max_commits: usize, top: usize) -> Result<Vec<FileChurn>> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    revwalk.set_sorting(git2::Sort::TIME)?;

    // path -> (touches, churn, last_time)
    let mut stats: HashMap<String, (usize, usize, i64)> = HashMap::new();

    for (i, oid) in revwalk.enumerate() {
        if i >= max_commits {
            break;
        }
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let time = commit.time().seconds();
        let tree = commit.tree()?;
        let parent_tree = match commit.parent(0) {
            Ok(p) => Some(p.tree()?),
            Err(_) => None,
        };

        let mut opts = DiffOptions::new();
        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;

        let count = diff.deltas().len();
        for idx in 0..count {
            let Ok(Some(patch)) = Patch::from_diff(&diff, idx) else {
                continue; // binary or unmodified delta
            };
            let delta = patch.delta();
            // Prefer the new path; a deletion has only the old one.
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .and_then(|p| p.to_str());
            let Some(path) = path else { continue };

            let (_ctx, adds, dels) = patch.line_stats()?;
            let entry = stats.entry(path.to_string()).or_insert((0, 0, time));
            entry.0 += 1;
            entry.1 += adds + dels;
            entry.2 = entry.2.max(time);
        }
    }

    let mut files: Vec<FileChurn> = stats
        .into_iter()
        .map(|(path, (touches, churn, last_time))| FileChurn {
            path,
            touches,
            churn,
            last_time,
        })
        .collect();

    // Most churn first; ties broken by touch count, then most-recently touched.
    files.sort_by(|a, b| {
        b.churn
            .cmp(&a.churn)
            .then(b.touches.cmp(&a.touches))
            .then(b.last_time.cmp(&a.last_time))
    });
    files.truncate(top);
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;
    use std::fs;

    fn commit(repo: &Repository, files: &[(&str, &str)], msg: &str) {
        let sig = Signature::now("Test", "t@e.st").unwrap();
        for (name, contents) in files {
            fs::write(repo.workdir().unwrap().join(name), contents).unwrap();
        }
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
            .unwrap();
    }

    #[test]
    fn ranks_the_most_churned_file_first() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();

        // hot.rs changes in every commit; cold.rs only once.
        commit(&repo, &[("hot.rs", "a\n"), ("cold.rs", "x\n")], "c1");
        commit(&repo, &[("hot.rs", "a\nb\n")], "c2");
        commit(&repo, &[("hot.rs", "a\nb\nc\n")], "c3");
        commit(&repo, &[("hot.rs", "a\nB\nc\nd\n")], "c4");

        let ranked = volatility(&repo, 100, 10).unwrap();
        assert_eq!(ranked[0].path, "hot.rs");
        assert!(ranked[0].touches >= 4);
        assert!(ranked[0].churn > ranked[1].churn);
        assert_eq!(ranked[1].path, "cold.rs");
    }

    #[test]
    fn respects_the_commit_window_and_top_n() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        for i in 0..5 {
            commit(&repo, &[("f.rs", &format!("line {i}\n"))], &format!("c{i}"));
        }
        // Only the most recent commit is in the window → 1 touch, not 5.
        let ranked = volatility(&repo, 1, 10).unwrap();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].touches, 1);

        // top = 0 yields nothing even though there is history.
        assert!(volatility(&repo, 100, 0).unwrap().is_empty());
    }
}
