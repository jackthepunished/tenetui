//! tenetui — scrub through a file's git history like a video timeline.
//!
//! The binary is a thin driver: parse args, load the timeline + HEAD snapshot,
//! set up/tear down the terminal, and run the one event loop. Everything it
//! coordinates lives in the `tenetui` library crate (see `lib.rs`). All git
//! access is in `repo::`, all mutation in `app::update`, all rendering in `ui::`.

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use git2::{Oid, Repository};
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use tenetui::app::{self, AppState, Direction};
use tenetui::input::{self, Action, SearchAction};
use tenetui::repo::blame::{BlameRequest, BlameResult};
use tenetui::repo::{self, SnapshotCache};
use tenetui::syntax::{HighlightRequest, HighlightResult};
use tenetui::{diff, syntax, theme, ui};

/// Snapshot cache capacity. Generously larger than the prefetch window (±20)
/// so ordinary back-and-forth scrubbing stays warm too.
const SNAPSHOT_CACHE_CAPACITY: usize = 256;

/// Idle poll interval when nothing is playing — just often enough to stay
/// responsive to terminal resizes.
const IDLE_POLL: Duration = Duration::from_millis(250);

const ONE_DAY_SECS: i64 = 86_400;
const ONE_WEEK_SECS: i64 = 7 * ONE_DAY_SECS;

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

    // Kick off highlighting of the initial HEAD snapshot; it lands on an early
    // frame (~20ms later) via the async worker rather than blocking startup.
    engine.request_highlight(&state);

    // `ratatui::init` enables raw mode, enters the alternate screen, and installs
    // a panic hook that restores the terminal — so a crash never leaves it broken.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut engine, state);
    ratatui::restore();
    result
}

/// The scrub/playback/blame machinery the event loop drives: the main thread's
/// own `Repository` handle plus cache for on-demand fetches, and the channel
/// pairs talking to the two background threads (prefetch, blame — see
/// `repo::prefetch`, `repo::blame`, and docs/architecture.md "Threading").
/// Bundled together so `run`'s signature doesn't grow a parameter per concern.
struct Engine {
    repo: Repository,
    cache: SnapshotCache,
    hints: Sender<usize>,
    ready: Receiver<(Oid, repo::Snapshot)>,
    blame_requests: Sender<BlameRequest>,
    blame_ready: Receiver<BlameResult>,
    /// Bumped on every request sent; a result whose generation doesn't match
    /// this was superseded by a later move and is dropped on receipt.
    blame_generation: u64,
    highlight_requests: Sender<HighlightRequest>,
    highlight_ready: Receiver<HighlightResult>,
    /// Generation guard for highlighting, same role as `blame_generation`.
    highlight_generation: u64,
}

impl Engine {
    /// Open the cache, spawn the prefetch and blame threads (each given its own
    /// path so it can open an independent `Repository` — never the one `repo`
    /// here), and warm the snapshot window around `initial_playhead` immediately.
    fn spawn(
        repo: Repository,
        repo_path: PathBuf,
        file_path: String,
        timeline: Vec<repo::CommitMeta>,
        initial_playhead: usize,
    ) -> Self {
        let (hint_tx, hint_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (blame_req_tx, blame_req_rx) = mpsc::channel();
        let (blame_ready_tx, blame_ready_rx) = mpsc::channel();

        if !timeline.is_empty() {
            let oids: Vec<Oid> = timeline.iter().map(|c| c.oid).collect();
            {
                let repo_path = repo_path.clone();
                let file_path = file_path.clone();
                thread::spawn(move || {
                    repo::prefetch::run(repo_path, file_path, oids, hint_rx, ready_tx)
                });
            }
            thread::spawn(move || {
                repo::blame::run(repo_path, file_path, blame_req_rx, blame_ready_tx)
            });
            let _ = hint_tx.send(initial_playhead);
        }

        // The highlight worker needs no repo — it works on snapshot content
        // alone — so it's spawned unconditionally (highlighting is useful even
        // with trivial history).
        let (hl_req_tx, hl_req_rx) = mpsc::channel();
        let (hl_ready_tx, hl_ready_rx) = mpsc::channel();
        thread::spawn(move || syntax::run(hl_req_rx, hl_ready_tx));

        Engine {
            repo,
            cache: SnapshotCache::new(SNAPSHOT_CACHE_CAPACITY),
            hints: hint_tx,
            ready: ready_rx,
            blame_requests: blame_req_tx,
            blame_ready: blame_ready_rx,
            blame_generation: 0,
            highlight_requests: hl_req_tx,
            highlight_ready: hl_ready_rx,
            highlight_generation: 0,
        }
    }

    /// Request highlighting of the current snapshot, bumping the generation so a
    /// result for the old position is dropped on receipt. A no-op payload (an
    /// absent file) still bumps the generation so stale colors can't reappear.
    fn request_highlight(&mut self, state: &AppState) {
        self.highlight_generation += 1;
        let _ = self.highlight_requests.send(HighlightRequest {
            generation: self.highlight_generation,
            content: state.current.content.clone(),
            path: state.file_path.clone(),
            theme: state.theme,
        });
    }

    /// Resolve one commit-step of scrubbing in the given direction.
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
        self.jump_to(state, next)
    }

    /// Move the playhead directly to `next` (used by scrub, `g`/`G`, day/week
    /// jumps, and search) — fetch its snapshot (cache hit or a git2 tree
    /// lookup), diff against the outgoing snapshot for ghosting, hand the
    /// result to `app::set_playhead`, then re-arm the prefetch hint and (if the
    /// gutter is visible) request fresh blame. This and [`Self::drain_prefetched`]/
    /// [`Self::drain_blame`] are the only places the event loop talks to git2;
    /// `app`/`ui` never do. Returns whether the playhead actually moved (used to
    /// auto-pause playback at either end of history).
    fn jump_to(&mut self, state: &mut AppState, next: usize) -> Result<bool> {
        if state.timeline.is_empty() || next == state.playhead {
            return Ok(false);
        }

        let forward = next > state.playhead;
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
        // Show plain text immediately; the async worker colorizes a beat later.
        app::set_highlighted(state, None);
        self.request_highlight(state);
        let _ = self.hints.send(next);
        if state.blame_visible {
            self.request_blame(state);
        }
        Ok(true)
    }

    /// Send a fresh blame request for the current playhead, bumping the
    /// generation so any result still in flight for the old position is
    /// discarded on arrival instead of overwriting newer data.
    fn request_blame(&mut self, state: &AppState) {
        let Some(commit) = state.current_commit() else {
            return;
        };
        self.blame_generation += 1;
        let _ = self.blame_requests.send(BlameRequest {
            generation: self.blame_generation,
            oid: commit.oid,
            line_count: state.current.line_count(),
        });
    }

    /// Merge any snapshots the background prefetch thread has finished
    /// materializing since we last checked. Never blocks.
    fn drain_prefetched(&mut self) {
        while let Ok((oid, snapshot)) = self.ready.try_recv() {
            self.cache.insert(oid, snapshot);
        }
    }

    /// Apply a completed blame result if it's still current; drop it silently
    /// otherwise (a later move already superseded it). Never blocks.
    fn drain_blame(&mut self, state: &mut AppState) {
        while let Ok(result) = self.blame_ready.try_recv() {
            if result.generation == self.blame_generation {
                app::set_blame(state, result.lines);
            }
        }
    }

    /// Apply a completed highlight result if it's still current; drop stale ones.
    /// Never blocks.
    fn drain_highlight(&mut self, state: &mut AppState) {
        while let Ok(result) = self.highlight_ready.try_recv() {
            if result.generation == self.highlight_generation {
                app::set_highlighted(state, result.highlighted);
            }
        }
    }
}

fn run(terminal: &mut DefaultTerminal, engine: &mut Engine, mut state: AppState) -> Result<()> {
    while !state.should_quit {
        engine.drain_prefetched();
        engine.drain_blame(&mut state);
        engine.drain_highlight(&mut state);
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
            {
                if state.search.is_some() {
                    handle_search_key(&mut state, engine, key)?;
                } else if let Some(action) = input::map_key(key) {
                    handle_action(&mut state, engine, action)?;
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

fn handle_action(state: &mut AppState, engine: &mut Engine, action: Action) -> Result<()> {
    match action {
        Action::ScrubForward => {
            engine.scrub(state, true)?;
        }
        Action::ScrubBackward => {
            engine.scrub(state, false)?;
        }
        Action::JumpDayForward => {
            let target = app::jump_target(state, true, ONE_DAY_SECS);
            engine.jump_to(state, target)?;
        }
        Action::JumpDayBackward => {
            let target = app::jump_target(state, false, ONE_DAY_SECS);
            engine.jump_to(state, target)?;
        }
        Action::JumpWeekForward => {
            let target = app::jump_target(state, true, ONE_WEEK_SECS);
            engine.jump_to(state, target)?;
        }
        Action::JumpWeekBackward => {
            let target = app::jump_target(state, false, ONE_WEEK_SECS);
            engine.jump_to(state, target)?;
        }
        Action::JumpFirst => {
            engine.jump_to(state, 0)?;
        }
        Action::JumpLast => {
            let last = state.timeline.len().saturating_sub(1);
            engine.jump_to(state, last)?;
        }
        Action::ToggleBlame => {
            app::update(state, action);
            if state.blame_visible {
                engine.request_blame(state);
            }
        }
        other => app::update(state, other),
    }
    Ok(())
}

fn handle_search_key(
    state: &mut AppState,
    engine: &mut Engine,
    key: event::KeyEvent,
) -> Result<()> {
    let Some(search_action) = input::map_search_key(key) else {
        return Ok(());
    };
    match search_action {
        SearchAction::Type(c) => app::search_type(state, c),
        SearchAction::Backspace => app::search_backspace(state),
        SearchAction::Cancel => app::search_cancel(state),
        SearchAction::Confirm => {
            if let Some(target) = app::search_target(state) {
                engine.jump_to(state, target)?;
            }
            app::search_cancel(state);
        }
    }
    Ok(())
}
