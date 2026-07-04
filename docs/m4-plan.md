# tenetui — M4 Implementation Plan (Polish + Release)

Status: planned, not started. This is the execution plan for the final roadmap
milestone. It expands roadmap.md's four M4 bullets into ordered, independently
committable work items. Decisions marked **[locked]** were confirmed with the
maintainer on 2026-07-05; append any that change to decisions.md.

**Accept criterion (from roadmap.md):** a stranger can `cargo install tenetui`,
run it, and use it knowing only `?`.

## Locked decisions

- **[locked] Syntax palette = custom muted, not a stock syntect theme.** Code
  tokens use only cold, desaturated tones; saturated red/blue stay reserved for
  time-direction and ghost trails (the *Tenet* identity — see decisions.md
  "Visual identity"). More work than shipping a base16 theme, chosen to keep the
  identity coherent end to end.
- **[locked] Release = build + prep; maintainer publishes.** Everything is
  implemented and packaged here (README, `vhs` tape, LICENSE files, version
  bump, `cargo publish --dry-run`), then work stops. The two steps that need the
  maintainer's environment — `vhs demo.tape` (a real TTY) and `cargo publish`
  (a crates.io token) — are handed off as exact commands, not run here.

## Work items (each = one themed commit)

### 1. Benches + highlight-perf spike
Do this first: the highlight bench result decides item 2's architecture.

- `benches/hot_paths.rs` (criterion, `harness = false`; add criterion dev-dep).
- Bench `diff::compute_ghosts` on synthetic strings (500 / 5k / 20k lines).
- Bench `snapshot_at` + `SnapshotCache` hit/miss against a temp-repo fixture
  built in setup (same pattern the tests use).
- Bench `syntax`-highlighting a file at 500 / 5k / 20k lines.
- **Decision gate:** if whole-file highlight fits comfortably in budget at 20k
  lines, highlight on the interactive path (simpler). If not, highlight on the
  prefetch thread and ship already-styled snapshots. Record the outcome in
  decisions.md.

### 2. Syntax highlighting + terminal-respecting theme
- **New module `src/syntax.rs`** — owns a `SyntaxSet`; produces per-line style
  runs (`Vec<Vec<(Range, Color)>>`), plain data, no git2, no widgets.
- Dependency: `syntect` with the **fancy-regex** feature (pure Rust — avoids
  stacking an oniguruma C build on top of vendored libgit2). Verify cold-open
  stays < 1s with syntax assets loaded (whitepaper perf target).
- **Purity rule:** highlighting never runs in `draw()`. It runs when a snapshot
  becomes current and is cached keyed by oid (parallel to the snapshot cache).
  Placement (main thread vs. prefetch thread) per item 1's gate.
- **Custom muted palette [locked]:** a small hand-built theme (or token-class →
  color map) using cold desaturated tones only — e.g. keyword = muted
  slate-cyan, string = muted tan, comment = dim steel, type/fn = cool grey-green.
  Never saturated red or blue. Sampled through the `theme` module so it degrades
  with `ColorDepth` like everything else.
- **Composition with ghosting:** on a changed line the ghost glow *overrides*
  syntax color (the comet trail must read as red/blue); unchanged lines get
  syntax color.
- **"Theme respects terminal colors":** keep inheriting the terminal background
  (never paint full-screen); default to the dark-optimized muted palette, with a
  light variant selectable via config (item 3).
- **Verify (TestBackend):** a keyword cell gets its syntax color; a ghosted line
  still glows red/blue over syntax.

### 3. Config file
- **New module `src/config.rs`** — `serde` + `toml`; locate `config.toml` via the
  `dirs` crate (`~/.config/tenetui/` and platform equivalents). Optional: no file
  → defaults; parse error → warn to stderr, use defaults, never crash.
- Fields: `speed_ms`, `cache_size`, `[keybinds]`, `palette` (dark/light).
- **The one real refactor:** turn `input.rs`'s hardcoded match into a
  data-driven `Keymap` (default table overlaid with config). The "keys matched
  in one place" convention holds — defaults are one table; config overlays it.
  The search-mode table stays separate.
- **Verify:** parse/merge/default unit tests + a rebind test (remap a key →
  correct `Action`).

### 4. Help overlay (`?`)
- `AppState.help_visible`; `?` toggles, `Esc` closes; modal (other keys ignored
  while open). New `ui/help.rs` — centered box listing the **live** keymap (so it
  reflects config overrides from item 3), grouped by function, thin-bordered over
  a dimmed pane (minimal-chrome principle). Depends on item 3's `Keymap`.
- **Verify (TestBackend):** overlay lists bindings; it's modal.

### 5. CI
- `.github/workflows/ci.yml` — on push/PR: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- **Matrix: Linux + Windows** (maintainer develops on Windows; it's a
  cross-platform TUI). Cache the cargo registry + target dir. libgit2's vendored
  build needs a C toolchain — present on both runners by default.

### 6. Release prep (hand-off, not published here)
- `README.md` — hero GIF first, one-line install, ~5 core keybinds, the Tenet
  identity blurb.
- `demo.tape` — a `vhs` (charmbracelet) script driving a scripted scrub →
  playback → blame toggle → search, rendered to the README GIF. **Maintainer
  runs** `vhs demo.tape` in a real terminal (needs vhs + ffmpeg).
- `LICENSE-MIT` + `LICENSE-APACHE` (Cargo.toml already declares the dual
  license).
- Bump `version` `0.0.0 → 0.1.0`; `cargo publish --dry-run`.
- **Maintainer runs** `cargo publish` (needs a crates.io token). Confirm the
  crate name `tenetui` is available before this step.

## Sequencing rationale
1 before 2 (bench decides architecture) · 3 before 4 (help renders the keymap) ·
5 any time after 1 · 6 last (packages the finished product).

## Risks / open items
- **Highlight perf on large files** — the item-1 bench is the mitigation; falls
  back to prefetch-thread highlighting if needed.
- **crates.io name availability** — confirm `tenetui` is free before item 6.
- **linux.git-scale validation** — not possible in the dev sandbox (no network
  for the clone); verified architecturally + on synthetic repos, same as M3.
  Worth a real large-repo run before publish.

## Not in scope (still deferred)
- Deletion `+/-` gutter sign (decisions.md, 2026-07-05) — no single-pane anchor
  for a removed line; revisit if the blame gutter's margin suggests a design.
- M5 stretch items (function-level tracking, volatile-files overview, rename
  following, dual-playhead pincer mode).
