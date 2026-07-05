//! The `F` function picker: a centered modal listing the functions found in the
//! current snapshot; `Enter` scopes the view to the highlighted one. Rendered
//! over a `Clear`ed rect; the event loop makes it modal.

use crate::app::FunctionPicker;
use crate::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

const WIDTH: u16 = 44;
/// Cap the visible rows so a huge file's function list stays a tidy modal.
const MAX_ROWS: usize = 16;

pub fn render(frame: &mut Frame, area: Rect, picker: &FunctionPicker, th: &Theme) {
    // Scroll the window so the selection stays visible.
    let total = picker.functions.len();
    let rows = total.min(MAX_ROWS);
    let offset = picker
        .selected
        .saturating_sub(rows.saturating_sub(1))
        .min(total.saturating_sub(rows));

    let lines: Vec<Line> = picker
        .functions
        .iter()
        .enumerate()
        .skip(offset)
        .take(rows)
        .map(|(i, f)| {
            let selected = i == picker.selected;
            let marker = if selected { "▸ " } else { "  " };
            let name_style = if selected {
                Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(th.foreground())
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(th.forward())),
                Span::styled(f.name.clone(), name_style),
                Span::styled(
                    format!("  :{}", f.start_line + 1),
                    Style::default().fg(th.chrome()),
                ),
            ])
        })
        .collect();

    let height = (rows as u16) + 2; // + border rows
    let rect = centered(area, WIDTH, height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(th.chrome()))
        .title(Span::styled(
            " scope to function ",
            Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD),
        ));

    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

/// A `width`×`height` rect centered in `area`, clamped to fit.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::FunctionDef;
    use crate::theme::ColorDepth;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn lists_functions_with_the_selection_marked() {
        let picker = FunctionPicker {
            functions: vec![
                FunctionDef {
                    name: "alpha".into(),
                    start_line: 0,
                    end_line: 3,
                },
                FunctionDef {
                    name: "beta".into(),
                    start_line: 10,
                    end_line: 20,
                },
            ],
            selected: 1,
        };
        let th = Theme {
            depth: ColorDepth::TrueColor,
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &picker, &th))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..12)
            .flat_map(|y| (0..60).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol().to_string())
            .collect();

        assert!(text.contains("scope to function"), "{text}");
        assert!(text.contains("alpha"), "{text}");
        assert!(text.contains("beta"), "{text}");
        assert!(text.contains("▸ beta"), "selection marker missing:\n{text}");
        assert!(text.contains(":11"), "beta's line number missing:\n{text}");
    }
}
