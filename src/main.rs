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
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::time::Duration;

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

    // `ratatui::init` enables raw mode, enters the alternate screen, and installs
    // a panic hook that restores the terminal — so a crash never leaves it broken.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, state);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, mut state: AppState) -> Result<()> {
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
            app::update(&mut state, action);
        }
    }
    Ok(())
}
