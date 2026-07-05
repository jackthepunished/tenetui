//! All git2 access lives here. UI code consumes only the plain structs below —
//! it never touches a `git2` type. `Repository` is `!Sync`, so each thread that
//! needs one owns its own handle (see docs/architecture.md "Threading").

pub mod blame;
pub mod prefetch;
mod snapshot;
mod walk;

pub use blame::BlameLine;
pub use snapshot::{SnapshotCache, head_snapshot};
pub use walk::timeline;

use anyhow::{Context, Result, anyhow};
use git2::{Oid, Repository};
use std::path::Path;
use std::sync::Arc;

/// One commit that touched the target file. This list *is* the timeline.
#[derive(Clone, Debug)]
pub struct CommitMeta {
    pub oid: Oid,
    /// Commit time, seconds since the Unix epoch (UTC).
    pub time: i64,
    pub author: String,
    pub summary: String,
    pub insertions: usize,
    pub deletions: usize,
    /// The file's path *as of this commit*. Constant for a file that was never
    /// moved; changes at a rename boundary as the walk follows the file back
    /// under its former name (see `walk::timeline`).
    pub path: String,
    /// More than one parent — a timeline landmark.
    pub is_merge: bool,
    /// Reachable from a tag (lightweight or annotated) — a timeline landmark.
    pub is_tagged: bool,
}

impl CommitMeta {
    /// Total churn — the heatmap intensity signal.
    pub fn churn(&self) -> usize {
        self.insertions + self.deletions
    }

    /// Short hex oid for display.
    pub fn short(&self) -> String {
        let s = self.oid.to_string();
        s[..s.len().min(7)].to_string()
    }

    /// `time` as a UTC calendar date (`YYYY-MM-DD`), for the status bar.
    ///
    /// No `chrono`/`time` dependency: this is Howard Hinnant's `civil_from_days`
    /// (public-domain, widely used), which needs only integer arithmetic.
    pub fn date(&self) -> String {
        let days = self.time.div_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        format!("{y:04}-{m:02}-{d:02}")
    }
}

/// Days-since-epoch (1970-01-01 = 0) → `(year, month, day)`, proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The file's content at one commit. `content` is `Arc<str>` so the snapshot
/// cache can share it without cloning the bytes.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The commit this snapshot came from — identifies the transition for the
    /// M2 diff-ghosting decay map.
    #[allow(dead_code)]
    pub oid: Oid,
    pub content: Arc<str>,
    /// `false` when the file did not exist at this commit — a *state*, not an
    /// error; the UI renders a "not yet created" placeholder.
    pub existed: bool,
}

impl Snapshot {
    /// Number of lines, for scroll bounds and the status bar.
    pub fn line_count(&self) -> usize {
        if !self.existed {
            0
        } else {
            self.content.lines().count().max(1)
        }
    }
}

/// Whether the repository has at least one commit (a born HEAD). `false` for a
/// freshly `git init`ed repo with nothing committed yet.
pub fn has_commits(repo: &Repository) -> bool {
    repo.head().is_ok()
}

/// Open the repository at `repo_path`, searching upward like `git` does.
/// Returns a friendly error if there's no repository there.
pub fn open(repo_path: &Path) -> Result<Repository> {
    Repository::discover(repo_path).map_err(|_| {
        anyhow!(
            "no git repository at {} (or any parent directory)",
            repo_path.display()
        )
    })
}

/// Normalize a user-supplied file path to a repository-relative path using
/// forward slashes (what libgit2 wants on every platform).
pub fn relative_path(repo: &Repository, file: &Path) -> Result<String> {
    let workdir = repo
        .workdir()
        .context("this is a bare repository; tenetui needs a working tree")?;

    let abs = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()?.join(file)
    };

    // If the file exists on disk, canonicalize and strip the workdir prefix.
    // Otherwise assume the user already gave a repo-relative path.
    let rel = match (abs.canonicalize(), workdir.canonicalize()) {
        (Ok(abs), Ok(root)) => abs
            .strip_prefix(&root)
            .map(|p| p.to_path_buf())
            .map_err(|_| {
                anyhow!(
                    "{} is not inside the repository at {}",
                    file.display(),
                    root.display()
                )
            })?,
        _ => file.to_path_buf(),
    };

    let slashed: String = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/");

    if slashed.is_empty() {
        return Err(anyhow!("empty file path"));
    }
    Ok(slashed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_at(time: i64) -> CommitMeta {
        CommitMeta {
            oid: Oid::zero(),
            time,
            author: "a".into(),
            summary: "s".into(),
            insertions: 0,
            deletions: 0,
            path: "f.txt".into(),
            is_merge: false,
            is_tagged: false,
        }
    }

    #[test]
    fn date_formats_known_epoch_seconds() {
        assert_eq!(meta_at(0).date(), "1970-01-01");
        assert_eq!(meta_at(86_400).date(), "1970-01-02");
        // 2000-02-29 12:00:00 UTC (a leap day, the classic calendar-math trap).
        assert_eq!(meta_at(951_825_600).date(), "2000-02-29");
        // 2026-07-05 00:00:00 UTC.
        assert_eq!(meta_at(1_783_209_600).date(), "2026-07-05");
    }

    #[test]
    fn date_handles_pre_epoch_time() {
        // A commit backdated to 1969-12-31 (negative unix time) must not panic
        // or produce a garbage year via truncating division.
        assert_eq!(meta_at(-3_600).date(), "1969-12-31");
    }
}
