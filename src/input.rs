//! The ONE table where keys become actions. Per the project conventions, no
//! widget matches keys on its own — everything funnels through [`map_key`].
//!
//! M0 + M1 bindings. Playback (`space`), jumps (`w`/`b`, `g`/`G`, `/`), and blame
//! (`b`) join this table as their milestones land.

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
}

/// Map a key press to an action, or `None` if the key is unbound.
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
}
