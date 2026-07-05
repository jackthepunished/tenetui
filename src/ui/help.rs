//! The `?` help overlay: a centered modal listing the *live* keymap, so it
//! reflects any config rebinds. Pure render over a `Clear`ed rect; the event
//! loop makes it modal.

use crate::app::AppState;
use crate::input::{Action, Keymap};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

const WIDTH: u16 = 46;
/// Keys column width, so the descriptions line up.
const KEYS_COL: usize = 14;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState, keymap: &Keymap) {
    let th = &state.theme;

    let mut lines: Vec<Line> = Vec::new();
    for action in Action::ALL {
        let keys = keymap
            .keys_for(action)
            .into_iter()
            .map(|c| c.display())
            .collect::<Vec<_>>()
            .join(" / ");
        lines.push(Line::from(vec![
            Span::styled(
                format!("{keys:>KEYS_COL$}"),
                Style::default().fg(th.forward()),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                action.describe().to_string(),
                Style::default().fg(th.foreground()),
            ),
        ]));
    }

    // +2 for the border rows; the modal is as tall as its content needs.
    let height = (lines.len() as u16) + 2;
    let rect = centered(area, WIDTH, height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(th.chrome()))
        .title(Span::styled(
            " tenetui — keys ",
            Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD),
        ));

    // Clear whatever is underneath so the overlay reads as a distinct surface.
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
    use crate::repo::{CommitMeta, Snapshot};
    use crate::theme::{ColorDepth, Theme};
    use git2::Oid;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn state() -> AppState {
        let mut s = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.txt".into(),
            vec![CommitMeta {
                oid: Oid::zero(),
                time: 0,
                author: "a".into(),
                summary: "s".into(),
                insertions: 0,
                deletions: 0,
                path: "f.txt".into(),
                is_merge: false,
                is_tagged: false,
            }],
            Snapshot {
                oid: Oid::zero(),
                content: "x\n".into(),
                existed: true,
            },
        );
        s.help_visible = true;
        s
    }

    #[test]
    fn overlay_lists_bindings_and_descriptions() {
        let km = Keymap::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state(), &km))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..30)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol().to_string())
            .collect();

        assert!(text.contains("tenetui — keys"), "title missing");
        assert!(text.contains("play / pause"), "description missing");
        assert!(text.contains("scrub forward"), "description missing");
        // A rebindable key label shows up (space for play/pause).
        assert!(text.contains("space"), "key label missing");
    }
}
