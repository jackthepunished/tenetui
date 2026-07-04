//! Rendering. Every function here is pure: it reads `&AppState` and writes to the
//! frame, never mutating state and never doing I/O or diffing (see the frame-budget
//! rules in CLAUDE.md). All color comes from `state.theme`.

mod filepane;
mod statusbar;
mod timeline;

use crate::app::AppState;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Top-level frame composition: header · file pane · timeline · status bar.
pub fn draw(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header / wordmark
        Constraint::Min(1),    // file pane
        Constraint::Length(1), // timeline strip
        Constraint::Length(1), // status bar
    ])
    .split(frame.area());

    header(frame, chunks[0], state);
    filepane::render(frame, chunks[1], state);
    timeline::render(frame, chunks[2], state);
    statusbar::render(frame, chunks[3], state);
}

/// The wordmark (left) and the pincer legend (right), sharing one row.
fn header(frame: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    let th = &state.theme;

    let wordmark = Line::from(vec![
        Span::styled("▶ ", Style::default().fg(th.forward())),
        Span::styled(
            "tenetui",
            Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ◀   ", Style::default().fg(th.inverted())),
        Span::styled(state.file_path.clone(), Style::default().fg(th.chrome())),
    ]);

    // Doubles as the color key: blue is the past you're inverting toward, red the future.
    let legend = Line::from(vec![
        Span::styled("◀ inverted", Style::default().fg(th.inverted())),
        Span::styled("   forward ▶", Style::default().fg(th.forward())),
    ]);

    frame.render_widget(Paragraph::new(wordmark).alignment(Alignment::Left), area);
    frame.render_widget(Paragraph::new(legend).alignment(Alignment::Right), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::{CommitMeta, Snapshot};
    use crate::theme::{ColorDepth, Theme};
    use git2::Oid;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn meta(summary: &str) -> CommitMeta {
        CommitMeta {
            oid: Oid::zero(),
            time: 0,
            author: "Ada".into(),
            summary: summary.into(),
            insertions: 3,
            deletions: 1,
            is_merge: false,
            is_tagged: false,
        }
    }

    /// Render a whole frame to an in-memory buffer and assert the M0 deliverables
    /// actually appear: the file content, the wordmark, and the commit count.
    #[test]
    fn static_render_shows_file_and_commit_count() {
        let state = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "src/lib.rs".into(),
            vec![meta("first"), meta("second")],
            Snapshot {
                oid: Oid::zero(),
                content: "line one\nline two\n".into(),
                existed: true,
            },
        );

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|frame| draw(frame, &state)).unwrap();

        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }

        assert!(text.contains("tenetui"), "wordmark missing:\n{text}");
        assert!(text.contains("src/lib.rs"), "file path missing:\n{text}");
        assert!(text.contains("line one"), "file content missing:\n{text}");
        assert!(text.contains("line two"), "file content missing:\n{text}");
        // Playhead pins to HEAD (2nd of 2 commits).
        assert!(text.contains("2/2"), "position missing:\n{text}");
        assert!(text.contains("second"), "commit summary missing:\n{text}");
    }
}
