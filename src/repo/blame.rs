//! Async blame: author + relative age per line, computed on a background thread
//! once scrubbing pauses — see docs/architecture.md "Blame" and "Threading".
//! A full `git2` blame is too slow to run per-frame, so the main loop only ever
//! reads results out of a channel; it never calls `blame_file` itself.

use git2::{BlameOptions, Oid, Repository};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

/// Author + age for one line, blamed *as of* a specific commit (not modern
/// HEAD) — see [`blame_at`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub author: String,
    /// Whole days between this line's introducing commit and the commit the
    /// blame was taken at.
    pub age_days: i64,
}

/// A short relative-age label ("today", "3d", "2w", "5mo", "1y") — the
/// conventional GitHub-style buckets, switching to years at the 365-day mark
/// rather than letting "mo" run all the way out to "24mo".
pub fn format_age(days: i64) -> String {
    match days {
        0 => "today".to_string(),
        1..=6 => format!("{days}d"),
        7..=29 => format!("{}w", days / 7),
        30..=364 => format!("{}mo", days / 30),
        _ => format!("{}y", days / 365),
    }
}

/// Blame `path` *as it existed at* `oid` — i.e. the walk stops at `oid`, so
/// lines are attributed relative to that point in history, not today. Returns
/// exactly `line_count` entries, one per line, so the file pane can index it
/// directly by line number.
pub fn blame_at(
    repo: &Repository,
    oid: Oid,
    path: &str,
    line_count: usize,
) -> anyhow::Result<Vec<BlameLine>> {
    let at_commit = repo.find_commit(oid)?;

    let mut opts = BlameOptions::new();
    opts.newest_commit(oid);
    let blame = repo.blame_file(Path::new(path), Some(&mut opts))?;

    let mut lines: Vec<Option<BlameLine>> = vec![None; line_count];
    for hunk in blame.iter() {
        let hunk_commit = repo.find_commit(hunk.final_commit_id())?;
        let author = hunk_commit.author().name().unwrap_or("unknown").to_string();
        let age_days =
            ((at_commit.time().seconds() - hunk_commit.time().seconds()) / 86_400).max(0);

        // libgit2 hunks are typically in ascending line order, but that's not a
        // documented guarantee — write by explicit index rather than assume it.
        let start = hunk.final_start_line().saturating_sub(1);
        for offset in 0..hunk.lines_in_hunk() {
            if let Some(slot) = lines.get_mut(start + offset) {
                *slot = Some(BlameLine {
                    author: author.clone(),
                    age_days,
                });
            }
        }
    }

    Ok(lines
        .into_iter()
        .map(|l| {
            l.unwrap_or_else(|| BlameLine {
                author: "?".to_string(),
                age_days: 0,
            })
        })
        .collect())
}

/// One blame request, tagged with a generation counter so the requester can
/// discard results superseded by a later move (docs/architecture.md: "stale
/// results, generation mismatch, are dropped on receipt").
pub struct BlameRequest {
    pub generation: u64,
    pub oid: Oid,
    pub line_count: usize,
}

pub struct BlameResult {
    pub generation: u64,
    pub lines: Vec<BlameLine>,
}

/// Run the blame worker loop: block for the next request, coalesce any that
/// queued up while busy (only the *latest* position matters — this is what
/// makes blame "compute once scrubbing pauses" without a separate debounce
/// timer), then blame and send. Ends when either channel closes.
pub fn run(
    repo_path: PathBuf,
    file_path: String,
    requests: Receiver<BlameRequest>,
    ready: Sender<BlameResult>,
) {
    let Ok(repo) = Repository::discover(&repo_path) else {
        return;
    };

    while let Ok(mut request) = requests.recv() {
        while let Ok(latest) = requests.try_recv() {
            request = latest;
        }
        if let Ok(lines) = blame_at(&repo, request.oid, &file_path, request.line_count) {
            let _ = ready.send(BlameResult {
                generation: request.generation,
                lines,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;
    use std::fs;

    fn commit(
        repo: &Repository,
        sig: &Signature,
        file: &str,
        contents: &str,
        message: &str,
        author_name: &str,
    ) -> Oid {
        fs::write(repo.workdir().unwrap().join(file), contents).unwrap();
        let author = Signature::now(author_name, "a@example.com").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        repo.commit(Some("HEAD"), &author, sig, message, &tree, &parents)
            .unwrap()
    }

    #[test]
    fn format_age_buckets_are_readable() {
        assert_eq!(format_age(0), "today");
        assert_eq!(format_age(3), "3d");
        assert_eq!(format_age(20), "2w");
        assert_eq!(format_age(90), "3mo");
        assert_eq!(format_age(400), "1y");
    }

    #[test]
    fn blame_attributes_each_line_to_its_introducing_author() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let sig = Signature::now("Committer", "c@example.com").unwrap();

        commit(&repo, &sig, "f.txt", "alice line\n", "first", "Alice");
        let bob_oid = commit(
            &repo,
            &sig,
            "f.txt",
            "alice line\nbob line\n",
            "second",
            "Bob",
        );

        let blame = blame_at(&repo, bob_oid, "f.txt", 2).unwrap();
        assert_eq!(blame[0].author, "Alice");
        assert_eq!(blame[1].author, "Bob");
    }

    #[test]
    fn blame_as_of_an_earlier_commit_ignores_later_authors() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let sig = Signature::now("Committer", "c@example.com").unwrap();

        let alice_oid = commit(&repo, &sig, "f.txt", "alice line\n", "first", "Alice");
        commit(
            &repo,
            &sig,
            "f.txt",
            "alice line\nbob line\n",
            "second",
            "Bob",
        );

        // Blaming as of the FIRST commit must not know about Bob's line at all.
        let blame = blame_at(&repo, alice_oid, "f.txt", 1).unwrap();
        assert_eq!(blame.len(), 1);
        assert_eq!(blame[0].author, "Alice");
    }

    #[test]
    fn worker_thread_discards_nothing_and_delivers_the_latest_request() {
        use std::sync::mpsc;
        use std::time::Duration;

        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let sig = Signature::now("Committer", "c@example.com").unwrap();
        let oid = commit(&repo, &sig, "f.txt", "line one\n", "first", "Alice");

        let (req_tx, req_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let repo_path = tmp.path().to_path_buf();
        let handle = std::thread::spawn(move || run(repo_path, "f.txt".into(), req_rx, ready_tx));

        req_tx
            .send(BlameRequest {
                generation: 7,
                oid,
                line_count: 1,
            })
            .unwrap();
        drop(req_tx);

        let result = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(result.generation, 7);
        assert_eq!(result.lines[0].author, "Alice");
        handle.join().unwrap();
    }
}
