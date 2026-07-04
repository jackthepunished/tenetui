//! The single source of truth. `draw()` reads `AppState`; only `update()` (and
//! [`set_playhead`], for the scrub path) mutate it. Keeping mutation here — and
//! out of rendering — is what makes the UI a pure function of state; see
//! docs/decisions.md "Immediate-mode state model".

use crate::input::Action;
use crate::repo::{CommitMeta, Snapshot};
use crate::theme::Theme;

/// Everything the renderer needs for a frame.
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
///
/// `ScrubForward`/`ScrubBackward` are handled by the caller instead: resolving
/// them needs a snapshot fetch (git2 + the LRU cache), which only `main`'s event
/// loop has the `Repository` handle to do — see [`set_playhead`].
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
        Action::ScrubBackward | Action::ScrubForward => {}
    }
}

/// Move the playhead to `index` and swap in the snapshot already resolved for
/// it. Pure assignment — no I/O here, matching the rest of the update path;
/// the fetch happens in the caller before this is invoked.
pub fn set_playhead(state: &mut AppState, index: usize, snapshot: Snapshot) {
    debug_assert!(index < state.timeline.len().max(1));
    state.playhead = index;
    state.current = snapshot;
    state.scroll = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ColorDepth, Theme};
    use git2::Oid;

    fn commit(summary: &str) -> CommitMeta {
        CommitMeta {
            oid: Oid::zero(),
            time: 0,
            author: "a".into(),
            summary: summary.into(),
            insertions: 0,
            deletions: 0,
        }
    }

    fn snapshot(content: &str) -> Snapshot {
        Snapshot {
            oid: Oid::zero(),
            content: content.into(),
            existed: true,
        }
    }

    fn state_with(timeline: Vec<CommitMeta>) -> AppState {
        AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            timeline,
            snapshot("a\nb\nc\n"),
        )
    }

    #[test]
    fn new_pins_playhead_to_head() {
        let state = state_with(vec![commit("A"), commit("B"), commit("C")]);
        assert_eq!(state.playhead, 2);
        assert_eq!(state.current_commit().unwrap().summary, "C");
    }

    #[test]
    fn scroll_clamps_to_line_count_and_never_negative() {
        let mut state = state_with(vec![commit("A")]); // 3 lines of content
        for _ in 0..10 {
            update(&mut state, Action::ScrollDown);
        }
        assert_eq!(state.scroll, 2); // last line index, not out past it

        update(&mut state, Action::ScrollUp);
        assert_eq!(state.scroll, 1);

        for _ in 0..10 {
            update(&mut state, Action::ScrollUp);
        }
        assert_eq!(state.scroll, 0); // saturates, never wraps negative
    }

    #[test]
    fn set_playhead_swaps_snapshot_and_resets_scroll() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        update(&mut state, Action::Bottom); // move scroll off zero first
        assert_ne!(state.scroll, 0);

        set_playhead(&mut state, 0, snapshot("only one line\n"));

        assert_eq!(state.playhead, 0);
        assert_eq!(&*state.current.content, "only one line\n");
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn scrub_actions_are_a_no_op_in_update_by_design() {
        // The event loop intercepts these before they ever reach update(); this
        // guards against someone routing them here and silently dropping a scrub.
        let mut state = state_with(vec![commit("A")]);
        let before = state.playhead;
        update(&mut state, Action::ScrubForward);
        update(&mut state, Action::ScrubBackward);
        assert_eq!(state.playhead, before);
    }
}
