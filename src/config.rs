//! User configuration from `~/.config/tenetui/config.toml` (and platform
//! equivalents via `dirs`). Entirely optional: a missing file or a parse error
//! falls back to defaults with a warning — a bad config never stops the app.
//!
//! ```toml
//! speed_ms = 150       # initial playback cadence (ms per commit)
//! cache_size = 512     # snapshot LRU capacity
//!
//! [keybinds]           # key = action-name; layered over the built-in defaults
//! x = "quit"
//! "ctrl-r" = "scrub_forward"
//! ```

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

const DEFAULT_SPEED_MS: u64 = 250;
const SPEED_MIN_MS: u64 = 30;
const SPEED_MAX_MS: u64 = 2000;
const DEFAULT_CACHE_SIZE: usize = 256;

/// Parsed config. All fields optional so a partial file is valid; resolved
/// values (with defaults + clamping applied) come from the accessor methods.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    speed_ms: Option<u64>,
    cache_size: Option<usize>,
    pub keybinds: HashMap<String, String>,
}

impl Config {
    /// Load and parse the config file, or return defaults. Warns to stderr on a
    /// parse error rather than failing (the terminal isn't in raw mode yet).
    pub fn load() -> Self {
        let Some(text) = Self::path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return Config::default();
        };
        Self::parse(&text).unwrap_or_else(|e| {
            eprintln!("tenetui: config parse error ({e}); using defaults");
            Config::default()
        })
    }

    /// Parse config text. Split out from [`Self::load`] so it's testable without
    /// touching the filesystem.
    pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("tenetui").join("config.toml"))
    }

    /// Initial playback cadence, clamped to the same bounds the `+`/`-` keys use.
    pub fn speed_ms(&self) -> u64 {
        self.speed_ms
            .unwrap_or(DEFAULT_SPEED_MS)
            .clamp(SPEED_MIN_MS, SPEED_MAX_MS)
    }

    /// Snapshot cache capacity (at least 1).
    pub fn cache_size(&self) -> usize {
        self.cache_size.unwrap_or(DEFAULT_CACHE_SIZE).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_defaults() {
        let c = Config::parse("").unwrap();
        assert_eq!(c.speed_ms(), DEFAULT_SPEED_MS);
        assert_eq!(c.cache_size(), DEFAULT_CACHE_SIZE);
        assert!(c.keybinds.is_empty());
    }

    #[test]
    fn parses_values_and_keybinds() {
        let c = Config::parse(
            r#"
            speed_ms = 120
            cache_size = 64
            [keybinds]
            x = "quit"
            "ctrl-r" = "scrub_forward"
            "#,
        )
        .unwrap();
        assert_eq!(c.speed_ms(), 120);
        assert_eq!(c.cache_size(), 64);
        assert_eq!(c.keybinds.get("x").map(String::as_str), Some("quit"));
        assert_eq!(
            c.keybinds.get("ctrl-r").map(String::as_str),
            Some("scrub_forward")
        );
    }

    #[test]
    fn out_of_range_speed_is_clamped() {
        assert_eq!(
            Config::parse("speed_ms = 5").unwrap().speed_ms(),
            SPEED_MIN_MS
        );
        assert_eq!(
            Config::parse("speed_ms = 99999").unwrap().speed_ms(),
            SPEED_MAX_MS
        );
    }

    #[test]
    fn zero_cache_size_floors_to_one() {
        assert_eq!(Config::parse("cache_size = 0").unwrap().cache_size(), 1);
    }

    #[test]
    fn unknown_field_is_an_error_not_a_silent_ignore() {
        // deny_unknown_fields → a typo'd key is a parse error the user sees,
        // rather than being silently dropped.
        assert!(Config::parse("speeed_ms = 100").is_err());
    }
}
