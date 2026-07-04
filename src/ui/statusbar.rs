//! Status bar: where you are in history (left) and the key hints (right).
//! The right side becomes a playback indicator while playing.

use crate::app::{AppState, Direction};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let th = &state.theme;
    let sep = || Span::styled("  ·  ", Style::default().fg(th.chrome()));

    let total = state.timeline.len();
    let mut left = vec![Span::styled(
        format!("{}/{}", state.playhead + 1, total.max(1)),
        Style::default().fg(th.pivot()),
    )];

    if let Some(commit) = state.current_commit() {
        left.push(sep());
        left.push(Span::styled(
            commit.short(),
            Style::default().fg(th.chrome()),
        ));
        left.push(Span::styled(
            format!(" {}", commit.summary),
            Style::default().fg(th.foreground()),
        ));
        left.push(sep());
        left.push(Span::styled(
            commit.author.clone(),
            Style::default().fg(th.chrome()),
        ));
        left.push(sep());
        left.push(Span::styled(
            commit.date(),
            Style::default().fg(th.chrome()),
        ));
    } else {
        left.push(sep());
        left.push(Span::styled(
            format!("{total} commits"),
            Style::default().fg(th.foreground()),
        ));
    }

    let hint = if state.playing {
        let (arrow, label, color) = match state.direction {
            Direction::Forward => ("▶", "forward", th.forward()),
            Direction::Backward => ("◀", "inverted", th.inverted()),
        };
        Line::from(vec![
            Span::styled(
                format!("{arrow} playing {label}"),
                Style::default().fg(color),
            ),
            Span::styled("  ·  space pause", Style::default().fg(th.chrome())),
        ])
    } else {
        Line::from(Span::styled(
            "h/l scrub · space play · j/k scroll · q quit",
            Style::default().fg(th.chrome()),
        ))
    };

    frame.render_widget(
        Paragraph::new(Line::from(left)).alignment(Alignment::Left),
        area,
    );
    frame.render_widget(Paragraph::new(hint).alignment(Alignment::Right), area);
}
