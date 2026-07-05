//! The volatile-files overview: the entry screen when no file is given. A ranked
//! list of the repo's hottest files; `Enter` drops into the player on the
//! selection. Churn bars are steel-only — red/blue stays reserved for
//! time-direction inside the player, not repo stats here.

use crate::repo::FileChurn;
use crate::repo::blame::format_age;
use crate::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

const BAR_WIDTH: usize = 14;
const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// The overview's own small state — it doesn't share `AppState` (the player's),
/// per the two-screen split in `main`.
pub struct OverviewState {
    pub files: Vec<FileChurn>,
    pub selected: usize,
    pub theme: Theme,
    /// "Now" in Unix seconds, captured once so ages render consistently.
    pub now: i64,
}

impl OverviewState {
    pub fn new(files: Vec<FileChurn>, theme: Theme, now: i64) -> Self {
        OverviewState {
            files,
            selected: 0,
            theme,
            now,
        }
    }

    pub fn select_down(&mut self) {
        if self.selected + 1 < self.files.len() {
            self.selected += 1;
        }
    }

    pub fn select_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.files.len().saturating_sub(1);
    }

    /// The repo-relative path of the highlighted file, if any.
    pub fn selected_path(&self) -> Option<&str> {
        self.files.get(self.selected).map(|f| f.path.as_str())
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &OverviewState) {
    let th = &state.theme;
    let [header, list, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    // Header wordmark + screen name.
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▶ ", Style::default().fg(th.forward())),
            Span::styled(
                "tenetui",
                Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ◀   volatile files", Style::default().fg(th.chrome())),
        ])),
        header,
    );

    if state.files.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no file history found in this repository",
                Style::default().fg(th.chrome()),
            ))
            .alignment(Alignment::Center),
            list,
        );
        return;
    }

    let max_churn = state
        .files
        .iter()
        .map(|f| f.churn)
        .max()
        .unwrap_or(1)
        .max(1);

    // Keep the selection visible: scroll the window so `selected` is on-screen.
    let h = list.height as usize;
    let offset = state
        .selected
        .saturating_sub(h.saturating_sub(1))
        .min(state.files.len().saturating_sub(h.max(1)));

    let rows: Vec<Line> = state
        .files
        .iter()
        .enumerate()
        .skip(offset)
        .take(h)
        .map(|(i, f)| row(th, i, f, state.selected == i, max_churn, state.now))
        .collect();
    frame.render_widget(Paragraph::new(rows), list);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "j/k select · enter open · q quit",
            Style::default().fg(th.chrome()),
        ))
        .alignment(Alignment::Right),
        footer,
    );
}

fn row(
    th: &Theme,
    rank: usize,
    f: &FileChurn,
    selected: bool,
    max_churn: usize,
    now: i64,
) -> Line<'static> {
    let bar = churn_bar(f.churn, max_churn);
    let age = format_age((now - f.last_time).max(0) / 86_400);

    let marker = if selected { "▸ " } else { "  " };
    let path_style = if selected {
        Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(th.foreground())
    };

    Line::from(vec![
        Span::styled(marker, Style::default().fg(th.forward())),
        Span::styled(
            format!("{:>2}  ", rank + 1),
            Style::default().fg(th.chrome()),
        ),
        Span::styled(bar, Style::default().fg(th.foreground())),
        Span::styled(
            format!("  {:>6}  {:>3}× {:>5}  ", f.churn, f.touches, age),
            Style::default().fg(th.chrome()),
        ),
        Span::styled(f.path.clone(), path_style),
    ])
}

/// A fixed-width churn bar using partial blocks for sub-cell resolution.
fn churn_bar(churn: usize, max_churn: usize) -> String {
    let filled = (churn as f32 / max_churn as f32 * BAR_WIDTH as f32).min(BAR_WIDTH as f32);
    let full = filled.floor() as usize;
    let mut bar = "█".repeat(full);
    let frac = filled - full as f32;
    if full < BAR_WIDTH && frac > 0.0 {
        let idx = ((frac * 8.0).round() as usize).clamp(1, 8) - 1;
        bar.push(BLOCKS[idx]);
    }
    // Pad to a constant column width.
    format!("{bar:<BAR_WIDTH$}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ColorDepth;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn state() -> OverviewState {
        let files = vec![
            FileChurn {
                path: "src/hot.rs".into(),
                touches: 40,
                churn: 900,
                last_time: 0,
            },
            FileChurn {
                path: "src/mid.rs".into(),
                touches: 12,
                churn: 200,
                last_time: 0,
            },
            FileChurn {
                path: "src/cold.rs".into(),
                touches: 2,
                churn: 20,
                last_time: 0,
            },
        ];
        OverviewState::new(
            files,
            Theme {
                depth: ColorDepth::TrueColor,
            },
            900_000,
        )
    }

    fn rendered(state: &OverviewState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut s = String::new();
        for y in 0..10 {
            for x in 0..80 {
                s.push_str(buffer[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn lists_files_ranked_with_bars_and_ages() {
        let text = rendered(&state());
        assert!(text.contains("volatile files"), "{text}");
        assert!(text.contains("src/hot.rs"), "{text}");
        assert!(text.contains("src/cold.rs"), "{text}");
        assert!(text.contains("█"), "churn bar missing:\n{text}");
        // ~10 days since last_time=0 (now=900000s) → the "1w" bucket.
        assert!(text.contains("1w"), "age missing:\n{text}");
    }

    #[test]
    fn selection_moves_and_clamps() {
        let mut s = state();
        assert_eq!(s.selected_path(), Some("src/hot.rs"));
        s.select_up(); // already at top
        assert_eq!(s.selected, 0);
        s.select_down();
        assert_eq!(s.selected_path(), Some("src/mid.rs"));
        s.select_last();
        assert_eq!(s.selected_path(), Some("src/cold.rs"));
        s.select_down(); // clamps at bottom
        assert_eq!(s.selected_path(), Some("src/cold.rs"));
    }

    #[test]
    fn empty_repo_shows_a_message_not_a_crash() {
        let empty = OverviewState::new(
            vec![],
            Theme {
                depth: ColorDepth::TrueColor,
            },
            0,
        );
        let text = rendered(&empty);
        assert!(text.contains("no file history"), "{text}");
    }
}
