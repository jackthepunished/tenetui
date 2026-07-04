//! tenetui — scrub through a file's git history like a video timeline.
//!
//! Top-level responsibilities only: parse args, load the timeline + HEAD snapshot,
//! set up/tear down the terminal, and run the one event loop. All git access is in
//! `repo::`, all mutation in `app::update`, all rendering in `ui::`.

mod app;
mod diff;
mod input;
mod repo;
mod theme;
mod ui;

use anyhow::{Context, Result};
use app::{AppState, Direction};
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use git2::{Oid, Repository};
use input::Action;
use ratatui::DefaultTerminal;
use repo::SnapshotCache;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

/// Snapshot cache capacity. Generously larger than the prefetch window (±20)
/// so ordinary back-and-forth scrubbing stays warm too.
const SNAPSHOT_CACHE_CAPACITY: usize = 256;

/// Idle poll interval when nothing is playing — just often enough to stay
/// responsive to terminal resizes.
const IDLE_POLL: Duration = Duration::from_millis(250);

#[derive(Parser)]
#[command(
    name = "tenetui",
    version,
    about = "Scrub through a file's git history like a video timeline — forward and inverted."
)]
struct Cli {
    /// Path to the git repository (searched upward, like git).
    repo: PathBuf,
    /// File within the repository to scrub through its history.
    file: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let repo = repo::open(&cli.repo)?;

    // Unborn HEAD → the repository has no commits; give a clean message instead
    // of a raw libgit2 error from the history walk.
    if !repo::has_commits(&repo) {
        anyhow::bail!(
            "{} has no commits yet — there's no history to scrub.",
            cli.repo.display()
        );
    }

    let rel = repo::relative_path(&repo, &cli.file)?;
    let timeline = repo::timeline(&repo, &rel)?;
    let current = repo::head_snapshot(&repo, &rel)?;

    if timeline.is_empty() && !current.existed {
        anyhow::bail!(
            "{rel} has no history and does not exist at HEAD — is it tracked in this repository?"
        );
    }

    let state = AppState::new(theme::Theme::new(), rel.clone(), timeline.clone(), current);
    let mut engine = Engine::spawn(repo, cli.repo.clone(), rel, timeline, state.playhead);

    // `ratatui::init` enables raw mode, enters the alternate screen, and installs
    // a panic hook that restores the terminal — so a crash never leaves it broken.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut engine, state);
    ratatui::restore();
    result
}

/// The scrub/playback machinery the event loop drives: the main thread's own
/// `Repository` handle plus cache for on-demand fetches, and the two ends of
/// the channel pair talking to the background prefetch thread (see
/// `repo::prefetch` and docs/architecture.md "Threading"). Bundled together so
/// `run`'s signature doesn't grow a parameter per concern.
struct Engine {
    repo: Repository,
    cache: SnapshotCache,
    hints: Sender<usize>,
    ready: Receiver<(Oid, repo::Snapshot)>,
}

impl Engine {
    /// Open the cache, spawn the prefetch thread (given its own path so it can
    /// open an independent `Repository` — never the one `repo` here), and warm
    /// the window around `initial_playhead` immediately.
    fn spawn(
        repo: Repository,
        repo_path: PathBuf,
        file_path: String,
        timeline: Vec<repo::CommitMeta>,
        initial_playhead: usize,
    ) -> Self {
        let (hint_tx, hint_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        if !timeline.is_empty() {
            let oids: Vec<Oid> = timeline.iter().map(|c| c.oid).collect();
            thread::spawn(move || {
                repo::prefetch::run(repo_path, file_path, oids, hint_rx, ready_tx)
            });
            let _ = hint_tx.send(initial_playhead);
        }

        Engine {
            repo,
            cache: SnapshotCache::new(SNAPSHOT_CACHE_CAPACITY),
            hints: hint_tx,
            ready: ready_rx,
        }
    }

    /// Resolve one commit-step of scrubbing: pick the neighboring playhead
    /// index, fetch its snapshot (cache hit or a git2 tree lookup), diff it
    /// against the outgoing snapshot for ghosting, then hand the result to
    /// `app::set_playhead` as a plain assignment. This — and [`Self::drain_prefetched`]
    /// — are the only places the event loop talks to git2; `app`/`ui` never do.
    /// Returns whether the playhead actually moved (used to auto-pause playback
    /// at either end of history).
    fn scrub(&mut self, state: &mut AppState, forward: bool) -> Result<bool> {
        let len = state.timeline.len();
        if len == 0 {
            return Ok(false);
        }
        let next = if forward {
            (state.playhead + 1).min(len - 1)
        } else {
            state.playhead.saturating_sub(1)
        };
        if next == state.playhead {
            return Ok(false);
        }

        let oid = state.timeline[next].oid;
        let old_content = state.current.content.clone();
        let snapshot = self.cache.get_or_fetch(&self.repo, oid, &state.file_path)?;
        let ghosts = diff::compute_ghosts(&old_content, &snapshot.content, &state.ghosts);
        let direction = if forward {
            Direction::Forward
        } else {
            Direction::Backward
        };

        app::set_playhead(state, next, snapshot, ghosts, direction);
        let _ = self.hints.send(next);
        Ok(true)
    }

    /// Merge any snapshots the background thread has finished materializing
    /// since we last checked. Never blocks.
    fn drain_prefetched(&mut self) {
        while let Ok((oid, snapshot)) = self.ready.try_recv() {
            self.cache.insert(oid, snapshot);
        }
    }
}

fn run(terminal: &mut DefaultTerminal, engine: &mut Engine, mut state: AppState) -> Result<()> {
    while !state.should_quit {
        engine.drain_prefetched();
        app::ease_scroll(&mut state);

        terminal
            .draw(|frame| ui::draw(frame, &state))
            .context("render failed")?;

        // While playing, the poll timeout doubles as the one tick source (see
        // docs/architecture.md "One event loop, one tick source"): a timeout
        // with no key event means it's time to advance one more commit.
        let poll_timeout = if state.playing {
            Duration::from_millis(state.speed_ms)
        } else {
            IDLE_POLL
        };

        if event::poll(poll_timeout)? {
            // Filter to key-down: crossterm reports press *and* release on
            // Windows, which would otherwise double-fire every binding.
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && let Some(action) = input::map_key(key)
            {
                match action {
                    Action::ScrubForward => {
                        engine.scrub(&mut state, true)?;
                    }
                    Action::ScrubBackward => {
                        engine.scrub(&mut state, false)?;
                    }
                    other => app::update(&mut state, other),
                }
            }
        } else if state.playing {
            let forward = state.direction == Direction::Forward;
            let moved = engine.scrub(&mut state, forward)?;
            if !moved {
                // Ran off the end (or start) of history — nothing left to play.
                state.playing = false;
            }
        }
    }
    Ok(())
}
