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

use crate::app::AppState;
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

    // The column the playhead lands on, so we can plant the white pivot marker.
    let playhead_col = (state.playhead * w) / n;

    let mut spans: Vec<Span> = Vec::with_capacity(w);
    for x in 0..w {
        if x == playhead_col {
            // The "now" cursor: a white-hot pivot block.
            spans.push(Span::styled(
                "█",
                Style::default().fg(th.timeline_cell(Pole::Pivot, 1.0)),
            ));
            continue;
        }

        // Which commits does this column cover? (Handles n<w by stretching and
        // n>w by bucketing — same formula.)
        let lo = (x * n) / w;
        let hi = (((x + 1) * n) / w).max(lo + 1).min(n);

        let bucket_churn = state.timeline[lo..hi]
            .iter()
            .map(|c| c.churn())
            .max()
            .unwrap_or(0);
        let intensity = bucket_churn as f32 / max_churn;

        // Direction relative to the playhead: earlier = past/blue, later = future/red.
        let center = (lo + hi) / 2;
        let pole = if center < state.playhead {
            Pole::Past
        } else {
            Pole::Future
        };

        let bucket = &state.timeline[lo..hi];
        if bucket.iter().any(|c| c.is_tagged) {
            spans.push(Span::styled(
                "▲",
                Style::default().fg(th.timeline_cell(Pole::Pivot, 1.0)),
            ));
        } else if bucket.iter().any(|c| c.is_merge) {
            spans.push(Span::styled(
                "◆",
                Style::default().fg(th.timeline_cell(pole, intensity)),
            ));
        } else {
            let glyph = BLOCKS[((intensity * 7.0).round() as usize).min(7)];
            spans.push(Span::styled(
                glyph.to_string(),
                Style::default().fg(th.timeline_cell(pole, intensity)),
            ));
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

        state.playhead = 2;
        let at_2 = render_row(&state, 10);
        assert!(is_blue_dominant(at_2[0]), "before playhead should be blue");
        assert!(is_blue_dominant(at_2[1]), "before playhead should be blue");
        assert!(is_red_dominant(at_2[9]), "after playhead should be red");

        state.playhead = 7;
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
        state.playhead = 0; // keep the pivot off of columns 3/6 so it can't mask them

        let symbols = render_symbols(&state, 10);
        assert_eq!(symbols[3], "▲", "tagged commit should show the tag marker");
        assert_eq!(symbols[6], "◆", "merge commit should show the merge marker");
        assert!(
            !BLOCKS.iter().any(|&b| symbols[3] == b.to_string()),
            "tag marker must not be a churn block"
        );
    }
}
