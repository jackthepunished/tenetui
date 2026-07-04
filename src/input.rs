//! The ONE table where keys become actions. Per the project conventions, no
//! widget matches keys on its own — everything funnels through [`map_key`]
//! (normal mode) or [`map_search_key`] (the `/`-search text-entry mode; a
//! second small table because it's a genuinely different input mode — text
//! entry vs. single-key commands — not a scattering of the same bindings).
//!
//! Note on `b`: the roadmap's own M3 bullets independently proposed `b` for
//! both "blame gutter toggle" and "jump back a day". Blame moved to `B`
//! (shift) so `w`/`b` could keep the day-jump pairing the whitepaper
//! describes; see docs/decisions.md.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A semantic action, decoupled from the physical key that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    ScrollUp,
    ScrollDown,
    Top,
    Bottom,
    /// Move the playhead one commit toward the past — inverted, blue.
    ScrubBackward,
    /// Move the playhead one commit toward the future — forward, red.
    ScrubForward,
    /// Start/stop autoplay, continuing in the last scrub direction.
    TogglePlayback,
    SpeedUp,
    SpeedDown,
    ToggleBlame,
    JumpDayForward,
    JumpDayBackward,
    JumpWeekForward,
    JumpWeekBackward,
    JumpFirst,
    JumpLast,
    /// Enter `/`-search mode.
    SearchStart,
}

/// Map a key press to an action, or `None` if the key is unbound. Only valid
/// in normal (non-search) mode — the caller checks `state.search` first.
pub fn map_key(key: KeyEvent) -> Option<Action> {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => Some(Action::Quit),
        (_, KeyCode::Char('q')) | (_, KeyCode::Esc) => Some(Action::Quit),
        (_, KeyCode::Char('j')) | (_, KeyCode::Down) => Some(Action::ScrollDown),
        (_, KeyCode::Char('k')) | (_, KeyCode::Up) => Some(Action::ScrollUp),
        (_, KeyCode::Home) => Some(Action::Top),
        (_, KeyCode::End) => Some(Action::Bottom),
        (_, KeyCode::Char('h')) | (_, KeyCode::Left) => Some(Action::ScrubBackward),
        (_, KeyCode::Char('l')) | (_, KeyCode::Right) => Some(Action::ScrubForward),
        (_, KeyCode::Char(' ')) => Some(Action::TogglePlayback),
        (_, KeyCode::Char('+')) | (_, KeyCode::Char('=')) => Some(Action::SpeedUp),
        (_, KeyCode::Char('-')) | (_, KeyCode::Char('_')) => Some(Action::SpeedDown),
        (_, KeyCode::Char('B')) => Some(Action::ToggleBlame),
        (_, KeyCode::Char('w')) => Some(Action::JumpDayForward),
        (_, KeyCode::Char('b')) => Some(Action::JumpDayBackward),
        (_, KeyCode::Char('}')) => Some(Action::JumpWeekForward),
        (_, KeyCode::Char('{')) => Some(Action::JumpWeekBackward),
        (_, KeyCode::Char('G')) => Some(Action::JumpLast),
        (_, KeyCode::Char('g')) => Some(Action::JumpFirst),
        (_, KeyCode::Char('/')) => Some(Action::SearchStart),
        _ => None,
    }
}

/// A keystroke while in `/`-search mode: everything is text entry except the
/// three control keys that leave the mode or edit the query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAction {
    Type(char),
    Backspace,
    /// Jump to the current match and leave search mode (`Enter`).
    Confirm,
    /// Leave search mode without moving (`Esc`).
    Cancel,
}

/// Map a key press while `state.search.is_some()`. Deliberately separate from
/// [`map_key`]: in this mode a bare `h` types the letter "h" into the query
/// rather than scrubbing backward.
pub fn map_search_key(key: KeyEvent) -> Option<SearchAction> {
    match key.code {
        KeyCode::Esc => Some(SearchAction::Cancel),
        KeyCode::Enter => Some(SearchAction::Confirm),
        KeyCode::Backspace => Some(SearchAction::Backspace),
        KeyCode::Char(c) => Some(SearchAction::Type(c)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn h_and_l_scrub_in_opposite_directions() {
        assert_eq!(
            map_key(key(KeyCode::Char('h'))),
            Some(Action::ScrubBackward)
        );
        assert_eq!(map_key(key(KeyCode::Char('l'))), Some(Action::ScrubForward));
        assert_eq!(map_key(key(KeyCode::Left)), Some(Action::ScrubBackward));
        assert_eq!(map_key(key(KeyCode::Right)), Some(Action::ScrubForward));
    }

    #[test]
    fn key_release_events_still_map_the_same_action() {
        // main.rs filters to KeyEventKind::Press before calling map_key; map_key
        // itself is kind-agnostic, so assert that explicitly rather than assume it.
        let mut k = key(KeyCode::Char('l'));
        k.kind = KeyEventKind::Release;
        assert_eq!(map_key(k), Some(Action::ScrubForward));
    }

    #[test]
    fn space_toggles_playback_and_shift_agnostic_keys_adjust_speed() {
        assert_eq!(
            map_key(key(KeyCode::Char(' '))),
            Some(Action::TogglePlayback)
        );
        // Both the shifted and unshifted physical key work, since terminals vary
        // in whether they report '+'/'=' or '-'/'_' depending on the keyboard.
        assert_eq!(map_key(key(KeyCode::Char('+'))), Some(Action::SpeedUp));
        assert_eq!(map_key(key(KeyCode::Char('='))), Some(Action::SpeedUp));
        assert_eq!(map_key(key(KeyCode::Char('-'))), Some(Action::SpeedDown));
        assert_eq!(map_key(key(KeyCode::Char('_'))), Some(Action::SpeedDown));
    }

    #[test]
    fn blame_toggle_uses_shift_b_not_lowercase_b() {
        // Lowercase `b` is claimed by the day-jump-backward motion instead —
        // see the module doc comment on the M3 roadmap key collision.
        assert_eq!(map_key(key(KeyCode::Char('B'))), Some(Action::ToggleBlame));
        assert_eq!(
            map_key(key(KeyCode::Char('b'))),
            Some(Action::JumpDayBackward)
        );
    }

    #[test]
    fn jump_motions_map_to_distinct_actions() {
        assert_eq!(
            map_key(key(KeyCode::Char('w'))),
            Some(Action::JumpDayForward)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('b'))),
            Some(Action::JumpDayBackward)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('}'))),
            Some(Action::JumpWeekForward)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('{'))),
            Some(Action::JumpWeekBackward)
        );
        assert_eq!(map_key(key(KeyCode::Char('g'))), Some(Action::JumpFirst));
        assert_eq!(map_key(key(KeyCode::Char('G'))), Some(Action::JumpLast));
    }

    #[test]
    fn slash_starts_search_mode() {
        assert_eq!(map_key(key(KeyCode::Char('/'))), Some(Action::SearchStart));
    }

    #[test]
    fn search_key_map_treats_letters_as_text_not_commands() {
        // The whole point of the second table: 'h' must NOT scrub in search mode.
        assert_eq!(
            map_search_key(key(KeyCode::Char('h'))),
            Some(SearchAction::Type('h'))
        );
        assert_eq!(
            map_search_key(key(KeyCode::Backspace)),
            Some(SearchAction::Backspace)
        );
        assert_eq!(
            map_search_key(key(KeyCode::Enter)),
            Some(SearchAction::Confirm)
        );
        assert_eq!(
            map_search_key(key(KeyCode::Esc)),
            Some(SearchAction::Cancel)
        );
    }
}
