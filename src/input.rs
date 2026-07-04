//! The ONE table where keys become actions. Per the project conventions, no
//! widget matches keys on its own — everything funnels through [`map_key`].
//!
//! M0 bindings only. Scrubbing (`h`/`l`), playback (`space`), jumps (`g`/`G`,
//! `w`/`b`), and blame (`b`) join this table as their milestones land.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A semantic action, decoupled from the physical key that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    ScrollUp,
    ScrollDown,
    Top,
    Bottom,
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
        _ => None,
    }
}
