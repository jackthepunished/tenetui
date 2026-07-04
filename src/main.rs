//! tenetui — scrub through a file's git history like a video timeline.
//!
//! Top-level responsibilities only: parse args, load the timeline + HEAD snapshot,
//! set up/tear down the terminal, and run the one event loop. All git access is in
//! `repo::`, all mutation in `app::update`, all rendering in `ui::`.

mod app;
mod input;
mod repo;
mod theme;
mod ui;

use anyhow::{Context, Result};
use app::AppState;
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use git2::Repository;
use input::Action;
use ratatui::DefaultTerminal;
use repo::SnapshotCache;
use std::path::PathBuf;
use std::time::Duration;

/// Snapshot cache capacity. M1 has no background prefetch yet (that's M2's
/// ±20-around-the-playhead warmer), so this just needs to comfortably hold a
/// session's worth of manual back-and-forth scrubbing.
const SNAPSHOT_CACHE_CAPACITY: usize = 256;

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

    let state = AppState::new(theme::Theme::new(), rel, timeline, current);
    let cache = SnapshotCache::new(SNAPSHOT_CACHE_CAPACITY);

    // `ratatui::init` enables raw mode, enters the alternate screen, and installs
    // a panic hook that restores the terminal — so a crash never leaves it broken.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &repo, cache, state);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut DefaultTerminal,
    repo: &Repository,
    mut cache: SnapshotCache,
    mut state: AppState,
) -> Result<()> {
    while !state.should_quit {
        terminal
            .draw(|frame| ui::draw(frame, &state))
            .context("render failed")?;

        // Idle wake keeps resizes responsive. The tighter playback tick is M2.
        // Filter to key-down: crossterm reports press *and* release on Windows,
        // which would otherwise double-fire every binding.
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(action) = input::map_key(key)
        {
            match action {
                Action::ScrubForward => scrub(&mut state, repo, &mut cache, true)?,
                Action::ScrubBackward => scrub(&mut state, repo, &mut cache, false)?,
                other => app::update(&mut state, other),
            }
        }
    }
    Ok(())
}

/// Resolve one commit-step of scrubbing: pick the neighboring playhead index,
/// fetch its snapshot (cache hit or a git2 tree lookup), then hand the result to
/// `app::set_playhead` as a plain assignment. This is the only place the event
/// loop talks to git2 — `app`/`ui` never do.
fn scrub(
    state: &mut AppState,
    repo: &Repository,
    cache: &mut SnapshotCache,
    forward: bool,
) -> Result<()> {
    let len = state.timeline.len();
    if len == 0 {
        return Ok(());
    }
    let next = if forward {
        (state.playhead + 1).min(len - 1)
    } else {
        state.playhead.saturating_sub(1)
    };
    if next == state.playhead {
        return Ok(());
    }
    let oid = state.timeline[next].oid;
    let snapshot = cache.get_or_fetch(repo, oid, &state.file_path)?;
    app::set_playhead(state, next, snapshot);
    Ok(())
}
