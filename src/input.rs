//! The ONE place keys become actions. Per the project conventions, no widget
//! matches keys on its own — everything funnels through the [`Keymap`] (normal
//! mode) or [`map_search_key`] (the `/`-search text-entry mode; a second small
//! table because it's a genuinely different input mode — text entry vs.
//! single-key commands — not a scattering of the same bindings).
//!
//! The keymap is data now, not a hardcoded match: [`Keymap::default`] holds the
//! built-in bindings and [`Keymap::apply_overrides`] layers the config file's
//! `[keybinds]` on top (see `config.rs`). Grouping the defaults in one table
//! keeps the "keys in one place" rule while making them user-remappable.
//!
//! Note on `b`: the roadmap's own M3 bullets independently proposed `b` for
//! both "blame gutter toggle" and "jump back a day". Blame moved to `B`
//! (shift) so `w`/`b` could keep the day-jump pairing the whitepaper
//! describes; see docs/decisions.md.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

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
    ToggleHelp,
    /// Toggle the temporal pincer (two playheads, forward + inverted).
    TogglePincer,
    /// Switch focus between the two pincer decks.
    ToggleFocus,
    /// Open the function picker (scope the view to one function).
    OpenFunctions,
    /// Toggle the space-time map view.
    ToggleMap,
}

impl Action {
    /// Every action, in a stable display order (used to build the help overlay).
    pub const ALL: [Action; 23] = [
        Action::ScrubForward,
        Action::ScrubBackward,
        Action::TogglePlayback,
        Action::SpeedUp,
        Action::SpeedDown,
        Action::JumpDayForward,
        Action::JumpDayBackward,
        Action::JumpWeekForward,
        Action::JumpWeekBackward,
        Action::JumpFirst,
        Action::JumpLast,
        Action::SearchStart,
        Action::ToggleBlame,
        Action::TogglePincer,
        Action::ToggleFocus,
        Action::OpenFunctions,
        Action::ToggleMap,
        Action::ScrollDown,
        Action::ScrollUp,
        Action::Top,
        Action::Bottom,
        Action::ToggleHelp,
        Action::Quit,
    ];

    /// The config token for this action (`[keybinds]` values reference these).
    pub fn name(self) -> &'static str {
        match self {
            Action::Quit => "quit",
            Action::ScrollUp => "scroll_up",
            Action::ScrollDown => "scroll_down",
            Action::Top => "top",
            Action::Bottom => "bottom",
            Action::ScrubBackward => "scrub_backward",
            Action::ScrubForward => "scrub_forward",
            Action::TogglePlayback => "toggle_playback",
            Action::SpeedUp => "speed_up",
            Action::SpeedDown => "speed_down",
            Action::ToggleBlame => "toggle_blame",
            Action::JumpDayForward => "jump_day_forward",
            Action::JumpDayBackward => "jump_day_backward",
            Action::JumpWeekForward => "jump_week_forward",
            Action::JumpWeekBackward => "jump_week_backward",
            Action::JumpFirst => "jump_first",
            Action::JumpLast => "jump_last",
            Action::SearchStart => "search",
            Action::ToggleHelp => "help",
            Action::TogglePincer => "toggle_pincer",
            Action::ToggleFocus => "toggle_focus",
            Action::OpenFunctions => "functions",
            Action::ToggleMap => "map",
        }
    }

    /// Parse a config token back into an action.
    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == name)
    }

    /// Human-readable label for the help overlay.
    pub fn describe(self) -> &'static str {
        match self {
            Action::Quit => "quit",
            Action::ScrollUp => "scroll up",
            Action::ScrollDown => "scroll down",
            Action::Top => "scroll to top",
            Action::Bottom => "scroll to bottom",
            Action::ScrubBackward => "scrub back (inverted)",
            Action::ScrubForward => "scrub forward",
            Action::TogglePlayback => "play / pause",
            Action::SpeedUp => "faster",
            Action::SpeedDown => "slower",
            Action::ToggleBlame => "toggle blame gutter",
            Action::JumpDayForward => "jump forward a day",
            Action::JumpDayBackward => "jump back a day",
            Action::JumpWeekForward => "jump forward a week",
            Action::JumpWeekBackward => "jump back a week",
            Action::JumpFirst => "first commit",
            Action::JumpLast => "last commit (HEAD)",
            Action::SearchStart => "search commit messages",
            Action::ToggleHelp => "toggle this help",
            Action::TogglePincer => "temporal pincer (turnstile)",
            Action::ToggleFocus => "switch pincer pane",
            Action::OpenFunctions => "scope to a function",
            Action::ToggleMap => "space-time map",
        }
    }
}

/// A normalized key press used as a map key. Only the Ctrl modifier is tracked;
/// Shift is carried by the character's case (as terminals report it) and other
/// modifiers are ignored — matching how the original hardcoded table behaved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub ctrl: bool,
    pub code: KeyCode,
}

impl KeyChord {
    fn plain(code: KeyCode) -> Self {
        KeyChord { ctrl: false, code }
    }

    fn from_event(key: KeyEvent) -> Self {
        KeyChord {
            ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
            code: key.code,
        }
    }

    /// Parse a config binding string (`"l"`, `"space"`, `"ctrl-c"`, `"{"`, ...).
    pub fn parse(s: &str) -> Option<KeyChord> {
        let s = s.trim();
        let (ctrl, key) = match s.strip_prefix("ctrl-") {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        Some(KeyChord {
            ctrl,
            code: parse_code(key)?,
        })
    }

    /// Short label for the help overlay (`"space"`, `"ctrl-c"`, `"↑"`, `"l"`).
    pub fn display(self) -> String {
        let key = match self.code {
            KeyCode::Char(' ') => "space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "enter".to_string(),
            KeyCode::Esc => "esc".to_string(),
            KeyCode::Tab => "tab".to_string(),
            KeyCode::Backspace => "backspace".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::Home => "home".to_string(),
            KeyCode::End => "end".to_string(),
            other => format!("{other:?}").to_lowercase(),
        };
        if self.ctrl {
            format!("ctrl-{key}")
        } else {
            key
        }
    }
}

fn parse_code(k: &str) -> Option<KeyCode> {
    match k {
        "space" => Some(KeyCode::Char(' ')),
        "enter" => Some(KeyCode::Enter),
        "esc" => Some(KeyCode::Esc),
        "tab" => Some(KeyCode::Tab),
        "backspace" => Some(KeyCode::Backspace),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        _ => k
            .chars()
            .next()
            .filter(|_| k.chars().count() == 1)
            .map(KeyCode::Char),
    }
}

/// The active key→action bindings. Built-in defaults, optionally overlaid with
/// the config file's `[keybinds]`.
#[derive(Clone, Debug)]
pub struct Keymap {
    bindings: HashMap<KeyChord, Action>,
}

impl Default for Keymap {
    fn default() -> Self {
        use Action::*;
        use KeyCode::*;
        let mut bindings = HashMap::new();
        let mut bind = |chord: KeyChord, action: Action| {
            bindings.insert(chord, action);
        };

        bind(
            KeyChord {
                ctrl: true,
                code: Char('c'),
            },
            Quit,
        );
        bind(KeyChord::plain(Char('q')), Quit);
        bind(KeyChord::plain(Esc), Quit);
        bind(KeyChord::plain(Char('j')), ScrollDown);
        bind(KeyChord::plain(Down), ScrollDown);
        bind(KeyChord::plain(Char('k')), ScrollUp);
        bind(KeyChord::plain(Up), ScrollUp);
        bind(KeyChord::plain(Home), Top);
        bind(KeyChord::plain(End), Bottom);
        bind(KeyChord::plain(Char('h')), ScrubBackward);
        bind(KeyChord::plain(Left), ScrubBackward);
        bind(KeyChord::plain(Char('l')), ScrubForward);
        bind(KeyChord::plain(Right), ScrubForward);
        bind(KeyChord::plain(Char(' ')), TogglePlayback);
        bind(KeyChord::plain(Char('+')), SpeedUp);
        bind(KeyChord::plain(Char('=')), SpeedUp);
        bind(KeyChord::plain(Char('-')), SpeedDown);
        bind(KeyChord::plain(Char('_')), SpeedDown);
        bind(KeyChord::plain(Char('B')), ToggleBlame);
        bind(KeyChord::plain(Char('w')), JumpDayForward);
        bind(KeyChord::plain(Char('b')), JumpDayBackward);
        bind(KeyChord::plain(Char('}')), JumpWeekForward);
        bind(KeyChord::plain(Char('{')), JumpWeekBackward);
        bind(KeyChord::plain(Char('G')), JumpLast);
        bind(KeyChord::plain(Char('g')), JumpFirst);
        bind(KeyChord::plain(Char('/')), SearchStart);
        bind(KeyChord::plain(Char('?')), ToggleHelp);
        bind(KeyChord::plain(Char('t')), TogglePincer);
        bind(KeyChord::plain(Tab), ToggleFocus);
        bind(KeyChord::plain(Char('F')), OpenFunctions);
        bind(KeyChord::plain(Char('m')), ToggleMap);

        Keymap { bindings }
    }
}

impl Keymap {
    /// Layer config `[keybinds]` (`key-string → action-name`) over the defaults.
    /// A binding whose key or action doesn't parse is warned about and skipped,
    /// so one typo can't break the rest of the map.
    pub fn apply_overrides(&mut self, overrides: &HashMap<String, String>) {
        for (key_str, action_str) in overrides {
            match (KeyChord::parse(key_str), Action::from_name(action_str)) {
                (Some(chord), Some(action)) => {
                    self.bindings.insert(chord, action);
                }
                _ => eprintln!("tenetui: ignoring invalid keybind {key_str:?} = {action_str:?}"),
            }
        }
    }

    /// Resolve a key press to an action (normal mode). The caller checks
    /// `state.search`/`state.help_visible` first.
    pub fn action_for(&self, key: KeyEvent) -> Option<Action> {
        self.bindings.get(&KeyChord::from_event(key)).copied()
    }

    /// The keys bound to `action`, for the help overlay. Sorted for stable
    /// display (single chars before named keys, then alphabetical).
    pub fn keys_for(&self, action: Action) -> Vec<KeyChord> {
        let mut keys: Vec<KeyChord> = self
            .bindings
            .iter()
            .filter(|&(_, &a)| a == action)
            .map(|(&chord, _)| chord)
            .collect();
        keys.sort_by_key(|c| c.display());
        keys
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

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn h_and_l_scrub_in_opposite_directions() {
        let km = Keymap::default();
        assert_eq!(
            km.action_for(key(KeyCode::Char('h'))),
            Some(Action::ScrubBackward)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('l'))),
            Some(Action::ScrubForward)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Left)),
            Some(Action::ScrubBackward)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Right)),
            Some(Action::ScrubForward)
        );
    }

    #[test]
    fn ctrl_c_quits_but_plain_c_is_unbound() {
        let km = Keymap::default();
        assert_eq!(km.action_for(ctrl(KeyCode::Char('c'))), Some(Action::Quit));
        assert_eq!(km.action_for(key(KeyCode::Char('c'))), None);
    }

    #[test]
    fn key_release_events_still_map_the_same_action() {
        // main.rs filters to KeyEventKind::Press before calling the keymap;
        // the keymap itself is kind-agnostic, so assert that explicitly.
        let km = Keymap::default();
        let mut k = key(KeyCode::Char('l'));
        k.kind = KeyEventKind::Release;
        assert_eq!(km.action_for(k), Some(Action::ScrubForward));
    }

    #[test]
    fn space_toggles_playback_and_shift_agnostic_keys_adjust_speed() {
        let km = Keymap::default();
        assert_eq!(
            km.action_for(key(KeyCode::Char(' '))),
            Some(Action::TogglePlayback)
        );
        // Both the shifted and unshifted physical key work, since terminals vary
        // in whether they report '+'/'=' or '-'/'_' depending on the keyboard.
        assert_eq!(
            km.action_for(key(KeyCode::Char('+'))),
            Some(Action::SpeedUp)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('='))),
            Some(Action::SpeedUp)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('-'))),
            Some(Action::SpeedDown)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('_'))),
            Some(Action::SpeedDown)
        );
    }

    #[test]
    fn blame_toggle_uses_shift_b_not_lowercase_b() {
        // Lowercase `b` is claimed by the day-jump-backward motion instead —
        // see the module doc comment on the M3 roadmap key collision.
        let km = Keymap::default();
        assert_eq!(
            km.action_for(key(KeyCode::Char('B'))),
            Some(Action::ToggleBlame)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('b'))),
            Some(Action::JumpDayBackward)
        );
    }

    #[test]
    fn jump_motions_map_to_distinct_actions() {
        let km = Keymap::default();
        assert_eq!(
            km.action_for(key(KeyCode::Char('w'))),
            Some(Action::JumpDayForward)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('b'))),
            Some(Action::JumpDayBackward)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('}'))),
            Some(Action::JumpWeekForward)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('{'))),
            Some(Action::JumpWeekBackward)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('g'))),
            Some(Action::JumpFirst)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('G'))),
            Some(Action::JumpLast)
        );
    }

    #[test]
    fn slash_starts_search_and_question_toggles_help() {
        let km = Keymap::default();
        assert_eq!(
            km.action_for(key(KeyCode::Char('/'))),
            Some(Action::SearchStart)
        );
        assert_eq!(
            km.action_for(key(KeyCode::Char('?'))),
            Some(Action::ToggleHelp)
        );
    }

    #[test]
    fn config_overrides_rebind_and_ignore_garbage() {
        let mut km = Keymap::default();
        let overrides = HashMap::from([
            ("x".to_string(), "quit".to_string()), // new binding
            ("ctrl-r".to_string(), "scrub_forward".to_string()), // ctrl chord
            ("nope".to_string(), "not_an_action".to_string()), // bad action
            ("also-bad-key-###".to_string(), "quit".to_string()), // bad key
        ]);
        km.apply_overrides(&overrides);

        assert_eq!(km.action_for(key(KeyCode::Char('x'))), Some(Action::Quit));
        assert_eq!(
            km.action_for(ctrl(KeyCode::Char('r'))),
            Some(Action::ScrubForward)
        );
        // Defaults survive the override pass.
        assert_eq!(
            km.action_for(key(KeyCode::Char('l'))),
            Some(Action::ScrubForward)
        );
    }

    #[test]
    fn action_name_roundtrips_for_every_action() {
        for action in Action::ALL {
            assert_eq!(Action::from_name(action.name()), Some(action));
        }
    }

    #[test]
    fn keys_for_reports_all_bindings_of_an_action() {
        let km = Keymap::default();
        let displays: Vec<String> = km
            .keys_for(Action::ScrubForward)
            .into_iter()
            .map(|c| c.display())
            .collect();
        assert!(displays.contains(&"l".to_string()));
        assert!(displays.contains(&"→".to_string()));
    }

    #[test]
    fn chord_parse_handles_named_keys_and_ctrl() {
        assert_eq!(
            KeyChord::parse("space"),
            Some(KeyChord::plain(KeyCode::Char(' ')))
        );
        assert_eq!(
            KeyChord::parse("l"),
            Some(KeyChord::plain(KeyCode::Char('l')))
        );
        assert_eq!(
            KeyChord::parse("ctrl-c"),
            Some(KeyChord {
                ctrl: true,
                code: KeyCode::Char('c')
            })
        );
        assert_eq!(
            KeyChord::parse("left"),
            Some(KeyChord::plain(KeyCode::Left))
        );
        assert_eq!(KeyChord::parse("nonsense"), None);
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
