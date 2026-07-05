//! The timeline strip — a churn heatmap and the *pincer* color code: blue toward
//! the past (left of the playhead), red toward the future (right), white pivot at
//! the playhead. Block height encodes churn magnitude, hue encodes direction.
//! Scrubbing (`h`/`l`, wired in `main::scrub`) moves `state.playhead`, which
//! sweeps this red/blue boundary along the bar.
//!
//! Tag and merge commits are landmarks, not churn data — a tagged bucket
//! overrides its glyph/color entirely (a white "▲", unmissable regardless of
//! position); a merge bucket keeps its normal direction color but swaps in a
//! "◆" so merge points are identifiable at a glance. Tag takes priority when a
//! bucket contains both (rare, and tags are the rarer, more significant event).

use crate::app::{AppState, HEAT_MAX};
use crate::theme::Pole;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Partial-block glyphs, low → high churn. Index 0 is the "dim tick" a
/// zero-churn commit still gets — present, never invisible.
const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let th = &state.theme;
    let w = area.width as usize;
    let n = state.timeline.len();

    if w == 0 {
        return;
    }
    if n == 0 {
        let empty = Line::from(Span::styled(
            "no history for this file",
            Style::default().fg(th.chrome()),
        ));
        frame.render_widget(Paragraph::new(empty), area);
        return;
    }

    let max_churn = state
        .timeline
        .iter()
        .map(|c| c.churn())
        .max()
        .unwrap_or(1)
        .max(1) as f32;

    // Pivot columns. In pincer mode there are two — deck 0 forward (red-hot) and
    // deck 1 inverted (blue-hot); otherwise the single playhead is a white pivot.
    let fwd_playhead = state.decks[0].playhead;
    let fwd_col = (fwd_playhead * w) / n;
    let (inv_playhead, inv_col) = if state.pincer && state.decks.len() > 1 {
        let p = state.decks[1].playhead;
        (Some(p), Some((p * w) / n))
    } else {
        (None, None)
    };

    // Palindrome cold open: columns are revealed from both edges converging
    // toward the center; unrevealed columns render as a waiting baseline.
    let reveal = state
        .intro_fade()
        .map(|t| (f64::from(t) * w as f64 / 2.0) as usize);

    let mut spans: Vec<Span> = Vec::with_capacity(w);
    for x in 0..w {
        if let Some(r) = reveal
            && x >= r
            && x < w.saturating_sub(r)
        {
            spans.push(Span::styled("▁", Style::default().fg(th.chrome())));
            continue;
        }
        // Pivot markers first, so nothing overwrites the "now" cursor(s).
        if x == fwd_col {
            let pole = if state.pincer {
                Pole::Future
            } else {
                Pole::Pivot
            };
            spans.push(Span::styled(
                "█",
                Style::default().fg(th.timeline_cell(pole, 1.0)),
            ));
            continue;
        }
        if inv_col == Some(x) {
            spans.push(Span::styled(
                "█",
                Style::default().fg(th.timeline_cell(Pole::Past, 1.0)),
            ));
            continue;
        }

        // Which commits does this column cover? (Handles n<w by stretching and
        // n>w by bucketing — same formula.)
        let lo = (x * n) / w;
        let hi = (((x + 1) * n) / w).max(lo + 1).min(n);
        let center = (lo + hi) / 2;

        let bucket_churn = state.timeline[lo..hi]
            .iter()
            .map(|c| c.churn())
            .max()
            .unwrap_or(0);
        let mut intensity = bucket_churn as f32 / max_churn;

        // Heat echo: a column the playhead recently swept through glows near
        // its pole's saturated end and cools back over ~10 frames.
        let heat = (lo..hi)
            .filter_map(|i| state.heat.get(&i))
            .max()
            .copied()
            .unwrap_or(0);
        if heat > 0 {
            intensity = intensity.max(0.95 * f32::from(heat) / f32::from(HEAT_MAX));
        }

        // Direction (hue) relative to the pivot(s). In pincer mode the span
        // *between* the two jaws is neutral steel; outside it, red toward the
        // future (past deck 0) and blue toward the past (past deck 1).
        let color = match inv_playhead {
            Some(inv) => {
                if center > fwd_playhead {
                    th.timeline_cell(Pole::Future, intensity)
                } else if center < inv {
                    th.timeline_cell(Pole::Past, intensity)
                } else {
                    th.chrome()
                }
            }
            None => {
                let pole = if center < fwd_playhead {
                    Pole::Past
                } else {
                    Pole::Future
                };
                th.timeline_cell(pole, intensity)
            }
        };

        let bucket = &state.timeline[lo..hi];
        if bucket.iter().any(|c| c.is_tagged) {
            spans.push(Span::styled(
                "▲",
                Style::default().fg(th.timeline_cell(Pole::Pivot, 1.0)),
            ));
        } else if bucket.iter().any(|c| c.is_merge) {
            spans.push(Span::styled("◆", Style::default().fg(color)));
        } else {
            let glyph = BLOCKS[((intensity * 7.0).round() as usize).min(7)];
            spans.push(Span::styled(glyph.to_string(), Style::default().fg(color)));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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

    fn meta() -> CommitMeta {
        CommitMeta {
            oid: Oid::zero(),
            time: 0,
            author: "a".into(),
            summary: "s".into(),
            insertions: 1,
            deletions: 0,
            path: "f.txt".into(),
            is_merge: false,
            is_tagged: false,
        }
    }

    /// Render the timeline alone into a 1-row buffer and pull out each column's
    /// foreground color, one per terminal cell.
    fn render_row(state: &AppState, width: u16) -> Vec<Color> {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..width).map(|x| buffer[(x, 0)].fg).collect()
    }

    fn is_red_dominant(c: Color) -> bool {
        matches!(c, Color::Rgb(r, _, b) if r > b)
    }

    fn is_blue_dominant(c: Color) -> bool {
        matches!(c, Color::Rgb(r, _, b) if b > r)
    }

    /// This is the whole visual thesis: moving the playhead must sweep the
    /// red/future | blue/past boundary along the bar, not just paint a static
    /// gradient. Width == commit count here so column index == commit index,
    /// making the boundary position exactly predictable.
    #[test]
    fn pincer_boundary_follows_the_playhead() {
        let timeline: Vec<_> = (0..10).map(|_| meta()).collect();
        let mut state = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            timeline,
            Snapshot {
                oid: Oid::zero(),
                content: "x\n".into(),
                existed: true,
            },
        );

        state.decks[0].playhead = 2;
        let at_2 = render_row(&state, 10);
        assert!(is_blue_dominant(at_2[0]), "before playhead should be blue");
        assert!(is_blue_dominant(at_2[1]), "before playhead should be blue");
        assert!(is_red_dominant(at_2[9]), "after playhead should be red");

        state.decks[0].playhead = 7;
        let at_7 = render_row(&state, 10);
        // Column 2 was on the red/future side of playhead 2; once the playhead
        // sweeps past it to 7, it must flip to blue/past.
        assert!(
            is_blue_dominant(at_7[2]),
            "column should flip red->blue once the playhead passes it"
        );
        assert!(is_red_dominant(at_7[9]), "still ahead of playhead 7");
    }

    /// Same 1-row-per-commit trick, but pulling glyphs instead of colors, to
    /// verify tag/merge landmarks render as their distinct markers rather than
    /// a churn-height block.
    fn render_symbols(state: &AppState, width: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..width)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect()
    }

    #[test]
    fn tag_and_merge_commits_render_as_distinct_landmarks() {
        let mut timeline: Vec<_> = (0..10).map(|_| meta()).collect();
        timeline[3].is_tagged = true;
        timeline[6].is_merge = true;

        let mut state = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            timeline,
            Snapshot {
                oid: Oid::zero(),
                content: "x\n".into(),
                existed: true,
            },
        );
        state.decks[0].playhead = 0; // keep the pivot off of columns 3/6 so it can't mask them

        let symbols = render_symbols(&state, 10);
        assert_eq!(symbols[3], "▲", "tagged commit should show the tag marker");
        assert_eq!(symbols[6], "◆", "merge commit should show the merge marker");
        assert!(
            !BLOCKS.iter().any(|&b| symbols[3] == b.to_string()),
            "tag marker must not be a churn block"
        );
    }

    #[test]
    fn pincer_mode_draws_a_red_and_a_blue_pivot() {
        let timeline: Vec<_> = (0..10).map(|_| meta()).collect();
        let mut state = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            timeline,
            Snapshot {
                oid: Oid::zero(),
                content: "x\n".into(),
                existed: true,
            },
        );
        // Enter pincer with the forward deck ahead (col 7) and inverted behind
        // (col 2); width == commit count so column index == playhead.
        state.pincer = true;
        state.decks.push(state.decks[0].clone());
        state.decks[0].playhead = 7; // forward, red
        state.decks[1].playhead = 2; // inverted, blue

        let row = render_row(&state, 10);
        assert!(
            is_red_dominant(row[7]),
            "forward pivot should be red: {:?}",
            row[7]
        );
        assert!(
            is_blue_dominant(row[2]),
            "inverted pivot should be blue: {:?}",
            row[2]
        );
        assert!(is_red_dominant(row[9]), "ahead of the forward jaw is red");
        assert!(is_blue_dominant(row[0]), "behind the inverted jaw is blue");
    }

    #[test]
    fn cold_open_reveals_from_both_edges_toward_the_center() {
        let timeline: Vec<_> = (0..10).map(|_| meta()).collect();
        let mut state = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            timeline,
            Snapshot {
                oid: Oid::zero(),
                content: "x
"
                .into(),
                existed: true,
            },
        );
        state.decks[0].playhead = 0;
        state.intro = crate::app::INTRO_FRAMES / 2; // halfway through the reveal

        let mut terminal = Terminal::new(TestBackend::new(10, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let syms: Vec<String> = (0..10)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();

        // Middle columns still unrevealed (waiting baseline), edges live.
        assert_eq!(syms[5], "▁", "center should still be baseline: {syms:?}");
        // Column 0 is the pivot (playhead 0) — revealed as the full block.
        assert_eq!(syms[0], "█", "left edge should be revealed: {syms:?}");
        assert_ne!(syms[9], " ", "right edge should be revealed: {syms:?}");
    }

    #[test]
    fn heat_echo_boosts_a_swept_column() {
        // Column 4 has tiny churn against a high-churn field, so without heat
        // it renders near-steel — leaving visible headroom for the echo.
        let timeline: Vec<_> = (0..10)
            .map(|i| {
                let mut m = meta();
                m.insertions = if i == 4 { 1 } else { 40 };
                m
            })
            .collect();
        let mut state = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            timeline,
            Snapshot {
                oid: Oid::zero(),
                content: "x
"
                .into(),
                existed: true,
            },
        );
        state.decks[0].playhead = 9;

        let cold = render_row(&state, 10);
        state.heat.insert(4, crate::app::HEAT_MAX);
        let hot = render_row(&state, 10);

        // Column 4 (past side, blue) should be much more saturated with heat.
        let sat = |c: Color| match c {
            Color::Rgb(r, _, b) => (i16::from(b) - i16::from(r)).abs(),
            _ => 0,
        };
        assert!(
            sat(hot[4]) > sat(cold[4]) + 40,
            "heat should visibly boost saturation: cold={:?} hot={:?}",
            cold[4],
            hot[4]
        );
    }
}
