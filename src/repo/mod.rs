//! All git2 access lives here. UI code consumes only the plain structs below —
//! it never touches a `git2` type. `Repository` is `!Sync`, so each thread that
//! needs one owns its own handle (see docs/architecture.md "Threading").

mod snapshot;
mod walk;

pub use snapshot::head_snapshot;
pub use walk::timeline;

use anyhow::{Context, Result, anyhow};
use git2::{Oid, Repository};
use std::path::Path;
use std::sync::Arc;

/// One commit that touched the target file. This list *is* the timeline.
#[derive(Clone, Debug)]
pub struct CommitMeta {
    pub oid: Oid,
    /// Commit time, seconds since the Unix epoch (UTC). Collected now (M0
    /// deliverable); rendered as a date in the M1 status bar.
    #[allow(dead_code)]
    pub time: i64,
    pub author: String,
    pub summary: String,
    pub insertions: usize,
    pub deletions: usize,
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
}

/// The file's content at one commit. `content` is `Arc<str>` so the snapshot
/// cache can share it without cloning the bytes.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The commit this snapshot came from — becomes the LRU cache key in M1.
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
