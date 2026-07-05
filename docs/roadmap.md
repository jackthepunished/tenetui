# tenetui — Roadmap

Milestones are ordered; each produces something runnable. Claude Code: tick checkboxes as items land, and don't start a milestone before the previous one's acceptance criteria pass.

## M0 — Skeleton (walking app)

- [x] Cargo project with ratatui + crossterm event loop, clean shutdown, terminal restore on panic
- [x] CLI args: repo path + file path (clap), friendly error if not a git repo
- [x] `repo::timeline()` — walk history for the file via git2, return `Vec<CommitMeta>` (oid, time, author, summary, insertions, deletions)
- [x] Static render: file content at HEAD in main pane, commit count in status bar

**Accept:** `cargo run -- . src/main.rs` opens, shows the file, quits with `q`, never leaves the terminal broken.

## M1 — Timeline + scrubbing (core loop)

- [x] Timeline widget: commits as heatmap cells, churn → color intensity, playhead cursor
- [x] `h`/`l` move playhead one commit; main pane re-renders file at that commit
- [x] Snapshot materialization via git2 tree lookup, LRU cache (`lru` crate)
- [x] Status bar: commit summary, author, date, position (n/total)

**Accept:** scrubbing a 1k-commit file feels instant (<16 ms cached); no flicker.

## M2 — Playback + ghosting (the demo)

- [x] `space` toggles playback; `+`/`-` adjust speed; playhead animates via tick events
- [x] Line diff between consecutive snapshots (`similar`), changed lines glow and decay over ~5 steps
- [x] Background prefetch thread warms cache ±20 commits around playhead
- [x] Auto-scroll: viewport follows the region with the most recent changes during playback

**Accept:** a screen recording of playback on a real repo is legible and smooth — this is the README GIF.

## M3 — Blame gutter + navigation

- [x] Blame gutter (toggle `B` — see docs/decisions.md for the `b` key collision this resolves): author + relative age per line, computed async on scrub pause
- [x] Jump motions: `w`/`b` by day, `{`/`}` by week, `g`/`G` first/last, `/` fuzzy-search commit messages
- [x] Tag and merge markers on the timeline

**Accept:** blame never blocks scrubbing; navigation works on linux.git without stalls.

## M4 — Polish + release

- [x] Syntax highlighting (syntect), theme respects terminal colors
- [x] Config file (`~/.config/tenetui/config.toml`): keybinds, speed, cache size
- [x] Help overlay (`?`), README with GIF, `cargo install tenetui` published to crates.io — help overlay + README + `demo.tape` + LICENSE files + `publish --dry-run` done; the maintainer runs `vhs demo.tape` and `cargo publish` (needs a real TTY + crates.io token)
- [x] Criterion benches for snapshot + diff hot paths; CI (fmt, clippy -D warnings, test)

**Accept:** a stranger can install and use it without reading anything but `?`.

## M5 — Stretch (post-v1, unordered)

- [x] Function-level tracking (`git log -L` equivalent via tree-sitter ranges) — `F` picker + scoped file pane behind the `functions` feature; timeline-collapse deferred (see decisions.md)
- [x] "Volatile files" overview screen: repo-wide churn ranking as entry point
- [x] Rename following across file moves
- [x] Temporal pincer mode: two playheads side-by-side, one scrubbing forward and one inverted, compare eras

## M6 — The space-time map (post-v1, from a maintainer mockup)

- [x] `m` toggles a braille-canvas map: warped gravity-well grid centered on the playhead, past commits left/blue, future right/red, churn as intensity, tag/merge landmarks, dashed paths radiating to the white-hot "you are here" pivot, real event-details panel
- [ ] Real stats strip (cache hits / worker events) as an optional follow-up
- [ ] Dual-pivot map in pincer mode (currently renders the focused deck)

**Accept:** the map re-anchors live while scrubbing/playing; everything on it is real data in the Tenet palette.

