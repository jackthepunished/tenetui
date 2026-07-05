# tenetui — M6 Plan: the space-time map (`entropy`)

Status: in progress. Origin: a maintainer mockup (a sci-fi "space-time grid"
console). Direction confirmed 2026-07-05: **translate the mockup into the Tenet
identity** (red=forward / blue=inverted / steel base, real data only — the
multi-hue palette and fictional gauges from the mockup are not adopted), and
build it as **one new full-screen view** toggled with `m`, leaving the player,
pincer, and overview untouched.

The mockup's own caption — "You are here. The anchor point. All paths radiate
from this moment." — is already this app's thesis (the playhead-centered
diverging map from the visual-identity ADR). The map screen renders that thesis
literally.

## What it is

A braille-canvas view of the file's history as a gravity-well grid centered on
the playhead:

- **Warped grid** — concentric rings + radial spokes in dim steel, drawn with
  ratatui's `Canvas` + `Marker::Braille` (2×4 sub-cell dots per character).
- **Nodes** — commits plotted around the center: past commits on the left in
  blue, future on the right in red; radial distance grows with distance from
  the playhead; churn drives color intensity. Tags (▲) and merges (◆) always
  plotted as landmarks.
- **Radiating paths** — dashed bezier arcs from the nearest commits to the
  center pivot, in the node's direction color: "all paths radiate from this
  moment," literally.
- **The pivot** — a white-hot cluster at the origin with the playhead's date
  and a quiet "you are here."
- **Event details panel** (right) — the playhead commit's real data: short oid,
  author, date, summary, +/- churn, path-at-commit, position n/total.
- The existing header, pincer timeline strip, and status bar stay — scrubbing
  (`h`/`l`), jumps, and playback all keep working, re-anchoring the whole web
  around the new "now" each move. Playback animating the map is the demo shot.

## Constraints that shaped it

- **Color is per cell**, even with braille sub-dots — two curves crossing in
  one cell share a color. Node/path layout keeps spacing to make this rare.
- **No glow/blur in terminals** — "heat" is carried by intensity ramps
  (`theme::timeline_cell`), not bloom.
- **Frame budget** — everything painted is pure math over `AppState` (bezier
  sampling + point plots, no I/O, no diffing); node count is capped (nearest
  ~120 + landmarks) so huge histories don't flood the canvas.
- **Identity** — colors only via the `theme` module; no new saturated hues; the
  terminal background stays untouched.

## Out of scope (this pass)

- The mockup's nav sidebar / whole-app dashboard chrome (scope decision).
- Fictional gauges (temporal integrity, paradox risk). A real stats strip
  (cache hits, worker events) is a candidate follow-up if wanted.
- Pincer-aware dual-web rendering: in pincer mode the map renders the focused
  deck's view (documented; a dual-pivot map is a possible follow-up).
