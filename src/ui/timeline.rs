//! The timeline strip — a churn heatmap and the *pincer* color code: blue toward
//! the past (left of the playhead), red toward the future (right), white pivot at
//! the playhead. Block height encodes churn magnitude, hue encodes direction.
//!
//! M0 renders it static (playhead pinned at HEAD, so history reads all-inverted /
//! blue behind you). Interactive scrubbing that sweeps the red/blue boundary is M1.

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

        let glyph = BLOCKS[((intensity * 7.0).round() as usize).min(7)];
        spans.push(Span::styled(
            glyph.to_string(),
            Style::default().fg(th.timeline_cell(pole, intensity)),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
