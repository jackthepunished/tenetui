//! Rendering. Every function here is pure: it reads `&AppState` and writes to the
//! frame, never mutating state and never doing I/O or diffing (see the frame-budget
//! rules in CLAUDE.md). All color comes from `state.theme`.

mod filepane;
mod help;
mod map;
pub mod overview;
mod picker;
mod statusbar;
mod timeline;

use crate::app::AppState;
use crate::input::Keymap;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Top-level frame composition: header · file pane(s) · timeline · status bar,
/// plus the help overlay on top when toggled. `keymap` is read-only, threaded in
/// only so the help overlay can render the live (possibly reconfigured) bindings.
pub fn draw(frame: &mut Frame, state: &AppState, keymap: &Keymap) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header / wordmark
        Constraint::Min(1),    // file pane(s)
        Constraint::Length(1), // timeline strip
        Constraint::Length(1), // status bar
    ])
    .split(frame.area());

    header(frame, chunks[0], state);
    if state.map_visible {
        // The space-time map replaces the file pane(s); the pincer timeline and
        // status bar stay, so scrubbing context never disappears.
        map::render(frame, chunks[1], state);
    } else {
        file_panes(frame, chunks[1], state);
    }
    timeline::render(frame, chunks[2], state);
    statusbar::render(frame, chunks[3], state);

    if let Some(picker) = &state.picker {
        picker::render(frame, frame.area(), picker, &state.theme);
    }
    if state.help_visible {
        help::render(frame, frame.area(), state, keymap);
    }
}

/// One pane in normal mode; two side-by-side in pincer mode (forward-red left,
/// inverted-blue right) with a thin divider between them.
fn file_panes(frame: &mut Frame, area: Rect, state: &AppState) {
    if state.pincer && state.decks.len() > 1 {
        let [left, divider, right] = Layout::horizontal([
            Constraint::Percentage(50),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(area);
        render_deck(frame, left, state, 0);
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(state.theme.chrome())),
            divider,
        );
        render_deck(frame, right, state, 1);
    } else {
        render_deck(frame, area, state, 0);
    }
}

fn render_deck(frame: &mut Frame, area: Rect, state: &AppState, deck: usize) {
    // Blame belongs to the focused deck only.
    let focused = deck == state.focus;
    let show_blame = state.blame_visible && focused;
    let blame = if focused {
        state.blame.as_deref()
    } else {
        None
    };
    filepane::render(
        frame,
        area,
        &state.decks[deck],
        &state.theme,
        show_blame,
        blame,
        state.scope.is_some(),
    );
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
        // The path *at the playhead* — surfaces the file's former name when you
        // scrub back across a rename.
        Span::styled(
            state.current_path().to_string(),
            Style::default().fg(th.chrome()),
        ),
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
            path: "src/lib.rs".into(),
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

        let keymap = Keymap::default();
        // Wide enough that the (long) status-bar hint doesn't truncate the
        // left-side commit summary via render_split.
        let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
        terminal.draw(|frame| draw(frame, &state, &keymap)).unwrap();

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
