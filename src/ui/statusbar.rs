//! Status bar: where you are in history (left) and the key hints (right).
//! The right side becomes a playback indicator while playing.

use crate::app::{AppState, Direction};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Render `left` and `right` into their own sub-rects of `area`, `right`
/// getting exactly the columns its content needs (clamped to the area) and
/// `left` the remainder. Two `Paragraph`s sharing one `Rect` would otherwise
/// let a long right-aligned line overwrite the left side on a narrow
/// terminal — splitting the space is what actually prevents that, not just
/// hoping the terminal is wide enough.
fn render_split(frame: &mut Frame, area: Rect, left: Line, right: Line) {
    let right_width = (right.width() as u16).min(area.width);
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).areas(area);
    frame.render_widget(Paragraph::new(left).alignment(Alignment::Left), left_area);
    frame.render_widget(
        Paragraph::new(right).alignment(Alignment::Right),
        right_area,
    );
}

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let th = &state.theme;
    let sep = || Span::styled("  ·  ", Style::default().fg(th.chrome()));

    // Search mode takes over the whole bar: the query being typed (left) and
    // how to confirm/cancel it (right) are the only things that matter here.
    if let Some(query) = &state.search {
        let left = Line::from(vec![
            Span::styled("/", Style::default().fg(th.pivot())),
            Span::styled(query.clone(), Style::default().fg(th.foreground())),
        ]);
        let hint = Line::from(Span::styled(
            "enter jump · esc cancel",
            Style::default().fg(th.chrome()),
        ));
        render_split(frame, area, left, hint);
        return;
    }

    let total = state.timeline.len();
    let mut left = vec![Span::styled(
        format!("{}/{}", state.focused().playhead + 1, total.max(1)),
        Style::default().fg(th.pivot()),
    )];

    // In pincer mode, name which pane has focus (and thus what h/l scrubs).
    if state.pincer {
        let (label, color) = match state.focused().direction {
            Direction::Forward => ("▶ forward", th.forward()),
            Direction::Backward => ("◀ inverted", th.inverted()),
        };
        left.push(sep());
        left.push(Span::styled(label, Style::default().fg(color)));
    }

    // When scoped to a function, name it.
    if let Some(name) = &state.scope {
        left.push(sep());
        left.push(Span::styled(
            format!("ƒ {name}"),
            Style::default().fg(th.pivot()),
        ));
    }

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
        let (label, color) = if state.pincer {
            ("⇄ pincer", th.pivot())
        } else {
            match state.focused().direction {
                Direction::Forward => ("▶ forward", th.forward()),
                Direction::Backward => ("◀ inverted", th.inverted()),
            }
        };
        Line::from(vec![
            Span::styled(format!("{label} playing"), Style::default().fg(color)),
            Span::styled("  ·  space pause", Style::default().fg(th.chrome())),
        ])
    } else if state.pincer {
        Line::from(Span::styled(
            "h/l scrub · tab focus · space pincer · t exit · ? help",
            Style::default().fg(th.chrome()),
        ))
    } else if state.scope.is_some() {
        Line::from(Span::styled(
            "h/l scrub · space play · esc unscope · ? help",
            Style::default().fg(th.chrome()),
        ))
    } else {
        Line::from(Span::styled(
            "h/l scrub · space play · m map · B blame · / search · ? help",
            Style::default().fg(th.chrome()),
        ))
    };

    render_split(frame, area, Line::from(left), hint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app;
    use crate::repo::{CommitMeta, Snapshot};
    use crate::theme::{ColorDepth, Theme};
    use git2::Oid;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn state() -> AppState {
        AppState::new(
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
        )
    }

    const TEST_WIDTH: u16 = 80; // a realistic terminal width, not 40 — see render_split

    fn row_text(state: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(TEST_WIDTH, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..TEST_WIDTH)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect()
    }

    #[test]
    fn search_mode_replaces_the_bar_with_the_query() {
        let mut s = state();
        app::search_start(&mut s);
        app::search_type(&mut s, 'f');
        app::search_type(&mut s, 'i');
        app::search_type(&mut s, 'x');

        let text = row_text(&s);
        assert!(text.contains("/fix"), "{text:?}");
        assert!(text.contains("enter jump"), "{text:?}");
        // The normal position indicator must not leak through while searching.
        assert!(!text.contains("1/1"), "{text:?}");
    }

    #[test]
    fn normal_mode_shows_position_and_hints() {
        let text = row_text(&state());
        assert!(text.contains("1/1"), "{text:?}");
        assert!(text.contains("B blame"), "{text:?}");
    }

    /// On a too-narrow terminal, the left and right sides must never corrupt
    /// each other by drawing into the same cells — `render_split` gives the
    /// hint its own sub-rect specifically to prevent that. Regression test for
    /// a real bug caught while writing M3: two `Paragraph`s sharing one `Rect`
    /// let the (longer, post-M3) hint text overwrite the position indicator.
    #[test]
    fn narrow_terminal_never_corrupts_either_side() {
        let mut terminal = Terminal::new(TestBackend::new(10, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..10)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();

        // Too narrow for both: the hint wins the whole row, but whatever shows
        // must be a clean prefix of the real hint, not an interleaved garble.
        assert!(
            "h/l scrub · space play · B blame · / search · q quit".starts_with(text.trim_end()),
            "{text:?}"
        );
    }
}
