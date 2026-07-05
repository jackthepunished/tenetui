//! The main pane: the file exactly as it existed at the playhead, with a
//! line-number gutter, an optional blame gutter, syntax highlighting, and
//! directional ghost highlighting on changed lines. Ghosting is word-level: the
//! changed words burn hot in the direction color while the rest of the line
//! half-glows, so the edit that moved is legible during playback.

use crate::app::{Deck, Direction};
use crate::diff::GHOST_MAX_DECAY;
use crate::repo::BlameLine;
use crate::repo::blame::format_age;
use crate::theme::{Pole, Theme};
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

/// Push a ghosted line's spans: the changed word ranges glow at full decay `t`,
/// the rest of the line at a fraction of it (still red/blue, but quieter). An
/// empty `ranges` (a whole-line insertion) glows the entire line.
fn ghost_spans(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    ranges: &[(usize, usize)],
    th: &Theme,
    pole: Pole,
    t: f32,
) {
    if ranges.is_empty() {
        spans.push(Span::styled(
            text.to_string(),
            Style::default().fg(th.ghost(pole, t)),
        ));
        return;
    }
    let hot = Style::default().fg(th.ghost(pole, t));
    let cold = Style::default().fg(th.ghost(pole, t * 0.35));
    let is_hot = |ci: usize| ranges.iter().any(|&(a, b)| ci >= a && ci < b);

    // Run-length group consecutive chars by hot/cold and emit one span per run.
    let mut run = String::new();
    let mut run_hot: Option<bool> = None;
    for (ci, ch) in text.chars().enumerate() {
        let h = is_hot(ci);
        if run_hot.is_some_and(|prev| prev != h) {
            let style = if run_hot == Some(true) { hot } else { cold };
            spans.push(Span::styled(std::mem::take(&mut run), style));
        }
        run.push(ch);
        run_hot = Some(h);
    }
    if !run.is_empty() {
        let style = if run_hot == Some(true) { hot } else { cold };
        spans.push(Span::styled(run, style));
    }
}

/// A centered dim message for the "nothing to show here" states.
fn placeholder(th: &Theme, msg: &str) -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            msg.to_string(),
            Style::default().fg(th.chrome()),
        )),
    ])
    .alignment(Alignment::Center)
    .wrap(Wrap { trim: true })
}

/// Presentation flags for one pane render, bundled so the signature stays flat.
#[derive(Default)]
pub struct PaneView<'a> {
    /// Show the blame gutter (the focused deck only).
    pub show_blame: bool,
    /// That deck's resolved blame; `None` renders a blank gutter while the
    /// async result is in flight.
    pub blame: Option<&'a [BlameLine]>,
    /// Clamp to `deck.scope_range` (a function), with a placeholder where the
    /// function is absent at this commit.
    pub scoped: bool,
    /// Cold-open progress: while `Some(t)` text renders on a dim-to-steel ramp
    /// and full color arrives when the intro ends.
    pub fade: Option<f32>,
}

/// Render one deck into `area` under `view`'s presentation flags. A non-zero
/// `deck.turnstile` briefly washes the pane in the new direction's hue — the
/// flip through the machine.
pub fn render(frame: &mut Frame, area: Rect, deck: &Deck, theme: &Theme, view: PaneView) {
    let th = theme;
    let PaneView {
        show_blame,
        blame,
        scoped,
        fade,
    } = view;

    // "The file didn't exist here" is a state, not an error.
    if !deck.current.existed {
        frame.render_widget(
            placeholder(th, "the file does not exist at this point in history"),
            area,
        );
        return;
    }

    // Scoped to a function that isn't present at this commit → placeholder.
    if scoped && deck.scope_range.is_none() {
        frame.render_widget(
            placeholder(th, "this function does not exist at this point in history"),
            area,
        );
        return;
    }
    // The line range to show: the whole file, or just the scoped function's.
    let range = if scoped { deck.scope_range } else { None };

    let fg = Style::default().fg(th.foreground());
    let gutter = Style::default().fg(th.chrome());

    // The transition's motion sets the ghost hue: forward glows red, inverted
    // (scrubbing/playing backward) glows blue — see docs/whitepaper.md
    // "Directional ghosting".
    let ghost_pole = match deck.direction {
        Direction::Forward => Pole::Future,
        Direction::Backward => Pole::Past,
    };

    let total = deck.current.content.lines().count().max(1);
    let width = total.to_string().len();

    let lines: Vec<Line> = deck
        .current
        .content
        .lines()
        .enumerate()
        .filter(|(i, _)| range.is_none_or(|(a, b)| *i >= a && *i <= b))
        .map(|(i, text)| {
            let ghost_t = deck
                .ghosts
                .decay_of(i)
                .map(|d| f32::from(d) / f32::from(GHOST_MAX_DECAY));

            let mut spans = Vec::new();
            if show_blame {
                // Blank cells while the async result hasn't arrived yet (or a
                // line past what was blamed) rather than blocking the render.
                let (author, age) = match blame.and_then(|b| b.get(i)) {
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

            // Precedence: cold-open fade > turnstile wash > ghost glow >
            // syntax > plain. The first two are transient whole-pane states, so
            // they may override syntax color for a few frames.
            if let Some(t) = fade {
                spans.push(Span::styled(
                    text.to_string(),
                    Style::default().fg(th.fade(t)),
                ));
            } else if deck.turnstile > 0 {
                let frac = f32::from(deck.turnstile) / f32::from(crate::app::TURNSTILE_FRAMES);
                spans.push(Span::styled(
                    text.to_string(),
                    Style::default().fg(th.ghost(ghost_pole, 0.20 + 0.35 * frac)),
                ));
            } else if let Some(t) = ghost_t {
                // A ghosting line reads red/blue, overriding syntax — but only
                // the *changed words* burn hot; the rest of the line half-glows,
                // so the edit that moved is legible during playback.
                ghost_spans(
                    &mut spans,
                    text,
                    deck.ghosts.hot_ranges(i),
                    th,
                    ghost_pole,
                    t,
                );
            } else if let Some(runs) = deck.highlighted.as_ref().and_then(|h| h.get(i))
                && !runs.is_empty()
            {
                for (piece, color) in runs {
                    spans.push(Span::styled(piece.clone(), Style::default().fg(*color)));
                }
            } else {
                spans.push(Span::styled(text.to_string(), fg));
            }
            Line::from(spans)
        })
        .collect();

    // No wrap: code keeps its columns; long lines truncate at the edge.
    let paragraph = Paragraph::new(lines).scroll((deck.scroll, 0));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Deck;
    use crate::diff::Ghosts;
    use crate::repo::Snapshot;
    use crate::theme::ColorDepth;
    use git2::Oid;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use std::collections::HashMap;

    fn theme() -> Theme {
        Theme {
            depth: ColorDepth::TrueColor,
        }
    }

    fn deck_with(content: &str) -> Deck {
        let mut deck = Deck::new_for_test();
        deck.current = Snapshot {
            oid: Oid::zero(),
            content: content.into(),
            existed: true,
        };
        deck
    }

    fn deck_with_ghost(direction: Direction, decay: u8) -> Deck {
        let mut deck = deck_with("one\ntwo\nthree\n");
        deck.direction = direction;
        // "two" (line 1) is the changed line; no specific word range.
        deck.ghosts = Ghosts::from_decay(HashMap::from([(1usize, decay)]));
        deck
    }

    /// Render a deck (no blame gutter) and pull the fg color of the text column
    /// on `row` (column 2 = past the "N " gutter for a 1-digit width).
    fn fg_at(deck: &Deck, row: u16) -> Color {
        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), deck, &theme(), PaneView::default()))
            .unwrap();
        terminal.backend().buffer()[(2, row)].fg
    }

    #[test]
    fn forward_ghost_glows_red_and_backward_glows_blue() {
        let forward = deck_with_ghost(Direction::Forward, GHOST_MAX_DECAY);
        let backward = deck_with_ghost(Direction::Backward, GHOST_MAX_DECAY);

        let is_red = |c: Color| matches!(c, Color::Rgb(r, _, b) if r > b);
        let is_blue = |c: Color| matches!(c, Color::Rgb(r, _, b) if b > r);

        assert!(is_red(fg_at(&forward, 1)), "{:?}", fg_at(&forward, 1));
        assert!(is_blue(fg_at(&backward, 1)), "{:?}", fg_at(&backward, 1));
    }

    #[test]
    fn ghost_fades_toward_the_unchanged_foreground_color_as_decay_drops() {
        let plain = deck_with("one\ntwo\nthree\n");
        let base_fg = fg_at(&plain, 1); // no ghost at all: plain foreground

        let mut nearly_faded = deck_with_ghost(Direction::Forward, 1);
        let nearly_faded_fg = fg_at(&nearly_faded, 1);
        nearly_faded.ghosts = Ghosts::default();
        let no_ghost_fg = fg_at(&nearly_faded, 1);

        assert_eq!(base_fg, no_ghost_fg);
        assert_ne!(
            nearly_faded_fg, base_fg,
            "even the last decay step should still be visibly tinted"
        );
    }

    /// Render a deck (with blame gutter) into a plain string of symbols on `row`.
    fn blame_row_text(deck: &Deck, blame: Option<&[BlameLine]>, row: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    deck,
                    &theme(),
                    PaneView {
                        show_blame: true,
                        blame,
                        ..PaneView::default()
                    },
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..40)
            .map(|x| buffer[(x, row)].symbol().to_string())
            .collect()
    }

    #[test]
    fn blame_gutter_shows_author_and_age_when_visible_and_ready() {
        let deck = deck_with("one\ntwo\nthree\n");
        let blame = vec![
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
        ];

        let row0 = blame_row_text(&deck, Some(&blame), 0);
        assert!(row0.contains("Alice"), "{row0:?}");
        assert!(row0.contains("3d"), "{row0:?}");

        let row1 = blame_row_text(&deck, Some(&blame), 1);
        assert!(row1.contains("Bob"), "{row1:?}");
        assert!(row1.contains("1y"), "{row1:?}");
    }

    #[test]
    fn blame_gutter_is_blank_but_present_before_results_arrive() {
        let deck = deck_with("one\ntwo\nthree\n");
        // Gutter on, no data yet: must not panic or show stale author text.
        let row0 = blame_row_text(&deck, None, 0);
        assert!(!row0.contains("Alice"));
    }

    #[test]
    fn blame_gutter_is_absent_when_toggled_off() {
        let deck = deck_with("one\ntwo\nthree\n");
        // With the gutter off, the text column starts right after "N ".
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &deck, &theme(), PaneView::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row0: String = (0..40)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();
        assert!(row0.trim_start().starts_with("1 one"));
    }

    #[test]
    fn syntax_colors_apply_to_unghosted_lines() {
        let mut deck = deck_with("one\ntwo\nthree\n");
        let a = Color::Rgb(200, 100, 50);
        let b = Color::Rgb(50, 200, 100);
        deck.highlighted = Some(vec![
            vec![("on".to_string(), a), ("e".to_string(), b)],
            vec![],
            vec![],
        ]);

        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &deck, &theme(), PaneView::default()))
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
        let mut deck = deck_with_ghost(Direction::Forward, GHOST_MAX_DECAY);
        let syntax_color = Color::Rgb(10, 20, 30);
        deck.highlighted = Some(vec![
            vec![],
            vec![("two".to_string(), syntax_color)],
            vec![],
        ]);

        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &deck, &theme(), PaneView::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cell = buffer[(2, 1)].fg;
        assert_ne!(cell, syntax_color, "ghost glow must override syntax color");
        assert!(
            matches!(cell, Color::Rgb(r, _, b) if r > b),
            "forward ghost should read red: {cell:?}"
        );
    }

    fn full_text(deck: &Deck, scoped: bool) -> String {
        let mut terminal = Terminal::new(TestBackend::new(24, 6)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    deck,
                    &theme(),
                    PaneView {
                        scoped,
                        ..PaneView::default()
                    },
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..6)
            .flat_map(|y| (0..24).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn scoped_pane_clamps_to_the_function_range() {
        let mut deck = deck_with("l0\nl1\nl2\nl3\nl4\n");
        deck.scope_range = Some((1, 2)); // show only lines 1..=2

        let text = full_text(&deck, true);
        assert!(text.contains("l1"), "{text}");
        assert!(text.contains("l2"), "{text}");
        assert!(!text.contains("l0"), "{text}");
        assert!(!text.contains("l3"), "{text}");
        // Original line numbers are preserved (2 and 3, not renumbered from 1).
        assert!(text.contains("2 l1"), "{text}");
        assert!(text.contains("3 l2"), "{text}");

        // Unscoped, the same deck shows the whole file.
        let whole = full_text(&deck, false);
        assert!(whole.contains("l0") && whole.contains("l4"), "{whole}");
    }

    #[test]
    fn scoped_pane_shows_placeholder_when_the_function_is_absent() {
        let mut deck = deck_with("a\nb\n");
        deck.scope_range = None; // function not present at this commit
        let text = full_text(&deck, true);
        // "exist" is a single word, so line-wrap padding can't split it.
        assert!(text.contains("exist"), "{text}");
        assert!(!text.contains("1 a"), "{text}");
    }

    #[test]
    fn cold_open_fade_dims_text_then_normal_color_returns() {
        let deck = deck_with(
            "one
two
three
",
        );
        let normal = fg_at(&deck, 0);

        // fade at t=0: dimmer than the normal foreground (compare luminance).
        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &deck,
                    &theme(),
                    PaneView {
                        fade: Some(0.0),
                        ..PaneView::default()
                    },
                )
            })
            .unwrap();
        let faded = terminal.backend().buffer()[(2, 0)].fg;

        let luma = |c: Color| match c {
            Color::Rgb(r, g, b) => u32::from(r) + u32::from(g) + u32::from(b),
            _ => 0,
        };
        assert!(
            luma(faded) < luma(normal),
            "fade(0) should be dimmer: {faded:?} vs {normal:?}"
        );
    }

    #[test]
    fn turnstile_washes_the_pane_in_the_new_direction_hue() {
        let mut deck = deck_with(
            "one
two
three
",
        );
        deck.direction = Direction::Forward;
        deck.turnstile = crate::app::TURNSTILE_FRAMES;

        let washed = fg_at(&deck, 0); // line 0 has no ghost; wash must color it
        assert!(
            matches!(washed, Color::Rgb(r, _, b) if r > b),
            "forward turnstile should wash red: {washed:?}"
        );

        deck.turnstile = 0;
        let after = fg_at(&deck, 0);
        assert!(
            matches!(after, Color::Rgb(r, _, b) if r == b || (i16::from(r) - i16::from(b)).abs() < 40),
            "wash should clear when the turnstile finishes: {after:?}"
        );
    }

    #[test]
    fn word_level_ghost_burns_changed_words_hotter_than_the_rest() {
        // "hello there" -> "hello world": only "world" (cols 6..) is hot.
        let mut deck = deck_with(
            "hello world
",
        );
        deck.direction = Direction::Forward;
        deck.ghosts = crate::diff::compute_ghosts(
            "hello there
",
            "hello world
",
            &Ghosts::default(),
        );

        let mut terminal = Terminal::new(TestBackend::new(24, 2)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &deck, &theme(), PaneView::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // Gutter "N " is 2 cols: 'h' of "hello" at col 2, 'w' of "world" at col 8.
        let redness = |c: Color| match c {
            Color::Rgb(r, _, b) => i16::from(r) - i16::from(b),
            _ => 0,
        };
        let cold = redness(buffer[(2, 0)].fg); // unchanged word — half-glow
        let hot = redness(buffer[(8, 0)].fg); // changed word — full glow
        assert!(
            hot > cold + 20,
            "changed word should burn hotter: hot={hot} cold={cold}"
        );
        assert!(cold > 0, "the rest of the line still reads red: {cold}");
    }
}
