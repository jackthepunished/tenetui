//! The space-time map (`m`): the file's history as a gravity-well grid centered
//! on the playhead — the visual-identity thesis rendered literally. "You are
//! here. All paths radiate from this moment."
//!
//! Drawn with ratatui's braille canvas (2×4 sub-cell dots per character): a
//! warped ring-and-spoke grid in dim steel, commits plotted as nodes — past
//! left/blue, future right/red, churn as intensity, tags/merges as landmarks —
//! and dashed arcs radiating from the nearest commits to the white-hot pivot.
//! Pure math over `AppState` (no I/O, no diffing); node count is capped so a
//! 10k-commit history doesn't flood the canvas.
//!
//! One braille cell has a single foreground color, so two paths crossing in a
//! cell share it — the layout spreads nodes to keep that rare, not impossible.

use crate::app::AppState;
use crate::repo::CommitMeta;
use crate::theme::{Pole, Theme};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Context, Points};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

/// Canvas coordinate bounds (x is wider: terminal cells are ~2:1 tall).
const X_MAX: f64 = 1.7;
const Y_MAX: f64 = 1.05;
/// Vertical squash for the "seen at an angle" grid perspective.
const SQUASH: f64 = 0.72;
/// At most this many commit nodes are plotted (nearest to the playhead first;
/// tag/merge landmarks are always kept).
const MAX_NODES: usize = 120;
/// The nearest N nodes get a dashed path radiating to the pivot.
const MAX_PATHS: usize = 14;
/// The nearest N nodes get a date label.
const MAX_LABELS: usize = 6;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let [canvas_area, details_area] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(30)]).areas(area);

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([-X_MAX, X_MAX])
        .y_bounds([-Y_MAX, Y_MAX])
        .paint(|ctx| paint(ctx, state));
    frame.render_widget(canvas, canvas_area);

    details(frame, details_area, state);
}

fn paint(ctx: &mut Context, state: &AppState) {
    let th = &state.theme;
    grid(ctx, th);

    let n = state.timeline.len();
    if n == 0 {
        ctx.print(
            -0.4,
            0.0,
            Line::from(Span::styled(
                "no history to map",
                Style::default().fg(th.chrome()),
            )),
        );
        return;
    }

    if state.pincer && state.decks.len() > 1 {
        paint_pincer(ctx, state);
    } else {
        paint_single(ctx, state);
    }
}

/// The single-pivot map: one radial gravity well centered on the playhead.
fn paint_single(ctx: &mut Context, state: &AppState) {
    let th = &state.theme;
    let n = state.timeline.len();
    let playhead = state.focused().playhead;
    let max_churn = state
        .timeline
        .iter()
        .map(|c| c.churn())
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    // Pick which commits to plot: the nearest MAX_NODES to the playhead, plus
    // every tag/merge landmark regardless of distance.
    let mut indices: Vec<usize> = (0..n).collect();
    indices.sort_by_key(|&i| i.abs_diff(playhead));
    let mut plotted: Vec<usize> = indices
        .iter()
        .copied()
        .take(MAX_NODES)
        .chain(
            indices
                .iter()
                .copied()
                .filter(|&i| state.timeline[i].is_tagged || state.timeline[i].is_merge),
        )
        .collect();
    plotted.sort_unstable();
    plotted.dedup();

    let max_dist = plotted
        .iter()
        .map(|&i| i.abs_diff(playhead))
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    // Dashed paths radiate from the nearest commits to the pivot.
    let mut by_dist = plotted.clone();
    by_dist.sort_by_key(|&i| i.abs_diff(playhead));
    for &i in by_dist.iter().skip(1).take(MAX_PATHS) {
        let (x, y) = node_pos(i, playhead, max_dist);
        let commit = &state.timeline[i];
        let intensity = 0.25 + 0.45 * (commit.churn() as f64 / max_churn) as f32;
        let color = th.timeline_cell(pole_of(i, playhead), intensity);
        dashed_arc(ctx, x, y, 0.0, 0.0, i, color);
    }

    // Nodes on top of paths.
    for &i in &plotted {
        if i == playhead {
            continue;
        }
        let commit = &state.timeline[i];
        let (x, y) = node_pos(i, playhead, max_dist);
        let intensity = (0.45 + 0.55 * (commit.churn() as f64 / max_churn)) as f32;
        let color = th.timeline_cell(pole_of(i, playhead), intensity);
        node(ctx, x, y, commit, color);
    }

    // Date labels for the nearest few (skipping the pivot itself). Labels are
    // anchored *away* from the pivot (left of past nodes, right of future ones)
    // and greedily skipped when they'd land on an already-placed label, so the
    // center never turns into overlapping text.
    let mut placed: Vec<(f64, f64)> = Vec::new();
    for &i in by_dist.iter().skip(1) {
        if placed.len() >= MAX_LABELS {
            break;
        }
        let (x, y) = node_pos(i, playhead, max_dist);
        let lx = if i < playhead { x - 0.42 } else { x + 0.06 };
        let ly = y - 0.07;
        if placed
            .iter()
            .any(|&(px, py)| (px - lx).abs() < 0.55 && (py - ly).abs() < 0.13)
        {
            continue;
        }
        placed.push((lx, ly));
        let commit = &state.timeline[i];
        ctx.print(
            lx,
            ly,
            Line::from(Span::styled(
                commit.date(),
                Style::default().fg(th.chrome()),
            )),
        );
    }

    // Comet trail: where "now" recently was, fading with age. Positions are
    // re-projected against the *current* playhead, so the trail reads as the
    // path the pivot took through today's field.
    let trail = &state.focused().trail;
    for (age, &idx) in trail.iter().enumerate() {
        if idx == playhead {
            continue;
        }
        let frac = 1.0 - (age as f32 / trail.len().max(1) as f32);
        let (x, y) = node_pos(idx, playhead, max_dist);
        ctx.draw(&Points {
            coords: &[(x, y), (x + 0.014, y), (x - 0.014, y)],
            color: th.timeline_cell(Pole::Pivot, 0.25 + 0.6 * frac),
        });
    }

    // The white-hot pivot: "you are here".
    pivot(ctx, 0.0, 0.0, th.timeline_cell(Pole::Pivot, 1.0));
    if let Some(commit) = state.current_commit() {
        ctx.print(
            0.07,
            0.09,
            Line::from(Span::styled(
                commit.date(),
                Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD),
            )),
        );
        ctx.print(
            0.07,
            -0.02,
            Line::from(Span::styled(
                "you are here",
                Style::default().fg(th.chrome()),
            )),
        );
    }

    // Edge captions: the two directions of travel.
    ctx.print(
        -X_MAX + 0.06,
        Y_MAX - 0.08,
        Line::from(Span::styled("◀ PAST", Style::default().fg(th.inverted()))),
    );
    ctx.print(
        X_MAX - 0.62,
        Y_MAX - 0.08,
        Line::from(Span::styled("FUTURE ▶", Style::default().fg(th.forward()))),
    );
}

/// Past commits sit left/blue of the pivot; future right/red.
fn pole_of(i: usize, playhead: usize) -> Pole {
    if i < playhead {
        Pole::Past
    } else {
        Pole::Future
    }
}

/// Where commit `i` sits on the canvas: side by direction, radius by distance
/// from the playhead (eased so near history gets more room), fanned vertically
/// by a deterministic hash so the field reads as a constellation, not a line.
fn node_pos(i: usize, playhead: usize, max_dist: f64) -> (f64, f64) {
    let dist = i.abs_diff(playhead) as f64;
    let r = 0.34 + 0.60 * (dist / max_dist).powf(0.62);

    // Deterministic scatter in [-1, 1] — stable per commit index, no RNG.
    let angle = hash_scatter(i) * 0.85; // fan of ±~49° around the horizontal axis

    let dir = if i < playhead { -1.0 } else { 1.0 };
    let x = dir * r * angle.cos() * (X_MAX * 0.82);
    let y = r * angle.sin() * SQUASH;
    (x, y)
}

/// A deterministic per-commit scatter in [-1, 1] - stable across frames (no
/// RNG), so the constellation never shimmers between redraws.
fn hash_scatter(i: usize) -> f64 {
    let h = (i.wrapping_mul(2_654_435_761) >> 8) % 2048;
    (h as f64 / 1023.5) - 1.0
}

/// The warped ring-and-spoke grid, all dim steel. Rings crowd toward the center
/// (the gravity well); spokes are dotted by sampling with gaps.
fn grid(ctx: &mut Context, th: &Theme) {
    let color = th.chrome();
    let mut dots: Vec<(f64, f64)> = Vec::with_capacity(2048);

    for ring in 1..=5 {
        // r^1.35 packs inner rings tighter — the well.
        let r = (ring as f64 / 5.0).powf(1.35);
        let step = 3.0_f64.to_radians();
        let mut a = 0.0_f64;
        while a < std::f64::consts::TAU {
            dots.push((r * a.cos() * X_MAX * 0.92, r * a.sin() * Y_MAX * SQUASH));
            a += step;
        }
    }
    for spoke in 0..12 {
        let a = f64::from(spoke) * std::f64::consts::TAU / 12.0;
        let mut r = 0.12;
        while r < 1.0 {
            // Two dots on, one gap — a dotted radial.
            dots.push((r * a.cos() * X_MAX * 0.92, r * a.sin() * Y_MAX * SQUASH));
            r += 0.045;
        }
    }
    ctx.draw(&Points {
        coords: &dots,
        color,
    });
}

/// A commit node: a small dot cluster (bigger with churn); tags get a ▲ glyph
/// and merges a ◆, in the pivot white so landmarks stay unmissable.
fn node(ctx: &mut Context, x: f64, y: f64, commit: &CommitMeta, color: ratatui::style::Color) {
    let mut dots = vec![
        (x, y),
        (x + 0.012, y),
        (x - 0.012, y),
        (x, y + 0.018),
        (x, y - 0.018),
    ];
    if commit.churn() > 0 {
        dots.push((x + 0.024, y));
        dots.push((x - 0.024, y));
    }
    ctx.draw(&Points {
        coords: &dots,
        color,
    });
    if commit.is_tagged {
        ctx.print(
            x - 0.02,
            y + 0.05,
            Line::from(Span::styled("▲", Style::default().fg(color))),
        );
    } else if commit.is_merge {
        ctx.print(
            x - 0.02,
            y + 0.05,
            Line::from(Span::styled("◆", Style::default().fg(color))),
        );
    }
}

/// A dashed quadratic arc from a node to the origin. Curvature alternates by
/// index so neighboring paths bow apart instead of overlapping.
fn dashed_arc(
    ctx: &mut Context,
    x: f64,
    y: f64,
    tx: f64,
    ty: f64,
    i: usize,
    color: ratatui::style::Color,
) {
    let bow = if i.is_multiple_of(2) { 0.16 } else { -0.16 };
    // Control point: chord midpoint pushed perpendicular to the chord.
    let (mx, my) = ((x + tx) / 2.0, (y + ty) / 2.0);
    let (dx, dy) = (tx - x, ty - y);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (px, py) = (-dy / len, dx / len);
    let (cx, cy) = (mx + px * bow, my + py * bow);

    const SEGS: usize = 26;
    let point = |t: f64| {
        let u = 1.0 - t;
        (
            u * u * x + 2.0 * u * t * cx + t * t * tx,
            u * u * y + 2.0 * u * t * cy + t * t * ty,
        )
    };
    let mut dots: Vec<(f64, f64)> = Vec::with_capacity(SEGS);
    for k in 0..SEGS {
        // Every third sample dropped → dashes.
        if k % 3 == 2 {
            continue;
        }
        let t = k as f64 / (SEGS - 1) as f64;
        dots.push(point(t));
    }
    ctx.draw(&Points {
        coords: &dots,
        color,
    });
}

/// A hot pivot cluster at `(cx, cy)` — white for the single "now", red/blue for
/// the two pincer anchors.
fn pivot(ctx: &mut Context, cx: f64, cy: f64, color: ratatui::style::Color) {
    let mut dots = Vec::new();
    let mut a = 0.0_f64;
    while a < std::f64::consts::TAU {
        dots.push((cx + 0.03 * a.cos() * 1.6, cy + 0.03 * a.sin()));
        dots.push((cx + 0.015 * a.cos() * 1.6, cy + 0.015 * a.sin()));
        a += 0.5;
    }
    dots.push((cx, cy));
    ctx.draw(&Points {
        coords: &dots,
        color,
    });
}

/// The dual-pivot pincer map: the whole timeline laid as a horizontal spacetime
/// band, with two hot anchors — the forward deck (red) and the inverted deck
/// (blue) — and their radiating webs. During pincer playback the two anchors
/// slide apart from a shared "now": red territory grows rightward toward the
/// future, blue leftward toward the past.
fn paint_pincer(ctx: &mut Context, state: &AppState) {
    let th = &state.theme;
    let n = state.timeline.len();
    let p_fwd = state.decks[0].playhead;
    let p_inv = state.decks[1].playhead;
    let max_churn = state
        .timeline
        .iter()
        .map(|c| c.churn())
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    let near = |i: usize| i.abs_diff(p_fwd).min(i.abs_diff(p_inv));
    let mut indices: Vec<usize> = (0..n).collect();
    indices.sort_by_key(|&i| near(i));
    let mut plotted: Vec<usize> = indices
        .iter()
        .copied()
        .take(MAX_NODES)
        .chain(
            indices
                .iter()
                .copied()
                .filter(|&i| state.timeline[i].is_tagged || state.timeline[i].is_merge),
        )
        .collect();
    plotted.sort_unstable();
    plotted.dedup();
    let max_dist = plotted.iter().map(|&i| near(i)).max().unwrap_or(1).max(1) as f64;

    // Index → x across the full width; y pinches toward whichever pivot a node
    // is nearest (two wells on the band); hue = the nearer pivot's color.
    let span = (n.max(2) - 1) as f64;
    let xn = |i: usize| ((i as f64) / span - 0.5) * 2.0 * X_MAX * 0.9;
    let ypos = |i: usize| {
        let amp = 0.12 + 0.62 * (near(i) as f64 / max_dist).min(1.0);
        hash_scatter(i) * amp * Y_MAX * 0.82
    };
    let pole = |i: usize| {
        if i.abs_diff(p_fwd) <= i.abs_diff(p_inv) {
            Pole::Future
        } else {
            Pole::Past
        }
    };

    // Comet trails for both decks.
    for deck in &state.decks {
        for (age, &idx) in deck.trail.iter().enumerate() {
            if idx == deck.playhead {
                continue;
            }
            let frac = 1.0 - age as f32 / deck.trail.len().max(1) as f32;
            let (x, y) = (xn(idx), ypos(idx));
            ctx.draw(&Points {
                coords: &[(x, y), (x + 0.014, y), (x - 0.014, y)],
                color: th.timeline_cell(Pole::Pivot, 0.25 + 0.6 * frac),
            });
        }
    }

    // Dashed webs: each pivot to its nearest commits, in that pivot's color.
    let path_intensity =
        |i: usize| 0.25 + 0.45 * (state.timeline[i].churn() as f64 / max_churn) as f32;
    let mut by_fwd = plotted.clone();
    by_fwd.sort_by_key(|&i| i.abs_diff(p_fwd));
    for &i in by_fwd.iter().filter(|&&i| i != p_fwd).take(MAX_PATHS / 2) {
        dashed_arc(
            ctx,
            xn(i),
            ypos(i),
            xn(p_fwd),
            0.0,
            i,
            th.timeline_cell(Pole::Future, path_intensity(i)),
        );
    }
    let mut by_inv = plotted.clone();
    by_inv.sort_by_key(|&i| i.abs_diff(p_inv));
    for &i in by_inv.iter().filter(|&&i| i != p_inv).take(MAX_PATHS / 2) {
        dashed_arc(
            ctx,
            xn(i),
            ypos(i),
            xn(p_inv),
            0.0,
            i,
            th.timeline_cell(Pole::Past, path_intensity(i)),
        );
    }

    // Nodes, colored by their nearer anchor.
    for &i in &plotted {
        if i == p_fwd || i == p_inv {
            continue;
        }
        let commit = &state.timeline[i];
        let intensity = (0.45 + 0.55 * (commit.churn() as f64 / max_churn)) as f32;
        node(
            ctx,
            xn(i),
            ypos(i),
            commit,
            th.timeline_cell(pole(i), intensity),
        );
    }

    // The two anchors and their labels.
    pivot(ctx, xn(p_fwd), 0.0, th.timeline_cell(Pole::Future, 1.0));
    pivot(ctx, xn(p_inv), 0.0, th.timeline_cell(Pole::Past, 1.0));
    ctx.print(
        xn(p_fwd) - 0.05,
        0.13,
        Line::from(Span::styled("▶ forward", Style::default().fg(th.forward()))),
    );
    ctx.print(
        xn(p_inv) - 0.05,
        -0.15,
        Line::from(Span::styled(
            "◀ inverted",
            Style::default().fg(th.inverted()),
        )),
    );
    ctx.print(
        -X_MAX + 0.06,
        Y_MAX - 0.08,
        Line::from(Span::styled("◀ PAST", Style::default().fg(th.inverted()))),
    );
    ctx.print(
        X_MAX - 0.62,
        Y_MAX - 0.08,
        Line::from(Span::styled("FUTURE ▶", Style::default().fg(th.forward()))),
    );
}

/// The right-hand panel: the playhead commit's real data.
fn details(frame: &mut Frame, area: Rect, state: &AppState) {
    let th = &state.theme;
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(th.chrome()))
        .title(Span::styled(
            " event details ",
            Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD),
        ));

    let mut lines: Vec<Line> = Vec::new();
    if let Some(commit) = state.current_commit() {
        let label = |s: &str| Span::styled(format!("{s:<9}"), Style::default().fg(th.chrome()));
        let value = |s: String| Span::styled(s, Style::default().fg(th.foreground()));
        lines.push(Line::from(vec![
            Span::styled("◉ ", Style::default().fg(th.pivot())),
            Span::styled(
                commit.date(),
                Style::default().fg(th.pivot()).add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![label("commit"), value(commit.short())]));
        lines.push(Line::from(vec![
            label("author"),
            value(commit.author.clone()),
        ]));
        lines.push(Line::from(vec![
            label("churn"),
            Span::styled(
                format!("+{}", commit.insertions),
                Style::default().fg(th.foreground()),
            ),
            Span::styled(
                format!(" -{}", commit.deletions),
                Style::default().fg(th.chrome()),
            ),
        ]));
        lines.push(Line::from(vec![label("path"), value(commit.path.clone())]));
        lines.push(Line::from(vec![
            label("position"),
            value(format!(
                "{}/{}",
                state.focused().playhead + 1,
                state.timeline.len()
            )),
        ]));
        lines.push(Line::from(""));
        for text in [
            "you are here.",
            "the anchor point.",
            "all paths radiate",
            "from this moment.",
        ] {
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(th.chrome()),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "summary",
            Style::default().fg(th.chrome()),
        )]));
        lines.push(Line::from(Span::styled(
            commit.summary.clone(),
            Style::default().fg(th.foreground()),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "no history",
            Style::default().fg(th.chrome()),
        )));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::Snapshot;
    use crate::theme::ColorDepth;
    use git2::Oid;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    fn meta(summary: &str) -> CommitMeta {
        CommitMeta {
            oid: Oid::zero(),
            time: 1_700_000_000,
            author: "Ada".into(),
            summary: summary.into(),
            insertions: 3,
            deletions: 1,
            path: "f.rs".into(),
            is_merge: false,
            is_tagged: false,
        }
    }

    fn state(n: usize, playhead: usize) -> AppState {
        let timeline: Vec<_> = (0..n).map(|i| meta(&format!("c{i}"))).collect();
        let mut s = AppState::new(
            Theme {
                depth: ColorDepth::TrueColor,
            },
            "f.rs".into(),
            timeline,
            Snapshot {
                oid: Oid::zero(),
                content: "x\n".into(),
                existed: true,
            },
        );
        s.decks[0].playhead = playhead;
        s
    }

    fn draw(state: &AppState, w: u16, h: u16) -> (String, Vec<Vec<Color>>) {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        let mut colors = vec![vec![Color::Reset; w as usize]; h as usize];
        for y in 0..h {
            for x in 0..w {
                text.push_str(buffer[(x, y)].symbol());
                colors[y as usize][x as usize] = buffer[(x, y)].fg;
            }
            text.push('\n');
        }
        (text, colors)
    }

    #[test]
    fn map_shows_pivot_labels_and_real_details() {
        let s = state(21, 10);
        let (text, _) = draw(&s, 110, 30);
        assert!(text.contains("you are here"), "{text}");
        assert!(text.contains("event details"), "{text}");
        assert!(text.contains("11/21"), "position missing: {text}");
        assert!(text.contains("PAST"), "{text}");
        assert!(text.contains("FUTURE"), "{text}");
        assert!(text.contains("c10"), "playhead summary missing: {text}");
    }

    #[test]
    fn past_side_leans_blue_and_future_side_leans_red() {
        let s = state(41, 20);
        let (_, colors) = draw(&s, 120, 32);
        // The canvas pane is the left ~90 columns. Count hue dominance per half,
        // skipping near-neutral cells: the steel grid (64,71,84) leans slightly
        // blue and the pivot white slightly cool, so a plain r==b check would
        // count the entire grid as "blue" — require real saturation instead.
        let (mut left_blue, mut left_red, mut right_blue, mut right_red) = (0, 0, 0, 0);
        for (y, row) in colors.iter().enumerate().take(30) {
            for (x, c) in row.iter().enumerate().take(88) {
                let Color::Rgb(r, _, b) = *c else { continue };
                if (i16::from(r) - i16::from(b)).abs() <= 40 {
                    continue; // steel grid / pivot white / faint path tails
                }
                let _ = y;
                if x < 44 {
                    if b > r {
                        left_blue += 1;
                    } else {
                        left_red += 1;
                    }
                } else if b > r {
                    right_blue += 1;
                } else {
                    right_red += 1;
                }
            }
        }
        assert!(
            left_blue > left_red,
            "past half should lean blue: {left_blue} blue vs {left_red} red"
        );
        assert!(
            right_red > right_blue,
            "future half should lean red: {right_red} red vs {right_blue} blue"
        );
    }

    #[test]
    fn empty_timeline_renders_a_message_not_a_panic() {
        let s = state(0, 0);
        let (text, _) = draw(&s, 100, 24);
        assert!(text.contains("no history"), "{text}");
    }

    #[test]
    fn pincer_map_labels_both_anchors_and_splits_territory() {
        let mut s = state(41, 20);
        crate::app::update(&mut s, crate::input::Action::TogglePincer);
        s.decks[0].playhead = 34; // forward, right
        s.decks[1].playhead = 6; // inverted, left

        let (text, colors) = draw(&s, 120, 32);
        assert!(text.contains("forward"), "forward label missing: {text}");
        assert!(text.contains("inverted"), "inverted label missing: {text}");

        // Territory: the future half (right) leans red, the past half (left)
        // leans blue — the two anchors owning their ends of the band.
        let (mut left_blue, mut left_red, mut right_blue, mut right_red) = (0, 0, 0, 0);
        for row in colors.iter().take(30) {
            for (x, c) in row.iter().enumerate().take(88) {
                let Color::Rgb(r, _, b) = *c else { continue };
                if (i16::from(r) - i16::from(b)).abs() <= 40 {
                    continue;
                }
                if x < 44 {
                    if b > r {
                        left_blue += 1;
                    } else {
                        left_red += 1;
                    }
                } else if b > r {
                    right_blue += 1;
                } else {
                    right_red += 1;
                }
            }
        }
        assert!(
            left_blue > left_red,
            "past/inverted side should lean blue: {left_blue} vs {left_red}"
        );
        assert!(
            right_red > right_blue,
            "future/forward side should lean red: {right_red} vs {right_blue}"
        );
    }
}
