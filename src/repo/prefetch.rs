//! Background snapshot warmer. Owns its *own* `git2::Repository` handle — never
//! the main thread's, since `Repository` is `!Sync` (see docs/architecture.md
//! "Threading") — and materializes snapshots in a window around the latest
//! playhead hint, so scrubbing near the current position lands on a cache hit
//! by the time the user gets there.

use super::Snapshot;
use super::snapshot::snapshot_at;
use git2::{Oid, Repository};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

/// Commits warmed on each side of the hinted playhead (roadmap M2: "±20").
const WINDOW: usize = 20;

/// The inclusive index range to warm around `center`, clamped to `[0, len)`.
/// Empty (never iterated) when `len == 0`.
fn window(center: usize, len: usize) -> RangeInclusive<usize> {
    if len == 0 {
        // Deliberately empty (`1..=0` iterates zero times) — there's nothing to warm.
        #[allow(clippy::reversed_empty_ranges)]
        return 1..=0;
    }
    let lo = center.saturating_sub(WINDOW);
    let hi = (center + WINDOW).min(len - 1);
    lo..=hi
}

/// Run the warmer loop: block for the next playhead hint, coalesce any hints
/// that queued up while idle (only the *latest* position matters), then
/// materialize and send every snapshot in that window. Returns — ending the
/// thread — once the hint channel closes (main dropped its sender) or `ready`'s
/// receiver is gone (main exited); the caller never needs to join it.
pub fn run(
    repo_path: PathBuf,
    file_path: String,
    oids: Vec<Oid>,
    hints: Receiver<usize>,
    ready: Sender<(Oid, Snapshot)>,
) {
    let Ok(repo) = Repository::discover(&repo_path) else {
        return;
    };

    while let Ok(mut center) = hints.recv() {
        while let Ok(latest) = hints.try_recv() {
            center = latest;
        }
        for idx in window(center, oids.len()) {
            let oid = oids[idx];
            let Ok(snapshot) = snapshot_at(&repo, oid, &file_path) else {
                continue;
            };
            if ready.send((oid, snapshot)).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_clamps_to_the_valid_range() {
        assert_eq!(window(5, 100), 0..=25);
        assert_eq!(window(50, 100), 30..=70);
        assert_eq!(window(99, 100), 79..=99);
        assert_eq!(window(0, 1), 0..=0);
    }

    #[test]
    fn window_is_empty_for_an_empty_timeline() {
        assert!(window(0, 0).is_empty());
    }

    #[test]
    fn thread_delivers_every_snapshot_in_the_hinted_window() {
        use git2::Signature;
        use std::collections::HashSet;
        use std::fs;
        use std::sync::mpsc;
        use std::time::Duration;

        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();

        let mut oids = Vec::new();
        for i in 0..5 {
            fs::write(tmp.path().join("f.txt"), format!("v{i}\n")).unwrap();
            let mut index = repo.index().unwrap();
            index
                .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<_> = parent.iter().collect();
            let oid = repo
                .commit(Some("HEAD"), &sig, &sig, &format!("c{i}"), &tree, &parents)
                .unwrap();
            oids.push(oid);
        }

        let (hint_tx, hint_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let repo_path = tmp.path().to_path_buf();
        let handle =
            std::thread::spawn(move || run(repo_path, "f.txt".into(), oids, hint_rx, ready_tx));

        hint_tx.send(2).unwrap(); // window(2, 5) covers the whole 5-commit history
        drop(hint_tx); // let the loop's next recv() fail once this batch is served

        let mut received = HashSet::new();
        while let Ok((oid, _snapshot)) = ready_rx.recv_timeout(Duration::from_secs(2)) {
            received.insert(oid);
        }
        handle.join().unwrap();

        assert_eq!(received.len(), 5);
    }
}
