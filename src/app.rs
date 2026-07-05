//! The single source of truth. `draw()` reads `AppState`; only `update()` (and
//! [`set_playhead`]/[`ease_scroll`], for the scrub and auto-follow paths) mutate
//! it. Keeping mutation here — and out of rendering — is what makes the UI a
//! pure function of state; see docs/decisions.md "Immediate-mode state model".

use crate::diff::Ghosts;
use crate::functions::FunctionDef;
use crate::input::Action;
use crate::repo::{BlameLine, CommitMeta, Snapshot};
use crate::syntax::Highlighted;
use crate::theme::Theme;
use std::collections::{HashMap, VecDeque};

/// Default playback cadence: one commit every 250ms.
const DEFAULT_TICK_MS: u64 = 250;
const MIN_TICK_MS: u64 = 30;
const MAX_TICK_MS: u64 = 2000;

/// Lines of context kept above a freshly changed region when auto-scroll follows it.
const FOLLOW_MARGIN: usize = 3;

/// Cold-open animation length in ~33ms frames (the palindrome pincer reveal).
pub const INTRO_FRAMES: u8 = 24;
/// Turnstile flip length in frames — the brief hue wash when direction reverses.
pub const TURNSTILE_FRAMES: u8 = 4;
/// Timeline heat-echo: frames a just-left playhead column stays hot.
pub const HEAT_MAX: u8 = 10;
/// Map comet trail: how many recent playhead positions are remembered.
const TRAIL_LEN: usize = 8;

/// Which way the playhead last moved — sets the ghost-glow hue (forward = red,
/// inverted = blue) and which way playback continues when `space` is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// One playhead and everything that hangs off it. In normal use there's a
/// single deck; the temporal pincer (`sator`) activates a second one so the
/// forward-red and inverted-blue panes each have independent position, ghost
/// trail, scroll, and highlighting.
#[derive(Clone)]
pub struct Deck {
    /// Index into `AppState::timeline` of this deck's "now" cursor.
    pub playhead: usize,
    /// The file as it existed at `playhead`.
    pub current: Snapshot,
    /// Lines changed by the last transition, decaying toward zero, with the
    /// changed word ranges within each. See `diff::compute_ghosts`.
    pub ghosts: Ghosts,
    /// Which way this deck last moved; sets its ghost hue. In pincer mode each
    /// deck's direction is fixed by its role (deck 0 forward, deck 1 inverted).
    pub direction: Direction,
    /// Vertical scroll offset into the pane, as currently rendered.
    pub scroll: u16,
    /// Where `scroll` is easing toward. See [`ease_scroll`].
    pub scroll_target: u16,
    /// Syntax highlighting for `current`, one entry per line, or `None` when the
    /// file type is unknown, the snapshot is absent, or a move just cleared it.
    pub highlighted: Option<Highlighted>,
    /// When scoped to a function (`arepo`), that function's 0-indexed inclusive
    /// line range *in this deck's snapshot* — or `None` if it isn't present here.
    /// Only meaningful while `AppState::scope` is set.
    pub scope_range: Option<(usize, usize)>,
    /// Turnstile frames remaining — set when this deck's direction reverses; the
    /// pane briefly washes in the new direction's hue while it decays.
    pub turnstile: u8,
    /// Recent playhead positions, newest first — the map's comet trail.
    pub trail: VecDeque<usize>,
}

impl Deck {
    fn new(playhead: usize, current: Snapshot) -> Self {
        Deck {
            playhead,
            current,
            ghosts: Ghosts::default(),
            direction: Direction::Forward,
            scroll: 0,
            scroll_target: 0,
            highlighted: None,
            scope_range: None,
            turnstile: 0,
            trail: VecDeque::new(),
        }
    }

    /// Upper bound for `scroll` so we never scroll the file entirely off-screen.
    pub fn max_scroll(&self) -> u16 {
        u16::try_from(self.current.line_count().saturating_sub(1)).unwrap_or(u16::MAX)
    }

    /// A blank deck for widget tests in other modules.
    #[cfg(test)]
    pub fn new_for_test() -> Deck {
        Deck::new(
            0,
            Snapshot {
                oid: git2::Oid::zero(),
                content: "".into(),
                existed: true,
            },
        )
    }
}

/// Everything the renderer needs for a frame.
pub struct AppState {
    pub theme: Theme,
    /// Repo-relative path at HEAD, for display fallback and error messages.
    pub file_path: String,
    /// Every commit touching the file, oldest → newest. The timeline.
    pub timeline: Vec<CommitMeta>,
    /// One deck in normal mode; two in pincer mode (0 = forward, 1 = inverted).
    pub decks: Vec<Deck>,
    /// Which deck has keyboard focus (0 or 1). Manual scrub/scroll act on it,
    /// and blame follows it.
    pub focus: usize,
    /// Whether the temporal pincer (two decks) is active.
    pub pincer: bool,
    pub playing: bool,
    /// Milliseconds per commit during playback.
    pub speed_ms: u64,
    /// Whether the blame gutter is toggled on.
    pub blame_visible: bool,
    /// Author + age per line at the *focused* deck's playhead, once the
    /// background worker delivers it. `None` while hidden, computing, or just
    /// invalidated by a move (docs/architecture.md "Blame": invalidated on move).
    pub blame: Option<Vec<BlameLine>>,
    /// `Some(query)` while in `/`-search mode; typed characters build the query
    /// in place. `None` in normal navigation mode.
    pub search: Option<String>,
    /// Whether the help overlay is showing (modal).
    pub help_visible: bool,
    /// Whether the space-time map view is showing instead of the file pane(s).
    pub map_visible: bool,
    /// Cold-open frames remaining; any key skips it. 0 = not running (the
    /// default, so tests and mid-session states render normally).
    pub intro: u8,
    /// Timeline heat-echo: commit index → frames of afterglow remaining, seeded
    /// each time the playhead leaves a position.
    pub heat: HashMap<usize, u8>,
    /// `Some(name)` while scoped to a function (`arepo`): the file pane clamps to
    /// that function's range in each snapshot. `None` = whole-file view.
    pub scope: Option<String>,
    /// The function-picker modal (`F`), if open.
    pub picker: Option<FunctionPicker>,
    pub should_quit: bool,
}

/// The modal function picker opened by `F`: the functions found in the current
/// snapshot, and which one is highlighted.
pub struct FunctionPicker {
    pub functions: Vec<FunctionDef>,
    pub selected: usize,
}

impl FunctionPicker {
    pub fn selected_fn(&self) -> Option<&FunctionDef> {
        self.functions.get(self.selected)
    }
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
            decks: vec![Deck::new(playhead, current)],
            focus: 0,
            pincer: false,
            playing: false,
            speed_ms: DEFAULT_TICK_MS,
            blame_visible: false,
            blame: None,
            search: None,
            help_visible: false,
            map_visible: false,
            intro: 0,
            heat: HashMap::new(),
            scope: None,
            picker: None,
            should_quit: false,
        }
    }

    /// The deck with keyboard focus.
    pub fn focused(&self) -> &Deck {
        &self.decks[self.focus]
    }

    fn focused_mut(&mut self) -> &mut Deck {
        &mut self.decks[self.focus]
    }

    /// The commit at deck `i`'s playhead, if any.
    pub fn commit_at(&self, deck: usize) -> Option<&CommitMeta> {
        self.timeline.get(self.decks[deck].playhead)
    }

    /// The commit at the focused deck's playhead.
    pub fn current_commit(&self) -> Option<&CommitMeta> {
        self.commit_at(self.focus)
    }

    /// The file's path at the focused deck's playhead — the same as `file_path`
    /// unless scrubbed back across a rename, in which case it's the older name.
    pub fn current_path(&self) -> &str {
        self.current_commit()
            .map(|c| c.path.as_str())
            .unwrap_or(&self.file_path)
    }

    /// Whether any decaying animation is running — the event loop shortens its
    /// poll timeout to ~30ms while this is true so effects play smoothly.
    /// (The map trail is position-indexed, not timed, so it doesn't count.)
    pub fn animating(&self) -> bool {
        self.intro > 0 || !self.heat.is_empty() || self.decks.iter().any(|d| d.turnstile > 0)
    }

    /// Cold-open progress as a fade fraction (0 = just started, 1 ≈ done), or
    /// `None` when the intro isn't running.
    pub fn intro_fade(&self) -> Option<f32> {
        (self.intro > 0).then(|| 1.0 - f32::from(self.intro) / f32::from(INTRO_FRAMES))
    }
}

/// Advance every decaying animation by one frame. Called by the event loop on
/// its animation tick; pure state arithmetic.
pub fn anim_tick(state: &mut AppState) {
    state.intro = state.intro.saturating_sub(1);
    for deck in &mut state.decks {
        deck.turnstile = deck.turnstile.saturating_sub(1);
    }
    state.heat.retain(|_, v| {
        *v = v.saturating_sub(1);
        *v > 0
    });
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
            let max = state.focused().max_scroll();
            let d = state.focused_mut();
            d.scroll = (d.scroll + 1).min(max);
            d.scroll_target = d.scroll; // manual scroll cancels auto-follow
        }
        Action::ScrollUp => {
            let d = state.focused_mut();
            d.scroll = d.scroll.saturating_sub(1);
            d.scroll_target = d.scroll;
        }
        Action::Top => {
            let d = state.focused_mut();
            d.scroll = 0;
            d.scroll_target = 0;
        }
        Action::Bottom => {
            let max = state.focused().max_scroll();
            let d = state.focused_mut();
            d.scroll = max;
            d.scroll_target = max;
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
        Action::ToggleMap => state.map_visible = !state.map_visible,
        Action::TogglePincer => toggle_pincer(state),
        Action::ToggleFocus => toggle_focus(state),
        // Handled by the caller: opening the picker needs to parse the current
        // snapshot (the `functions` module), which lives outside `app`.
        Action::OpenFunctions => {}
    }
}

/// Enter or leave temporal pincer mode. Entering clones the focused deck into a
/// second one and pins the two roles (deck 0 = forward/red, deck 1 =
/// inverted/blue); leaving drops the second deck. Both start at the current
/// position so playback fans them apart from a shared "now".
pub fn toggle_pincer(state: &mut AppState) {
    if state.pincer {
        state.decks.truncate(1);
        state.pincer = false;
        state.focus = 0;
        state.decks[0].direction = Direction::Forward;
    } else {
        let mut second = state.decks[0].clone();
        second.direction = Direction::Backward;
        state.decks[0].direction = Direction::Forward;
        state.decks.push(second);
        state.pincer = true;
        state.focus = 0;
    }
    // Blame belongs to the focused deck; a mode change re-scopes it.
    state.blame = None;
}

/// Switch keyboard focus between the two decks (no-op outside pincer mode).
pub fn toggle_focus(state: &mut AppState) {
    if state.pincer {
        state.focus = 1 - state.focus;
        state.blame = None; // blame follows focus
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
    deck: usize,
    index: usize,
    snapshot: Snapshot,
    ghosts: Ghosts,
    direction: Direction,
) {
    debug_assert!(index < state.timeline.len().max(1));
    {
        let d = &mut state.decks[deck];
        // Motion bookkeeping: the position being left behind echoes on the
        // timeline (heat) and joins the map's comet trail; a direction reversal
        // arms the turnstile flip.
        state.heat.insert(d.playhead, HEAT_MAX);
        d.trail.push_front(d.playhead);
        d.trail.truncate(TRAIL_LEN);
        if d.direction != direction {
            d.turnstile = TURNSTILE_FRAMES;
        }
        d.playhead = index;
        d.current = snapshot;
        d.direction = direction;
        if let Some(top) = ghosts.freshest_changed_line() {
            d.scroll_target = u16::try_from(top.saturating_sub(FOLLOW_MARGIN))
                .unwrap_or(u16::MAX)
                .min(d.max_scroll());
        }
        d.ghosts = ghosts;
    }
    // Invalidated on move: the old blame describes the wrong commit now. Only
    // the focused deck's blame is shown; the caller re-requests it if visible.
    if deck == state.focus {
        state.blame = None;
    }
}

/// Merge in a blame result the background worker finished computing. The
/// caller (`Engine::drain_blame`) has already checked the generation still
/// matches the latest request before calling this.
pub fn set_blame(state: &mut AppState, lines: Vec<BlameLine>) {
    state.blame = Some(lines);
}

/// Replace a deck's syntax highlighting. Computed by the caller (`main`) since
/// it needs the `Highlighter`; kept out of `set_playhead` so the two are
/// independently testable and highlighting can be swapped/disabled.
pub fn set_highlighted(state: &mut AppState, deck: usize, highlighted: Option<Highlighted>) {
    state.decks[deck].highlighted = highlighted;
}

/// Open the function picker on `functions` (no-op if the list is empty — e.g. an
/// unsupported file type, or the `functions` feature is off).
pub fn open_picker(state: &mut AppState, functions: Vec<FunctionDef>) {
    if !functions.is_empty() {
        state.picker = Some(FunctionPicker {
            functions,
            selected: 0,
        });
    }
}

pub fn picker_down(state: &mut AppState) {
    if let Some(p) = &mut state.picker
        && p.selected + 1 < p.functions.len()
    {
        p.selected += 1;
    }
}

pub fn picker_up(state: &mut AppState) {
    if let Some(p) = &mut state.picker {
        p.selected = p.selected.saturating_sub(1);
    }
}

pub fn close_picker(state: &mut AppState) {
    state.picker = None;
}

/// Enter function scope by name and close the picker. The per-deck line ranges
/// (and any re-scroll) are resolved by the caller, which has the parser.
pub fn enter_scope(state: &mut AppState, name: String) {
    state.scope = Some(name);
    state.picker = None;
    for deck in &mut state.decks {
        deck.scroll = 0;
        deck.scroll_target = 0;
    }
}

/// Leave function scope; the file pane returns to the whole file.
pub fn exit_scope(state: &mut AppState) {
    state.scope = None;
    for deck in &mut state.decks {
        deck.scope_range = None;
    }
}

/// Record the resolved function range for `deck`'s current snapshot (`None` =
/// the function isn't present at this commit). Computed by the caller.
pub fn set_scope_range(state: &mut AppState, deck: usize, range: Option<(usize, usize)>) {
    state.decks[deck].scope_range = range;
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
    let playhead = state.focused().playhead;
    (1..=n)
        .map(|offset| (playhead + offset) % n)
        .find(|&i| fuzzy_matches(query, &state.timeline[i].summary))
}

/// The nearest commit at least `seconds` away from the playhead's commit time,
/// in the given direction, clamped to the ends of history — the `w`/`b`
/// (day) and `{`/`}` (week) jump motions.
pub fn jump_target(state: &AppState, forward: bool, seconds: i64) -> usize {
    let playhead = state.focused().playhead;
    let Some(current_time) = state.current_commit().map(|c| c.time) else {
        return playhead;
    };
    let n = state.timeline.len();
    if forward {
        let target_time = current_time + seconds;
        (playhead + 1..n)
            .find(|&i| state.timeline[i].time >= target_time)
            .unwrap_or_else(|| n.saturating_sub(1))
    } else {
        let target_time = current_time - seconds;
        (0..playhead)
            .rev()
            .find(|&i| state.timeline[i].time <= target_time)
            .unwrap_or(0)
    }
}

/// Nudge every deck's `scroll` a fraction of the way toward its `scroll_target`,
/// called once per frame regardless of what triggered it — the "eases toward...
/// rather than snapping" auto-scroll from the whitepaper. Pure arithmetic, safe
/// to call unconditionally; a no-op for a deck already at its target.
pub fn ease_scroll(state: &mut AppState) {
    for deck in &mut state.decks {
        if deck.scroll == deck.scroll_target {
            continue;
        }
        let diff = i32::from(deck.scroll_target) - i32::from(deck.scroll);
        let step = (diff.unsigned_abs() / 3).max(1);
        if diff > 0 {
            deck.scroll = deck
                .scroll
                .saturating_add(step as u16)
                .min(deck.scroll_target);
        } else {
            deck.scroll = deck
                .scroll
                .saturating_sub(step as u16)
                .max(deck.scroll_target);
        }
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
        assert_eq!(state.focused().playhead, 2);
        assert_eq!(state.current_commit().unwrap().summary, "C");
    }

    #[test]
    fn scroll_clamps_to_line_count_and_never_negative() {
        let mut state = state_with(vec![commit("A")]); // 3 lines of content
        for _ in 0..10 {
            update(&mut state, Action::ScrollDown);
        }
        assert_eq!(state.focused().scroll, 2); // last line index, not out past it

        update(&mut state, Action::ScrollUp);
        assert_eq!(state.focused().scroll, 1);

        for _ in 0..10 {
            update(&mut state, Action::ScrollUp);
        }
        assert_eq!(state.focused().scroll, 0); // saturates, never wraps negative
    }

    #[test]
    fn manual_scroll_cancels_the_auto_follow_target() {
        let mut state = state_with(vec![commit("A")]);
        state.decks[0].scroll_target = 2; // pretend a transition is easing toward line 2
        update(&mut state, Action::ScrollUp);
        // Manual input takes over immediately; nothing should keep easing past it.
        assert_eq!(state.focused().scroll_target, state.focused().scroll);
    }

    #[test]
    fn set_playhead_swaps_snapshot_and_ghosts_but_leaves_scroll_for_easing() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        state.decks[0].scroll = 5;

        set_playhead(
            &mut state,
            0,
            0,
            snapshot("only one line\n"),
            Ghosts::default(),
            Direction::Backward,
        );

        let d = state.focused();
        assert_eq!(d.playhead, 0);
        assert_eq!(&*d.current.content, "only one line\n");
        assert_eq!(d.direction, Direction::Backward);
        // No fresh ghosts this transition, so the follow target doesn't move...
        assert_eq!(d.scroll_target, 0);
        // ...and `scroll` itself is untouched, left for `ease_scroll` to animate.
        assert_eq!(d.scroll, 5);
    }

    #[test]
    fn set_playhead_aims_the_follow_target_at_the_freshest_change() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        let ghosts = Ghosts::from_decay(HashMap::from([(10usize, crate::diff::GHOST_MAX_DECAY)]));

        set_playhead(
            &mut state,
            0,
            1,
            snapshot("l\n".repeat(20).as_str()),
            ghosts,
            Direction::Forward,
        );

        assert_eq!(state.focused().scroll_target, 7); // 10 - FOLLOW_MARGIN(3)
    }

    #[test]
    fn ease_scroll_converges_without_overshoot() {
        let mut state = state_with(vec![commit("A")]);
        state.decks[0].scroll = 0;
        state.decks[0].scroll_target = 10;

        let mut steps = 0;
        while state.focused().scroll != state.focused().scroll_target {
            ease_scroll(&mut state);
            steps += 1;
            assert!(steps < 100, "ease_scroll never converged");
        }
        assert_eq!(state.focused().scroll, 10);

        // Calling again once converged must be a no-op, not oscillate.
        ease_scroll(&mut state);
        assert_eq!(state.focused().scroll, 10);
    }

    #[test]
    fn scrub_actions_are_a_no_op_in_update_by_design() {
        // The event loop intercepts these before they ever reach update(); this
        // guards against someone routing them here and silently dropping a scrub.
        let mut state = state_with(vec![commit("A")]);
        let before = state.focused().playhead;
        update(&mut state, Action::ScrubForward);
        update(&mut state, Action::ScrubBackward);
        assert_eq!(state.focused().playhead, before);
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
    fn set_playhead_invalidates_blame_for_the_focused_deck() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        state.blame = Some(vec![]);
        set_playhead(
            &mut state,
            0,
            1,
            snapshot("x\n"),
            Ghosts::default(),
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
        state.decks[0].playhead = 0;
        state.search = Some("fx".into()); // subsequence of "fix", not "add"/"refactor"

        // Nearest match strictly after the playhead: index 3 ("fix typo").
        assert_eq!(search_target(&state), Some(3));

        state.decks[0].playhead = 3;
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
        state.decks[0].playhead = 0;
        assert_eq!(jump_target(&state, true, day), 2); // first commit >= +1 day

        state.decks[0].playhead = 3; // time = 2*day; target = 2*day - day = day
        assert_eq!(jump_target(&state, false, day), 2); // "t1" is exactly 1 day back
    }

    #[test]
    fn jump_target_clamps_at_the_ends_of_history() {
        let state = state_with(vec![commit_at("only", 0)]);
        assert_eq!(jump_target(&state, true, 86_400), 0);
        assert_eq!(jump_target(&state, false, 86_400), 0);
    }

    #[test]
    fn toggle_pincer_spawns_and_removes_the_second_deck_with_fixed_roles() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        assert_eq!(state.decks.len(), 1);
        assert!(!state.pincer);

        update(&mut state, Action::TogglePincer);
        assert!(state.pincer);
        assert_eq!(state.decks.len(), 2);
        // Both seeded at the same position, with fixed opposite roles.
        assert_eq!(state.decks[0].playhead, state.decks[1].playhead);
        assert_eq!(state.decks[0].direction, Direction::Forward);
        assert_eq!(state.decks[1].direction, Direction::Backward);
        assert_eq!(state.focus, 0);

        update(&mut state, Action::TogglePincer);
        assert!(!state.pincer);
        assert_eq!(state.decks.len(), 1);
        assert_eq!(state.focus, 0);
    }

    #[test]
    fn toggle_focus_only_switches_inside_pincer_mode() {
        let mut state = state_with(vec![commit("A")]);
        update(&mut state, Action::ToggleFocus); // no-op in single-deck mode
        assert_eq!(state.focus, 0);

        update(&mut state, Action::TogglePincer);
        update(&mut state, Action::ToggleFocus);
        assert_eq!(state.focus, 1);
        update(&mut state, Action::ToggleFocus);
        assert_eq!(state.focus, 0);
    }

    #[test]
    fn ease_scroll_animates_both_decks_independently() {
        let mut state = state_with(vec![commit("A"), commit("B")]);
        update(&mut state, Action::TogglePincer);
        state.decks[0].scroll_target = 9;
        state.decks[1].scroll_target = 6;

        for _ in 0..100 {
            ease_scroll(&mut state);
        }
        assert_eq!(state.decks[0].scroll, 9);
        assert_eq!(state.decks[1].scroll, 6);
    }

    #[test]
    fn set_playhead_seeds_heat_trail_and_turnstile_on_reversal() {
        let mut state = state_with(vec![commit("A"), commit("B"), commit("C")]);
        // Forward move first (2 -> can't go up; start by moving to 1 backward).
        set_playhead(
            &mut state,
            0,
            1,
            snapshot(
                "x
",
            ),
            Ghosts::default(),
            Direction::Backward,
        );
        // Old playhead (2) echoes on the timeline and joins the trail; the
        // direction flipped from the default Forward, so the turnstile arms.
        assert_eq!(state.heat.get(&2), Some(&HEAT_MAX));
        assert_eq!(state.decks[0].trail.front(), Some(&2));
        assert_eq!(state.decks[0].turnstile, TURNSTILE_FRAMES);

        // Same-direction move: no new turnstile once the old one is cleared.
        state.decks[0].turnstile = 0;
        set_playhead(
            &mut state,
            0,
            0,
            snapshot(
                "y
",
            ),
            Ghosts::default(),
            Direction::Backward,
        );
        assert_eq!(state.decks[0].turnstile, 0);
        assert_eq!(state.decks[0].trail.front(), Some(&1));
    }

    #[test]
    fn anim_tick_decays_intro_turnstile_and_heat() {
        let mut state = state_with(vec![commit("A")]);
        state.intro = 2;
        state.decks[0].turnstile = 1;
        state.heat.insert(0, 1);
        assert!(state.animating());

        anim_tick(&mut state);
        assert_eq!(state.intro, 1);
        assert_eq!(state.decks[0].turnstile, 0);
        assert!(state.heat.is_empty(), "heat at 0 must be dropped");

        anim_tick(&mut state);
        assert_eq!(state.intro, 0);
        assert!(!state.animating());
    }

    #[test]
    fn intro_fade_progress_runs_zero_to_one_then_none() {
        let mut state = state_with(vec![commit("A")]);
        assert_eq!(state.intro_fade(), None);
        state.intro = INTRO_FRAMES;
        assert_eq!(state.intro_fade(), Some(0.0));
        state.intro = 1;
        let t = state.intro_fade().unwrap();
        assert!(t > 0.9 && t < 1.0, "{t}");
        state.intro = 0;
        assert_eq!(state.intro_fade(), None);
    }
}
