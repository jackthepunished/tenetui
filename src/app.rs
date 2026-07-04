//! The single source of truth. `draw()` reads `AppState`; only `update()` mutates
//! it. Keeping mutation here (and out of rendering) is what makes the UI a pure
//! function of state — see docs/decisions.md "Immediate-mode state model".

use crate::input::Action;
use crate::repo::{CommitMeta, Snapshot};
use crate::theme::Theme;

/// Everything the renderer needs for a frame. M0 holds a fixed playhead at HEAD;
/// scrubbing (moving the playhead + swapping the snapshot) lands in M1.
pub struct AppState {
    pub theme: Theme,
    /// Repo-relative path, for display.
    pub file_path: String,
    /// Every commit touching the file, oldest → newest. The timeline.
    pub timeline: Vec<CommitMeta>,
    /// Index into `timeline` of the "now" cursor. M0 pins it to HEAD (last).
    pub playhead: usize,
    /// The file as it existed at `playhead`.
    pub current: Snapshot,
    /// Vertical scroll offset into the file pane.
    pub scroll: u16,
    pub should_quit: bool,
}

impl AppState {
    pub fn new(
        theme: Theme,
        file_path: String,
        timeline: Vec<CommitMeta>,
        current: Snapshot,
    ) -> Self {
        let playhead = timeline.len().saturating_sub(1);
        AppState {
            theme,
            file_path,
            timeline,
            playhead,
            current,
            scroll: 0,
            should_quit: false,
        }
    }

    /// The commit at the playhead, if the timeline is non-empty.
    pub fn current_commit(&self) -> Option<&CommitMeta> {
        self.timeline.get(self.playhead)
    }

    /// Upper bound for `scroll` so we never scroll the file entirely off-screen.
    fn max_scroll(&self) -> u16 {
        u16::try_from(self.current.line_count().saturating_sub(1)).unwrap_or(u16::MAX)
    }
}

/// The one place state changes in response to an [`Action`].
pub fn update(state: &mut AppState, action: Action) {
    match action {
        Action::Quit => state.should_quit = true,
        Action::ScrollDown => {
            state.scroll = (state.scroll + 1).min(state.max_scroll());
        }
        Action::ScrollUp => {
            state.scroll = state.scroll.saturating_sub(1);
        }
        Action::Top => state.scroll = 0,
        Action::Bottom => state.scroll = state.max_scroll(),
    }
}
