//! tenetui — scrub through a file's git history like a video timeline.
//!
//! The binary is a thin driver: parse args, load the timeline + HEAD snapshot,
//! set up/tear down the terminal, and run the one event loop. Everything it
//! coordinates lives in the `tenetui` library crate (see `lib.rs`). All git
//! access is in `repo::`, all mutation in `app::update`, all rendering in `ui::`.

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use git2::{Oid, Repository};
use ratatui::DefaultTerminal;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tenetui::app::{self, AppState, Direction};
use tenetui::config::Config;
use tenetui::input::{self, Action, Keymap, SearchAction};
use tenetui::repo::blame::{BlameRequest, BlameResult};
use tenetui::repo::{self, SnapshotCache};
use tenetui::syntax::{HighlightRequest, HighlightResult};
use tenetui::ui::overview::OverviewState;
use tenetui::{diff, functions, syntax, theme, ui};

/// Idle poll interval when nothing is playing — just often enough to stay
/// responsive to terminal resizes.
const IDLE_POLL: Duration = Duration::from_millis(250);

const ONE_DAY_SECS: i64 = 86_400;
const ONE_WEEK_SECS: i64 = 7 * ONE_DAY_SECS;

/// Overview scan window and list length — bounded so cold-open stays fast.
const OVERVIEW_MAX_COMMITS: usize = 500;
const OVERVIEW_TOP_N: usize = 200;

#[derive(Parser)]
#[command(
    name = "tenetui",
    version,
    about = "Scrub through a file's git history like a video timeline — forward and inverted."
)]
struct Cli {
    /// Path to the git repository (searched upward, like git).
    repo: PathBuf,
    /// File to scrub. Omit it to open the volatile-files overview and pick one.
    file: Option<PathBuf>,
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

    let config = Config::load();
    let mut keymap = Keymap::default();
    keymap.apply_overrides(&config.keybinds);
    let theme = theme::Theme::new();

    // Resolve the file up front (if given) so a bad path errors before we enter
    // raw mode. `None` → start at the overview.
    let initial = match &cli.file {
        Some(f) => Some(repo::relative_path(&repo, f)?),
        None => None,
    };
    drop(repo);

    // `ratatui::init` enables raw mode, enters the alternate screen, and installs
    // a panic hook that restores the terminal — so a crash never leaves it broken.
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, &cli.repo, &config, &keymap, theme, initial);
    ratatui::restore();
    result
}

/// How the overview screen was left.
enum OverviewExit {
    Quit,
    /// Open the player on this repo-relative path.
    Open(String),
}

/// Top-level screen loop. With a file it's just the player; without one it's the
/// overview, opening the player on the chosen file and returning to the overview
/// when that player quits (so you can pick another file). Each screen opens its
/// own `Repository` — cheap, and it keeps handles from outliving a screen.
fn run_app(
    terminal: &mut DefaultTerminal,
    repo_path: &Path,
    config: &Config,
    keymap: &Keymap,
    theme: theme::Theme,
    initial: Option<String>,
) -> Result<()> {
    if let Some(rel) = initial {
        return run_player(terminal, repo_path, config, keymap, theme, &rel);
    }
    loop {
        let repo = repo::open(repo_path)?;
        let exit = run_overview(terminal, &repo, theme)?;
        drop(repo);
        match exit {
            OverviewExit::Quit => return Ok(()),
            OverviewExit::Open(rel) => {
                run_player(terminal, repo_path, config, keymap, theme, &rel)?
            }
        }
    }
}

/// The volatile-files overview. Its own small input handling (not the player
/// keymap): `j`/`k` select, `Enter` opens, `q` quits.
fn run_overview(
    terminal: &mut DefaultTerminal,
    repo: &Repository,
    theme: theme::Theme,
) -> Result<OverviewExit> {
    let files = repo::volatility(repo, OVERVIEW_MAX_COMMITS, OVERVIEW_TOP_N)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut state = OverviewState::new(files, theme, now);

    loop {
        terminal
            .draw(|frame| ui::overview::render(frame, frame.area(), &state))
            .context("render failed")?;

        if event::poll(IDLE_POLL)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('c'))
                | (_, KeyCode::Char('q'))
                | (_, KeyCode::Esc) => return Ok(OverviewExit::Quit),
                (_, KeyCode::Char('j')) | (_, KeyCode::Down) => state.select_down(),
                (_, KeyCode::Char('k')) | (_, KeyCode::Up) => state.select_up(),
                (_, KeyCode::Char('g')) | (_, KeyCode::Home) => state.select_first(),
                (_, KeyCode::Char('G')) | (_, KeyCode::End) => state.select_last(),
                (_, KeyCode::Enter) => {
                    if let Some(path) = state.selected_path() {
                        return Ok(OverviewExit::Open(path.to_string()));
                    }
                }
                _ => {}
            }
        }
    }
}

/// Build the player state + engine for `rel` and run the event loop until quit.
fn run_player(
    terminal: &mut DefaultTerminal,
    repo_path: &Path,
    config: &Config,
    keymap: &Keymap,
    theme: theme::Theme,
    rel: &str,
) -> Result<()> {
    let repo = repo::open(repo_path)?;
    let timeline = repo::timeline(&repo, rel)?;
    let current = repo::head_snapshot(&repo, rel)?;

    if timeline.is_empty() && !current.existed {
        anyhow::bail!(
            "{rel} has no history and does not exist at HEAD — is it tracked in this repository?"
        );
    }

    let mut state = AppState::new(theme, rel.to_string(), timeline.clone(), current);
    state.speed_ms = config.speed_ms();
    let mut engine = Engine::spawn(
        repo,
        repo_path.to_path_buf(),
        timeline,
        state.focused().playhead,
        config.cache_size(),
    );

    // Kick off highlighting of the initial HEAD snapshot; it lands on an early
    // frame (~20ms later) via the async worker rather than blocking startup.
    engine.request_highlight(&state, 0);

    run(terminal, &mut engine, keymap, state)
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
    /// Generation guard for highlighting, per deck (pincer mode has two).
    highlight_generation: [u64; 2],
}

impl Engine {
    /// Open the cache, spawn the prefetch and blame threads (each given its own
    /// path so it can open an independent `Repository` — never the one `repo`
    /// here), and warm the snapshot window around `initial_playhead` immediately.
    fn spawn(
        repo: Repository,
        repo_path: PathBuf,
        timeline: Vec<repo::CommitMeta>,
        initial_playhead: usize,
        cache_size: usize,
    ) -> Self {
        let (hint_tx, hint_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (blame_req_tx, blame_req_rx) = mpsc::channel();
        let (blame_ready_tx, blame_ready_rx) = mpsc::channel();

        if !timeline.is_empty() {
            // Each entry carries the file's path *at that commit* so prefetch and
            // blame keep working across a rename.
            let entries: Vec<(Oid, String)> =
                timeline.iter().map(|c| (c.oid, c.path.clone())).collect();
            {
                let repo_path = repo_path.clone();
                thread::spawn(move || repo::prefetch::run(repo_path, entries, hint_rx, ready_tx));
            }
            thread::spawn(move || repo::blame::run(repo_path, blame_req_rx, blame_ready_tx));
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
            cache: SnapshotCache::new(cache_size),
            hints: hint_tx,
            ready: ready_rx,
            blame_requests: blame_req_tx,
            blame_ready: blame_ready_rx,
            blame_generation: 0,
            highlight_requests: hl_req_tx,
            highlight_ready: hl_ready_rx,
            highlight_generation: [0, 0],
        }
    }

    /// Request highlighting of `deck`'s current snapshot, bumping that deck's
    /// generation so a result for its old position is dropped on receipt. A
    /// no-op payload (an absent file) still bumps the generation so stale colors
    /// can't reappear.
    fn request_highlight(&mut self, state: &AppState, deck: usize) {
        let d = &state.decks[deck];
        let path = state
            .commit_at(deck)
            .map(|c| c.path.clone())
            .unwrap_or_else(|| state.file_path.clone());
        self.highlight_generation[deck] += 1;
        let _ = self.highlight_requests.send(HighlightRequest {
            generation: self.highlight_generation[deck],
            deck,
            content: d.current.content.clone(),
            path,
            theme: state.theme,
        });
    }

    /// Resolve one commit-step of scrubbing of the *focused* deck.
    fn scrub(&mut self, state: &mut AppState, forward: bool) -> Result<bool> {
        let deck = state.focus;
        let len = state.timeline.len();
        if len == 0 {
            return Ok(false);
        }
        let cur = state.decks[deck].playhead;
        let next = if forward {
            (cur + 1).min(len - 1)
        } else {
            cur.saturating_sub(1)
        };
        self.jump_to(state, deck, next)
    }

    /// Advance the temporal pincer one tick: the forward deck (0) steps toward
    /// the future, the inverted deck (1) toward the past. Returns whether either
    /// deck actually moved (so playback stops once both hit their ends).
    fn pincer_tick(&mut self, state: &mut AppState) -> Result<bool> {
        let len = state.timeline.len();
        if len == 0 {
            return Ok(false);
        }
        let next0 = (state.decks[0].playhead + 1).min(len - 1);
        let moved0 = self.jump_to(state, 0, next0)?;
        let next1 = state.decks[1].playhead.saturating_sub(1);
        let moved1 = self.jump_to(state, 1, next1)?;
        Ok(moved0 || moved1)
    }

    /// Move `deck`'s playhead to `next` (used by scrub, the pincer, `g`/`G`,
    /// day/week jumps, and search) — fetch its snapshot (cache hit or a git2
    /// tree lookup), diff against the outgoing snapshot for ghosting, hand the
    /// result to `app::set_playhead`, then re-arm the prefetch hint and (if this
    /// is the focused deck and blame is on) request fresh blame. This and the
    /// drain methods are the only places the event loop talks to git2. Returns
    /// whether the playhead actually moved.
    fn jump_to(&mut self, state: &mut AppState, deck: usize, next: usize) -> Result<bool> {
        let playhead = state.decks[deck].playhead;
        if state.timeline.is_empty() || next == playhead {
            return Ok(false);
        }

        let forward = next > playhead;
        let oid = state.timeline[next].oid;
        let path = state.timeline[next].path.clone();
        let old_content = state.decks[deck].current.content.clone();
        let snapshot = self.cache.get_or_fetch(&self.repo, oid, &path)?;
        let ghosts =
            diff::compute_ghosts(&old_content, &snapshot.content, &state.decks[deck].ghosts);

        // In pincer mode a deck's ghost hue is fixed by its role (0 forward/red,
        // 1 inverted/blue); otherwise it follows the movement direction.
        let direction = if state.pincer {
            if deck == 0 {
                Direction::Forward
            } else {
                Direction::Backward
            }
        } else if forward {
            Direction::Forward
        } else {
            Direction::Backward
        };

        app::set_playhead(state, deck, next, snapshot, ghosts, direction);
        // Show plain text immediately; the async worker colorizes a beat later.
        app::set_highlighted(state, deck, None);
        self.request_highlight(state, deck);
        // When scoped to a function, re-resolve its line range in the new
        // snapshot (a fast tree-sitter parse; `None` = absent at this commit).
        if let Some(name) = state.scope.clone() {
            let range = functions::range_of(&state.decks[deck].current.content, &path, &name);
            app::set_scope_range(state, deck, range);
        }
        let _ = self.hints.send(next);
        if deck == state.focus && state.blame_visible {
            self.request_blame(state);
        }
        Ok(true)
    }

    /// Send a fresh blame request for the *focused* deck's playhead, bumping the
    /// generation so any result still in flight for the old position is
    /// discarded on arrival instead of overwriting newer data.
    fn request_blame(&mut self, state: &AppState) {
        let Some(commit) = state.current_commit() else {
            return;
        };
        let (oid, path) = (commit.oid, commit.path.clone());
        let line_count = state.focused().current.line_count();
        self.blame_generation += 1;
        let _ = self.blame_requests.send(BlameRequest {
            generation: self.blame_generation,
            oid,
            path,
            line_count,
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

    /// Apply a completed highlight result to its deck if still current; drop
    /// stale ones (superseded move, or a deck that no longer exists after
    /// leaving pincer mode). Never blocks.
    fn drain_highlight(&mut self, state: &mut AppState) {
        while let Ok(result) = self.highlight_ready.try_recv() {
            if result.deck < state.decks.len()
                && result.generation == self.highlight_generation[result.deck]
            {
                app::set_highlighted(state, result.deck, result.highlighted);
            }
        }
    }
}

fn run(
    terminal: &mut DefaultTerminal,
    engine: &mut Engine,
    keymap: &Keymap,
    mut state: AppState,
) -> Result<()> {
    while !state.should_quit {
        engine.drain_prefetched();
        engine.drain_blame(&mut state);
        engine.drain_highlight(&mut state);
        app::ease_scroll(&mut state);

        terminal
            .draw(|frame| ui::draw(frame, &state, keymap))
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
                if state.help_visible {
                    // Modal: only `?` (ToggleHelp) or Esc closes it; the app's
                    // Esc→quit binding is suppressed while help is up.
                    if key.code == KeyCode::Esc
                        || keymap.action_for(key) == Some(Action::ToggleHelp)
                    {
                        app::update(&mut state, Action::ToggleHelp);
                    }
                } else if state.picker.is_some() {
                    handle_picker_key(&mut state, engine, key)?;
                } else if state.search.is_some() {
                    handle_search_key(&mut state, engine, key)?;
                } else if state.scope.is_some() && key.code == KeyCode::Esc {
                    // Esc leaves function scope (rather than quitting).
                    app::exit_scope(&mut state);
                } else if let Some(action) = keymap.action_for(key) {
                    handle_action(&mut state, engine, action)?;
                }
            }
        } else if state.playing {
            // A poll timeout with no key is the playback tick. In pincer mode
            // that advances both decks at once; otherwise the focused deck in
            // its own direction.
            let moved = if state.pincer {
                engine.pincer_tick(&mut state)?
            } else {
                let forward = state.focused().direction == Direction::Forward;
                engine.scrub(&mut state, forward)?
            };
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
            let (deck, target) = (state.focus, app::jump_target(state, true, ONE_DAY_SECS));
            engine.jump_to(state, deck, target)?;
        }
        Action::JumpDayBackward => {
            let (deck, target) = (state.focus, app::jump_target(state, false, ONE_DAY_SECS));
            engine.jump_to(state, deck, target)?;
        }
        Action::JumpWeekForward => {
            let (deck, target) = (state.focus, app::jump_target(state, true, ONE_WEEK_SECS));
            engine.jump_to(state, deck, target)?;
        }
        Action::JumpWeekBackward => {
            let (deck, target) = (state.focus, app::jump_target(state, false, ONE_WEEK_SECS));
            engine.jump_to(state, deck, target)?;
        }
        Action::JumpFirst => {
            let deck = state.focus;
            engine.jump_to(state, deck, 0)?;
        }
        Action::JumpLast => {
            let deck = state.focus;
            let last = state.timeline.len().saturating_sub(1);
            engine.jump_to(state, deck, last)?;
        }
        Action::ToggleBlame => {
            app::update(state, action);
            if state.blame_visible {
                engine.request_blame(state);
            }
        }
        Action::TogglePincer | Action::ToggleFocus => {
            app::update(state, action);
            // The deck set / focus changed: blame was cleared, so refresh it for
            // the (possibly new) focused deck. Highlighting rides along on the
            // cloned deck when entering pincer, so it needs no re-request.
            if state.blame_visible {
                engine.request_blame(state);
            }
        }
        Action::OpenFunctions => {
            // Parse the focused deck's snapshot and open the picker (a no-op if
            // no functions are found — unsupported file type or feature off).
            let content = state.focused().current.content.clone();
            let path = state.current_path().to_string();
            let fns = functions::functions_in(&content, &path);
            app::open_picker(state, fns);
        }
        other => app::update(state, other),
    }
    Ok(())
}

/// Modal key handling while the function picker is open: `j`/`k` move,
/// `Enter` scopes to the highlighted function, `Esc` cancels.
fn handle_picker_key(
    state: &mut AppState,
    engine: &mut Engine,
    key: event::KeyEvent,
) -> Result<()> {
    match key.code {
        KeyCode::Esc => app::close_picker(state),
        KeyCode::Char('j') | KeyCode::Down => app::picker_down(state),
        KeyCode::Char('k') | KeyCode::Up => app::picker_up(state),
        KeyCode::Enter => {
            if let Some(name) = state
                .picker
                .as_ref()
                .and_then(|p| p.selected_fn())
                .map(|f| f.name.clone())
            {
                enter_scope(state, name);
                // Blame lines shift when the pane is scoped; refresh if shown.
                if state.blame_visible {
                    engine.request_blame(state);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Enter function scope: resolve the function's line range in every deck's
/// current snapshot (tree-sitter parse — pure, no git2), then flip the mode on.
fn enter_scope(state: &mut AppState, name: String) {
    for deck in 0..state.decks.len() {
        let content = state.decks[deck].current.content.clone();
        let path = state
            .commit_at(deck)
            .map(|c| c.path.clone())
            .unwrap_or_else(|| state.file_path.clone());
        let range = functions::range_of(&content, &path, &name);
        app::set_scope_range(state, deck, range);
    }
    app::enter_scope(state, name);
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
                let deck = state.focus;
                engine.jump_to(state, deck, target)?;
            }
            app::search_cancel(state);
        }
    }
    Ok(())
}
