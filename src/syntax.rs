//! Syntax highlighting. Turns file content into per-line colored runs using
//! `syntect`, colored through tenetui's muted palette so code reads as quiet
//! structure and the saturated red/blue stays reserved for time-direction.
//!
//! Pure computation: no git2, no rendering. The caller highlights a snapshot
//! when it becomes current (off the `draw()` path — see the 16ms frame-budget
//! rule) and hands the result to `ui::filepane`. On a changed line the ghost
//! glow overrides these colors; unchanged lines show syntax color.

use crate::theme::{SyntaxClass, Theme, syntax_rgb};
use ratatui::style::Color;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SynColor, ScopeSelectors, StyleModifier, Theme as SynTheme, ThemeItem, ThemeSettings,
};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// One rendered line as a sequence of `(text, color)` runs.
pub type StyledLine = Vec<(String, Color)>;
/// A whole file's highlighting, one [`StyledLine`] per source line (indices line
/// up with `content.lines()`), so the file pane can index it by line number.
pub type Highlighted = Vec<StyledLine>;

/// Owns the syntax definitions and the muted color theme. Construct once
/// (it loads syntect's default syntax set) and reuse for every highlight.
pub struct Highlighter {
    syntaxes: SyntaxSet,
    theme: SynTheme,
}

impl Highlighter {
    pub fn new() -> Self {
        Highlighter {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            theme: muted_theme(),
        }
    }

    /// Highlight `content`, picking the syntax from `path`'s extension (falling
    /// back to a first-line shebang match). Returns `None` when no syntax
    /// matches or highlighting fails — the caller then renders plain text.
    /// `app_theme` routes syntect's resolved colors through the terminal's
    /// color-depth fallback.
    pub fn highlight(&self, content: &str, path: &str, app_theme: &Theme) -> Option<Highlighted> {
        let syntax = self.find_syntax(content, path)?;
        let mut lines = HighlightLines::new(syntax, &self.theme);

        let mut out = Highlighted::new();
        for line in LinesWithEndings::from(content) {
            let ranges = lines.highlight_line(line, &self.syntaxes).ok()?;
            let styled = ranges
                .into_iter()
                .filter_map(|(style, piece)| {
                    let text = piece.trim_end_matches(['\n', '\r']);
                    if text.is_empty() {
                        return None;
                    }
                    let fg = style.foreground;
                    Some((text.to_string(), app_theme.rgb(fg.r, fg.g, fg.b)))
                })
                .collect();
            out.push(styled);
        }
        Some(out)
    }

    fn find_syntax(&self, content: &str, path: &str) -> Option<&syntect::parsing::SyntaxReference> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        self.syntaxes.find_syntax_by_extension(ext).or_else(|| {
            self.syntaxes
                .find_syntax_by_first_line(content.lines().next().unwrap_or(""))
        })
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

/// A request to highlight a snapshot, tagged with a generation so a result that
/// arrives after the playhead has moved on can be discarded (same pattern as
/// blame — see docs/architecture.md). Highlighting is far too slow to run on
/// the frame path (the `hot_paths` bench measured ~20ms for 500 lines), so it
/// runs on its own thread and the main loop only ever reads finished results.
pub struct HighlightRequest {
    pub generation: u64,
    /// Which deck this is for (temporal pincer has two); echoed back so the
    /// result is routed to the right pane.
    pub deck: usize,
    pub content: Arc<str>,
    pub path: String,
    pub theme: Theme,
}

pub struct HighlightResult {
    pub generation: u64,
    pub deck: usize,
    pub highlighted: Option<Highlighted>,
}

/// Highlight worker loop: block for a request, then coalesce the queued backlog
/// to the latest request *per deck* (positions scrubbed through are superseded,
/// but the two pincer decks don't starve each other), highlight each, send back.
/// Ends when either channel closes.
pub fn run(requests: Receiver<HighlightRequest>, ready: Sender<HighlightResult>) {
    let highlighter = Highlighter::new();
    loop {
        let Ok(first) = requests.recv() else {
            return;
        };
        let mut latest: HashMap<usize, HighlightRequest> = HashMap::new();
        latest.insert(first.deck, first);
        while let Ok(req) = requests.try_recv() {
            latest.insert(req.deck, req);
        }
        for request in latest.into_values() {
            let highlighted =
                highlighter.highlight(&request.content, &request.path, &request.theme);
            if ready
                .send(HighlightResult {
                    generation: request.generation,
                    deck: request.deck,
                    highlighted,
                })
                .is_err()
            {
                return;
            }
        }
    }
}

/// Build a `syntect` theme whose only colors are tenetui's muted syntax palette
/// (`theme::syntax_rgb`). `syntect` does the scope→style resolution; we just
/// supply the color table and let it match.
fn muted_theme() -> SynTheme {
    let color = |class| {
        let (r, g, b) = syntax_rgb(class);
        SynColor { r, g, b, a: 255 }
    };
    let item = |selectors: &str, class| -> Option<ThemeItem> {
        Some(ThemeItem {
            scope: ScopeSelectors::from_str(selectors).ok()?,
            style: StyleModifier {
                foreground: Some(color(class)),
                background: None,
                font_style: None,
            },
        })
    };

    // Most-general first; syntect picks the most specific match per token.
    let scopes = [
        item("comment", SyntaxClass::Comment),
        item(
            "keyword, storage.modifier, variable.language",
            SyntaxClass::Keyword,
        ),
        item("storage.type", SyntaxClass::Keyword),
        item(
            "string, string.quoted, constant.other.symbol",
            SyntaxClass::StringLit,
        ),
        item(
            "constant.numeric, constant.language, constant.character",
            SyntaxClass::Constant,
        ),
        item(
            "entity.name.type, entity.name.class, support.type, support.class",
            SyntaxClass::Type,
        ),
        item(
            "entity.name.function, support.function, meta.function-call",
            SyntaxClass::Function,
        ),
        item(
            "keyword.operator, punctuation.separator, punctuation.terminator",
            SyntaxClass::Operator,
        ),
    ]
    .into_iter()
    .flatten()
    .collect();

    SynTheme {
        settings: ThemeSettings {
            foreground: Some(color(SyntaxClass::Text)),
            ..Default::default()
        },
        scopes,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ColorDepth;

    const RUST: &str = "// a comment\nfn main() {\n    let x = \"hello\";\n}\n";

    fn app_theme() -> Theme {
        Theme {
            depth: ColorDepth::TrueColor,
        }
    }

    #[test]
    fn highlights_rust_by_extension_with_line_parity() {
        let h = Highlighter::new();
        let out = h
            .highlight(RUST, "src/main.rs", &app_theme())
            .expect("rust should highlight");
        // One styled line per source line.
        assert_eq!(out.len(), RUST.lines().count());
    }

    #[test]
    fn distinct_token_kinds_get_distinct_colors() {
        let h = Highlighter::new();
        let out = h.highlight(RUST, "x.rs", &app_theme()).unwrap();

        // The comment line's color should differ from the keyword line's — i.e.
        // highlighting actually classified something, not one flat color.
        let comment_color = out[0].first().map(|(_, c)| *c);
        let keyword_color = out[1].first().map(|(_, c)| *c);
        assert!(comment_color.is_some() && keyword_color.is_some());
        assert_ne!(comment_color, keyword_color);

        // The `let x = "hello";` line should contain more than one distinct
        // color (keyword/operator/string), proving intra-line classification.
        let colors: std::collections::HashSet<_> = out[2].iter().map(|(_, c)| *c).collect();
        assert!(
            colors.len() > 1,
            "expected multiple token colors, got {colors:?}"
        );
    }

    #[test]
    fn unknown_extension_returns_none_for_plain_fallback() {
        let h = Highlighter::new();
        assert!(
            h.highlight("just text\n", "notes.unknownext", &app_theme())
                .is_none()
        );
    }
}
