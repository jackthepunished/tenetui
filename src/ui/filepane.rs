//! The main pane: the file exactly as it existed at the playhead, with a
//! line-number gutter. Ghost highlighting and the blame gutter arrive in M2/M3;
//! for now it's a quiet, scrollable, syntax-neutral view.

use crate::app::AppState;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

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

    let total = state.current.content.lines().count().max(1);
    let width = total.to_string().len();

    let lines: Vec<Line> = state
        .current
        .content
        .lines()
        .enumerate()
        .map(|(i, text)| {
            Line::from(vec![
                Span::styled(format!("{:>width$} ", i + 1, width = width), gutter),
                Span::styled(text.to_string(), fg),
            ])
        })
        .collect();

    // No wrap: code keeps its columns; long lines truncate at the edge.
    let paragraph = Paragraph::new(lines).scroll((state.scroll, 0));
    frame.render_widget(paragraph, area);
}
