//! The main pane: the file exactly as it existed at the playhead, with a
//! line-number gutter, an optional blame gutter, syntax highlighting, and
//! directional ghost highlighting on changed lines (which overrides syntax
//! color, since the comet trail must read as red/blue).

use crate::app::{AppState, Direction};
use crate::diff::GHOST_MAX_DECAY;
use crate::repo::blame::format_age;
use crate::theme::Pole;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

const BLAME_AUTHOR_WIDTH: usize = 10;
const BLAME_AGE_WIDTH: usize = 4;

/// Truncate to `max` chars, marking the cut with an ellipsis so the gutter
/// never grows past a fixed width regardless of author name length.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let th = &state.theme;

    // "The file didn't exist here" is a state, not an error.
    if !state.current.existed {
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "the file does not exist at this point in history",
                Style::default().fg(th.chrome()),
            )),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
        frame.render_widget(placeholder, area);
        return;
    }

    let fg = Style::default().fg(th.foreground());
    let gutter = Style::default().fg(th.chrome());

    // The transition's motion sets the ghost hue: forward glows red, inverted
    // (scrubbing/playing backward) glows blue — see docs/whitepaper.md
    // "Directional ghosting".
    let ghost_pole = match state.direction {
        Direction::Forward => Pole::Future,
        Direction::Backward => Pole::Past,
    };

    let total = state.current.content.lines().count().max(1);
    let width = total.to_string().len();

    let lines: Vec<Line> = state
        .current
        .content
        .lines()
        .enumerate()
        .map(|(i, text)| {
            let text_style = match state.ghosts.get(&i) {
                Some(&decay) => {
                    let t = f32::from(decay) / f32::from(GHOST_MAX_DECAY);
                    Style::default().fg(th.ghost(ghost_pole, t))
                }
                None => fg,
            };

            let mut spans = Vec::new();
            if state.blame_visible {
                // Blank cells while the async result hasn't arrived yet (or a
                // line past what was blamed) rather than blocking the render.
                let (author, age) = match state.blame.as_ref().and_then(|b| b.get(i)) {
                    Some(line) => (
                        truncate(&line.author, BLAME_AUTHOR_WIDTH),
                        format_age(line.age_days),
                    ),
                    None => (String::new(), String::new()),
                };
                spans.push(Span::styled(
                    format!("{author:<BLAME_AUTHOR_WIDTH$} {age:>BLAME_AGE_WIDTH$} │ "),
                    gutter,
                ));
            }
            spans.push(Span::styled(
                format!("{:>width$} ", i + 1, width = width),
                gutter,
            ));

            // A ghosting line glows in one direction color, overriding syntax —
            // the comet trail must read as red/blue. Otherwise use syntax colors
            // if we have them, else the plain foreground.
            match (
                state.ghosts.contains_key(&i),
                state.highlighted.as_ref().and_then(|h| h.get(i)),
            ) {
                (false, Some(runs)) if !runs.is_empty() => {
                    for (piece, color) in runs {
                        spans.push(Span::styled(piece.clone(), Style::default().fg(*color)));
                    }
                }
                _ => spans.push(Span::styled(text.to_string(), text_style)),
            }
            Line::from(spans)
        })
        .collect();

    // No wrap: code keeps its columns; long lines truncate at the edge.
    let paragraph = Paragraph::new(lines).scroll((state.scroll, 0));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;
    use crate::repo::{CommitMeta, Snapshot};
    use crate::theme::{ColorDepth, Theme};
    use git2::Oid;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use std::collections::HashMap;

    fn state_with_ghost(direction: Direction, decay: u8) -> AppState {
        let mut state = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            vec![CommitMeta {
                oid: Oid::zero(),
                time: 0,
                author: "a".into(),
                summary: "s".into(),
                insertions: 1,
                deletions: 0,
                path: "f.txt".into(),
                is_merge: false,
                is_tagged: false,
            }],
            Snapshot {
                oid: Oid::zero(),
                content: "one\ntwo\nthree\n".into(),
                existed: true,
            },
        );
        state.direction = direction;
        state.ghosts = HashMap::from([(1usize, decay)]); // "two" is the changed line
        state
    }

    /// Render into a buffer and pull the fg color of the cell at the start of
    /// the text column on the given row (row 1 = second content line, past the
    /// gutter's "N " prefix).
    fn fg_at(state: &AppState, row: u16) -> Color {
        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        buffer[(2, row)].fg // column 2: past the "N " gutter for a 1-digit width
    }

    #[test]
    fn forward_ghost_glows_red_and_backward_glows_blue() {
        let forward = state_with_ghost(Direction::Forward, GHOST_MAX_DECAY);
        let backward = state_with_ghost(Direction::Backward, GHOST_MAX_DECAY);

        let is_red = |c: Color| matches!(c, Color::Rgb(r, _, b) if r > b);
        let is_blue = |c: Color| matches!(c, Color::Rgb(r, _, b) if b > r);

        assert!(is_red(fg_at(&forward, 1)), "{:?}", fg_at(&forward, 1));
        assert!(is_blue(fg_at(&backward, 1)), "{:?}", fg_at(&backward, 1));
    }

    #[test]
    fn ghost_fades_toward_the_unchanged_foreground_color_as_decay_drops() {
        let state = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            vec![],
            Snapshot {
                oid: Oid::zero(),
                content: "one\ntwo\nthree\n".into(),
                existed: true,
            },
        );
        let base_fg = fg_at(&state, 1); // no ghost at all: plain foreground

        let mut nearly_faded = state_with_ghost(Direction::Forward, 1);
        let nearly_faded_fg = fg_at(&nearly_faded, 1);
        nearly_faded.ghosts.clear();
        let no_ghost_fg = fg_at(&nearly_faded, 1);

        assert_eq!(base_fg, no_ghost_fg);
        assert_ne!(
            nearly_faded_fg, base_fg,
            "even the last decay step should still be visibly tinted"
        );
    }

    /// Render a row into a plain string of symbols, for substring assertions.
    fn row_text(state: &AppState, row: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..40)
            .map(|x| buffer[(x, row)].symbol().to_string())
            .collect()
    }

    #[test]
    fn blame_gutter_shows_author_and_age_when_visible_and_ready() {
        use crate::repo::BlameLine;

        let mut state = state_with_ghost(Direction::Forward, 0);
        state.blame_visible = true;
        state.blame = Some(vec![
            BlameLine {
                author: "Alice".into(),
                age_days: 3,
            },
            BlameLine {
                author: "Bob".into(),
                age_days: 400,
            },
            BlameLine {
                author: "Alice".into(),
                age_days: 3,
            },
        ]);

        let row0 = row_text(&state, 0);
        assert!(row0.contains("Alice"), "{row0:?}");
        assert!(row0.contains("3d"), "{row0:?}");

        let row1 = row_text(&state, 1);
        assert!(row1.contains("Bob"), "{row1:?}");
        assert!(row1.contains("1y"), "{row1:?}");
    }

    #[test]
    fn blame_gutter_is_blank_but_present_before_results_arrive() {
        let mut state = state_with_ghost(Direction::Forward, 0);
        state.blame_visible = true;
        state.blame = None; // async result hasn't landed yet

        // Must not panic and must not show stale/garbage author text.
        let row0 = row_text(&state, 0);
        assert!(!row0.contains("Alice"));
    }

    #[test]
    fn blame_gutter_is_absent_when_toggled_off() {
        let state = state_with_ghost(Direction::Forward, 0);
        assert!(!state.blame_visible);
        // With the gutter off, the text column starts right after "N " as in
        // the ghost tests above — i.e. no blame prefix consumes those columns.
        let row0 = row_text(&state, 0);
        assert!(row0.trim_start().starts_with("1 one"));
    }

    #[test]
    fn syntax_colors_apply_to_unghosted_lines() {
        let mut state = state_with_ghost(Direction::Forward, 0);
        state.ghosts.clear();
        // Two style runs on line 0 ("one") in distinct non-foreground colors.
        let a = Color::Rgb(200, 100, 50);
        let b = Color::Rgb(50, 200, 100);
        state.highlighted = Some(vec![
            vec![("on".to_string(), a), ("e".to_string(), b)],
            vec![],
            vec![],
        ]);

        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // "1 " gutter is 2 cells; "o","n" get color a, "e" gets color b.
        assert_eq!(buffer[(2, 0)].fg, a);
        assert_eq!(buffer[(3, 0)].fg, a);
        assert_eq!(buffer[(4, 0)].fg, b);
    }

    #[test]
    fn ghost_glow_overrides_syntax_on_changed_lines() {
        // Line 1 ("two") is both ghosted and syntax-highlighted; the glow must win.
        let mut state = state_with_ghost(Direction::Forward, GHOST_MAX_DECAY);
        let syntax_color = Color::Rgb(10, 20, 30);
        state.highlighted = Some(vec![
            vec![],
            vec![("two".to_string(), syntax_color)],
            vec![],
        ]);

        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cell = buffer[(2, 1)].fg;
        assert_ne!(cell, syntax_color, "ghost glow must override syntax color");
        assert!(
            matches!(cell, Color::Rgb(r, _, b) if r > b),
            "forward ghost should read red: {cell:?}"
        );
    }
}
