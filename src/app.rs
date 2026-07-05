//! The single source of truth. `draw()` reads `AppState`; only `update()` (and
//! [`set_playhead`]/[`ease_scroll`], for the scrub and auto-follow paths) mutate
//! it. Keeping mutation here — and out of rendering — is what makes the UI a
//! pure function of state; see docs/decisions.md "Immediate-mode state model".

use crate::diff;
use crate::input::Action;
use crate::repo::{BlameLine, CommitMeta, Snapshot};
use crate::syntax::Highlighted;
use crate::theme::Theme;
use std::collections::HashMap;

/// Default playback cadence: one commit every 250ms.
const DEFAULT_TICK_MS: u64 = 250;
const MIN_TICK_MS: u64 = 30;
const MAX_TICK_MS: u64 = 2000;

/// Lines of context kept above a freshly changed region when auto-scroll follows it.
const FOLLOW_MARGIN: usize = 3;

/// Which way the playhead last moved — sets the ghost-glow hue (forward = red,
/// inverted = blue) and which way playback continues when `space` is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

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
    /// Lines changed by the last transition, decaying toward zero. Keyed by
    /// 0-indexed line number in `current`. See `diff::compute_ghosts`.
    pub ghosts: HashMap<usize, u8>,
    /// Which way the playhead last moved; sets ghost hue and playback direction.
    pub direction: Direction,
    pub playing: bool,
    /// Milliseconds per commit during playback.
    pub speed_ms: u64,
    /// Vertical scroll offset into the file pane, as currently rendered.
    pub scroll: u16,
    /// Where `scroll` is easing toward — set by a transition's changed region,
    /// or snapped to `scroll` itself by manual scrolling. See [`ease_scroll`].
    pub scroll_target: u16,
    /// Syntax highlighting for `current`, one entry per line, or `None` when the
    /// file type is unknown or the snapshot doesn't exist. Recomputed off the
    /// render path whenever the playhead moves (see `main`'s `jump_to`).
    pub highlighted: Option<Highlighted>,
    /// Whether the blame gutter is toggled on.
    pub blame_visible: bool,
    /// Author + age per line at `playhead`, once the background blame worker
    /// delivers it. `None` while hidden, still computing, or just invalidated
    /// by a move (see docs/architecture.md "Blame": invalidated on move).
    pub blame: Option<Vec<BlameLine>>,
    /// `Some(query)` while in `/`-search mode; typed characters build the query
    /// in place. `None` in normal navigation mode.
    pub search: Option<String>,
    /// Whether the help overlay is showing (modal).
    pub help_visible: bool,
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
            ghosts: HashMap::new(),
            direction: Direction::Forward,
            playing: false,
            speed_ms: DEFAULT_TICK_MS,
            scroll: 0,
            scroll_target: 0,
            highlighted: None,
            blame_visible: false,
            blame: None,
            search: None,
            help_visible: false,
            should_quit: false,
        }
    }

    /// The commit at the playhead, if the timeline is non-empty.
    pub fn current_commit(&self) -> Option<&CommitMeta> {
        self.timeline.get(self.playhead)
    }

    /// The file's path *at the playhead* — the same as `file_path` unless the
    /// playhead has scrubbed back across a rename, in which case it's the older
    /// name. Used for the header and for fetching content/blame/highlighting.
    pub fn current_path(&self) -> &str {
        self.current_commit()
            .map(|c| c.path.as_str())
            .unwrap_or(&self.file_path)
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
            state.scroll_target = state.scroll; // manual scroll cancels auto-follow
        }
        Action::ScrollUp => {
            state.scroll = state.scroll.saturating_sub(1);
            state.scroll_target = state.scroll;
        }
        Action::Top => {
            state.scroll = 0;
            state.scroll_target = 0;
        }
        Action::Bottom => {
            state.scroll = state.max_scroll();
            state.scroll_target = state.scroll;
        }
        // Handled by the caller instead — see the ScrubForward/ScrubBackward doc
        // comment above; the same reasoning applies to every jump/blame variant
        // below that needs a git2 fetch (jumps) or a channel send (blame).
        Action::ScrubBackward
        | Action::ScrubForward
        | Action::JumpDayForward
        | Action::JumpDayBackward
        | Action::JumpWeekForward
        | Action::JumpWeekBackward
        | Action::JumpFirst
        | Action::JumpLast => {}
        Action::TogglePlayback => state.playing = !state.playing,
        Action::SpeedUp => {
            state.speed_ms = ((state.speed_ms as f32 * 0.75) as u64).max(MIN_TICK_MS);
        }
        Action::SpeedDown => {
            state.speed_ms = ((state.speed_ms as f32 * 1.34) as u64).min(MAX_TICK_MS);
        }
        Action::ToggleBlame => {
            state.blame_visible = !state.blame_visible;
            if !state.blame_visible {
                state.blame = None;
            }
        }
        Action::SearchStart => search_start(state),
        Action::ToggleHelp => state.help_visible = !state.help_visible,
    }
}

/// Move the playhead to `index`, swap in the snapshot and ghost map already
/// resolved for the transition, and re-aim the auto-scroll follow target at
/// whatever just changed. Pure assignment — no I/O here; the fetch and diff
/// happen in the caller before this is invoked.
///
/// `scroll` itself is left untouched so [`ease_scroll`] can animate toward the
/// new target across the next few frames instead of snapping.
pub fn set_playhead(
    state: &mut AppState,
    index: usize,
    snapshot: Snapshot,
    ghosts: HashMap<usize, u8>,
    direction: Direction,
) {
    debug_assert!(index < state.timeline.len().max(1));
    state.playhead = index;
    state.current = snapshot;
    state.direction = direction;
    // Invalidated on move: the old blame describes the wrong commit now. The
    // caller re-requests it (see `Engine::jump_to`) if the gutter is visible.
    state.blame = None;
    if let Some(top) = diff::freshest_changed_line(&ghosts) {
        state.scroll_target = u16::try_from(top.saturating_sub(FOLLOW_MARGIN))
            .unwrap_or(u16::MAX)
            .min(state.max_scroll());
    }
    state.ghosts = ghosts;
}

/// Merge in a blame result the background worker finished computing. The
/// caller (`Engine::drain_blame`) has already checked the generation still
/// matches the latest request before calling this.
pub fn set_blame(state: &mut AppState, lines: Vec<BlameLine>) {
    state.blame = Some(lines);
}

/// Replace the current snapshot's syntax highlighting. Computed by the caller
/// (`main`) since it needs the `Highlighter`; kept out of `set_playhead` so the
/// two are independently testable and highlighting can be swapped/disabled.
pub fn set_highlighted(state: &mut AppState, highlighted: Option<Highlighted>) {
    state.highlighted = highlighted;
}

/// Enter `/`-search mode with an empty query.
pub fn search_start(state: &mut AppState) {
    state.search = Some(String::new());
}

pub fn search_type(state: &mut AppState, c: char) {
    if let Some(query) = &mut state.search {
        query.push(c);
    }
}

pub fn search_backspace(state: &mut AppState) {
    if let Some(query) = &mut state.search {
        query.pop();
    }
}

/// Leave search mode without moving the playhead.
pub fn search_cancel(state: &mut AppState) {
    state.search = None;
}

/// Case-insensitive subsequence match: every character of `query`, in order,
/// appears somewhere in `text` (not necessarily contiguous) — the same notion
/// of "fuzzy" fzf's basic mode uses.
fn fuzzy_matches(query: &str, text: &str) -> bool {
    let haystack = text.to_lowercase();
    let mut chars = haystack.chars();
    query
        .to_lowercase()
        .chars()
        .all(|qc| chars.any(|tc| tc == qc))
}

/// The nearest commit *after* the playhead (wrapping around to the start)
/// whose summary fuzzy-matches the current search query — vim's "search
/// forward, wrap" behavior. `None` if there's no query, no match, or an empty
/// timeline.
pub fn search_target(state: &AppState) -> Option<usize> {
    let query = state.search.as_ref()?;
    if query.is_empty() {
        return None;
    }
    let n = state.timeline.len();
    if n == 0 {
        return None;
    }
    (1..=n)
        .map(|offset| (state.playhead + offset) % n)
        .find(|&i| fuzzy_matches(query, &state.timeline[i].summary))
}

/// The nearest commit at least `seconds` away from the playhead's commit time,
/// in the given direction, clamped to the ends of history — the `w`/`b`
/// (day) and `{`/`}` (week) jump motions.
pub fn jump_target(state: &AppState, forward: bool, seconds: i64) -> usize {
    let Some(current_time) = state.current_commit().map(|c| c.time) else {
        return state.playhead;
    };
    let n = state.timeline.len();
    if forward {
        let target_time = current_time + seconds;
        (state.playhead + 1..n)
            .find(|&i| state.timeline[i].time >= target_time)
            .unwrap_or_else(|| n.saturating_sub(1))
    } else {
        let target_time = current_time - seconds;
        (0..state.playhead)
            .rev()
            .find(|&i| state.timeline[i].time <= target_time)
            .unwrap_or(0)
    }
}

/// Nudge `scroll` a fraction of the way toward `scroll_target`, called once per
/// frame regardless of what triggered it — this is the "eases toward... rather
/// than snapping" auto-scroll motion from the whitepaper. Pure arithmetic, safe
/// to call unconditionally; a no-op once `scroll == scroll_target`.
pub fn ease_scroll(state: &mut AppState) {
    if state.scroll == state.scroll_target {
        return;
    }
    let diff = i32::from(state.scroll_target) - i32::from(state.scroll);
    let step = (diff.unsigned_abs() / 3).max(1);
    if diff > 0 {
        state.scroll = state
            .scroll
            .saturating_add(step as u16)
            .min(state.scroll_target);
    } else {
        state.scroll = state
            .scroll
            .saturating_sub(step as u16)
            .max(state.scroll_target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ColorDepth, Theme};
    use git2::Oid;

    fn commit(summary: &str) -> CommitMeta {
        commit_at(summary, 0)
    }

    fn commit_at(summary: &str, time: i64) -> CommitMeta {
        CommitMeta {
            oid: Oid::zero(),
            time,
            author: "a".into(),
            summary: summary.into(),
            insertions: 0,
            deletions: 0,
            path: "f.txt".into(),
            is_merge: false,
            is_tagged: false,
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
    fn manual_scroll_cancels_the_auto_follow_target() {
        let mut state = state_with(vec![commit("A")]);
        state.scroll_target = 2; // pretend a transition is easing us toward line 2
        update(&mut state, Action::ScrollUp);
        // Manual input takes over immediately; nothing should keep easing past it.
        assert_eq!(state.scroll_target, state.scroll);
    }

    #[test]
    fn set_playhead_swaps_snapshot_and_ghosts_but_leaves_scroll_for_easing() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        state.scroll = 5;

        set_playhead(
            &mut state,
            0,
            snapshot("only one line\n"),
            HashMap::new(),
            Direction::Backward,
        );

        assert_eq!(state.playhead, 0);
        assert_eq!(&*state.current.content, "only one line\n");
        assert_eq!(state.direction, Direction::Backward);
        // No fresh ghosts this transition, so the follow target doesn't move...
        assert_eq!(state.scroll_target, 0);
        // ...and `scroll` itself is untouched, left for `ease_scroll` to animate.
        assert_eq!(state.scroll, 5);
    }

    #[test]
    fn set_playhead_aims_the_follow_target_at_the_freshest_change() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        let ghosts = HashMap::from([(10usize, diff::GHOST_MAX_DECAY)]);

        set_playhead(
            &mut state,
            1,
            snapshot("l\n".repeat(20).as_str()),
            ghosts,
            Direction::Forward,
        );

        assert_eq!(state.scroll_target, 7); // 10 - FOLLOW_MARGIN(3)
    }

    #[test]
    fn ease_scroll_converges_without_overshoot() {
        let mut state = state_with(vec![commit("A")]);
        state.scroll = 0;
        state.scroll_target = 10;

        let mut steps = 0;
        while state.scroll != state.scroll_target {
            ease_scroll(&mut state);
            steps += 1;
            assert!(steps < 100, "ease_scroll never converged");
        }
        assert_eq!(state.scroll, 10);

        // Calling again once converged must be a no-op, not oscillate.
        ease_scroll(&mut state);
        assert_eq!(state.scroll, 10);
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

    #[test]
    fn toggle_playback_flips_the_flag() {
        let mut state = state_with(vec![commit("A")]);
        assert!(!state.playing);
        update(&mut state, Action::TogglePlayback);
        assert!(state.playing);
        update(&mut state, Action::TogglePlayback);
        assert!(!state.playing);
    }

    #[test]
    fn speed_up_and_down_stay_within_bounds() {
        let mut state = state_with(vec![commit("A")]);
        assert_eq!(state.speed_ms, DEFAULT_TICK_MS);

        for _ in 0..50 {
            update(&mut state, Action::SpeedUp);
        }
        assert_eq!(state.speed_ms, MIN_TICK_MS);

        for _ in 0..50 {
            update(&mut state, Action::SpeedDown);
        }
        assert_eq!(state.speed_ms, MAX_TICK_MS);
    }

    #[test]
    fn toggle_blame_off_clears_any_cached_result() {
        let mut state = state_with(vec![commit("A")]);
        state.blame = Some(vec![BlameLine {
            author: "a".into(),
            age_days: 0,
        }]);
        update(&mut state, Action::ToggleBlame); // false -> true: blame_visible flips, cache untouched
        assert!(state.blame_visible);
        assert!(state.blame.is_some());

        update(&mut state, Action::ToggleBlame); // true -> false: hiding clears the cache too
        assert!(!state.blame_visible);
        assert!(state.blame.is_none());
    }

    #[test]
    fn set_playhead_always_invalidates_blame() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        state.blame = Some(vec![]);
        set_playhead(
            &mut state,
            1,
            snapshot("x\n"),
            HashMap::new(),
            Direction::Forward,
        );
        assert!(state.blame.is_none());
    }

    #[test]
    fn search_mode_types_and_cancels() {
        let mut state = state_with(vec![commit("A")]);
        assert!(state.search.is_none());

        search_start(&mut state);
        search_type(&mut state, 'f');
        search_type(&mut state, 'x');
        assert_eq!(state.search.as_deref(), Some("fx"));

        search_backspace(&mut state);
        assert_eq!(state.search.as_deref(), Some("f"));

        search_cancel(&mut state);
        assert!(state.search.is_none());
    }

    #[test]
    fn search_target_finds_nearest_fuzzy_match_forward_and_wraps() {
        let mut state = state_with(vec![
            commit("fix bug"),
            commit("add feature"),
            commit("refactor core"),
            commit("fix typo"),
        ]);
        state.playhead = 0;
        state.search = Some("fx".into()); // subsequence of "fix", not "add"/"refactor"

        // Nearest match strictly after the playhead: index 3 ("fix typo").
        assert_eq!(search_target(&state), Some(3));

        state.playhead = 3;
        // From the last index, wraps back around to index 0 ("fix bug").
        assert_eq!(search_target(&state), Some(0));
    }

    #[test]
    fn search_target_is_none_without_a_query_or_a_match() {
        let mut state = state_with(vec![commit("a"), commit("b")]);
        assert_eq!(search_target(&state), None); // no query yet

        state.search = Some("zzz".into());
        assert_eq!(search_target(&state), None); // no match

        state.search = Some(String::new());
        assert_eq!(search_target(&state), None); // empty query
    }

    #[test]
    fn jump_target_finds_the_nearest_commit_at_least_a_day_away() {
        let day = 86_400;
        let mut state = state_with(vec![
            commit_at("t0", 0),
            commit_at("t0.5", day / 2),
            commit_at("t1", day),
            commit_at("t2", 2 * day),
        ]);
        state.playhead = 0;
        assert_eq!(jump_target(&state, true, day), 2); // first commit >= +1 day

        state.playhead = 3; // time = 2*day; target = 2*day - day = day
        assert_eq!(jump_target(&state, false, day), 2); // "t1" is exactly 1 day back
    }

    #[test]
    fn jump_target_clamps_at_the_ends_of_history() {
        let state = state_with(vec![commit_at("only", 0)]);
        assert_eq!(jump_target(&state, true, 86_400), 0);
        assert_eq!(jump_target(&state, false, 86_400), 0);
    }
}
