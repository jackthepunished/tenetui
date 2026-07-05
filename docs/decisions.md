# tenetui — Decision Log (ADRs)

Append-only. One dated entry per non-trivial technical decision. Format: context → decision → consequences. Claude Code: add an entry whenever you choose between meaningful alternatives.

---

## 2026-07-05 — git2 over shelling out to git

**Context:** Need per-commit file snapshots and history walks at scrub speed.
**Decision:** Use `git2` (libgit2 bindings) rather than spawning `git` subprocesses.
**Consequences:** ~100x faster per-lookup, no process spawn in the hot path; cost is dealing with `Repository: !Sync` (solved by one Repository per thread) and slightly more complex tree traversal code.

## 2026-07-05 — Immediate-mode state model

**Context:** Ghosting and playback mean the UI is highly dynamic.
**Decision:** Single `AppState` struct, pure `draw()`, all mutation in `update()`. No retained widget state beyond ratatui's built-in scroll states.
**Consequences:** Trivially testable update logic; rendering bugs are reproducible from a state snapshot. Requires discipline: no sneaky computation in draw.

## 2026-07-05 — Lazy snapshots + prefetch over eager materialization

**Context:** Materializing every historical version up front is O(history × file size) memory and slow on large repos.
**Decision:** LRU cache with background prefetch ±20 commits around the playhead.
**Consequences:** Constant memory, instant scrubbing near the playhead; long-distance jumps (g/G, search) may show a one-frame loading state.

## 2026-07-05 — Visual identity: the *Tenet* color code (red=forward, blue=inverted)

**Context:** Adoption depends on a demo GIF that reads as "that's a terminal?"; a generic pretty gradient is forgettable. The tool is named after *Tenet*, whose central visual device is color-coding time direction — red for forward entropy, blue for inverted. tenetui's core interaction *is* moving forward and inverted through history, so the film's code is semantically native, not a costume.
**Decision:** Adopt red=forward / blue=inverted as the app's only two saturated colors over a cold steel-grey base. Three consequences for the build: (1) the timeline is a **diverging map centered on the playhead** — red toward the future, blue toward the past, white pivot at the playhead ("full pincer timeline," chosen over intensity-only and accent-only variants); (2) **ghost trails take their hue from scrub direction** (red forward / blue inverted), with add-vs-delete carried by luminance + gutter sign, never hue; (3) color is always sampled from a named ramp at normalized `t`, Oklab-interpolated, computed in one `theme` module — never a literal per callsite. Red/blue is chosen partly *because* it survives 16-color fallback and is colorblind-safe (unlike red/green diffs).
**Consequences:** Every widget depends on one `theme` module, so a retheme is one edit. Requires a small Oklab→sRGB helper (no dependency) and `COLORTERM` capability detection at startup. The M5 temporal-pincer mode becomes the visual thesis (forward-red pane | inverted-blue pane) rather than an afterthought, and the timeline widget (M1) must be designed as a playhead-centered diverging map from the start. No frame-budget impact — ramps are pure functions over precomputed state. Supersedes the earlier "nebula indigo→amber" ramp sketch.

## 2026-07-05 — Docs live under docs/

**Context:** CLAUDE.md references `docs/whitepaper.md` etc., but the four markdown files were sitting in the repo root.
**Decision:** Move whitepaper/architecture/roadmap/decisions into `docs/` to match the documented layout rather than rewriting CLAUDE.md's paths.
**Consequences:** CLAUDE.md's `@docs/*` references resolve correctly; repo root stays clean for the forthcoming Cargo project.

## 2026-07-05 — Playback tick reuses the input poll timeout

**Context:** Playback needs a fixed-cadence tick to auto-advance the playhead. A naive design spawns a dedicated ticker thread/timer.
**Decision:** No ticker thread. `crossterm::event::poll`'s timeout doubles as the tick: while playing, the loop polls with `state.speed_ms`; a timeout with no key event *is* the tick, advancing one commit via the same `Engine::scrub` path manual `h`/`l` uses. Exactly the "one event loop, one tick source" already specified in docs/architecture.md.
**Consequences:** No extra thread, no extra channel, no risk of a ticker and a keypress racing on `AppState`. Speed changes (`+`/`-`) take effect on the very next poll since the timeout is recomputed every loop iteration. Trade-off: tick precision is bounded by key-event latency, fine at the ~30ms-2000ms range this app needs, not suitable for sub-frame timing.

## 2026-07-05 — Prefetch thread always refetches its window; no cross-thread cache

**Context:** The background warmer (`repo::prefetch::run`) and the main thread's `SnapshotCache` are two different pieces of state on two different threads, by design (never share a `git2::Repository`, and channels are one-way data, not shared mutable state).
**Decision:** On every hint, the prefetch thread recomputes and refetches its entire ±20 window unconditionally, rather than tracking which oids it already warmed. It also coalesces the hint channel to only the latest position, so rapid scrubbing doesn't queue up stale windows.
**Consequences:** Simpler thread (no local cache to invalidate or bound), and the redundant git2 lookups happen off the render thread so they never touch the 16ms frame budget. Cost is up to ~41 tree lookups per hint even when most were already warmed last time; acceptable at M2's scale (a single file's history), revisit if profiling on a very large repo shows it matters.

## 2026-07-05 — Auto-scroll: eased top-anchored follow, not viewport-centered

**Context:** M2's roadmap bullet asks for the viewport to follow the most-recently-changed region. True centering needs the render viewport's height, but `draw()` must stay a pure read of `AppState` — plumbing terminal size into it (or having it write back) would break that invariant.
**Decision:** `app::set_playhead` aims `scroll_target` at the lowest freshly changed line minus a fixed `FOLLOW_MARGIN` (3 lines of context), computed from the ghost map alone — no viewport height needed. `app::ease_scroll`, called once per frame regardless of input, nudges `scroll` a fraction of the remaining distance toward `scroll_target` each tick, which is the whitepaper's "eases toward... rather than snapping" in practice.
**Consequences:** Changed lines are guaranteed visible near the top of the pane on any terminal size, motion is smooth (no snapping), and the mechanism stays inside `app.rs` with zero new coupling between rendering and control state. What it isn't: perfectly centered in the viewport — that's a fast-follow if we later thread `terminal.size()` through to the scrub path, not a blocker for M2's accept criterion.

## 2026-07-05 — Deletion +/- gutter sign deferred

**Context:** The whitepaper's "Directional ghosting" section says addition vs. deletion should be carried by luminance + a gutter sign, not hue. But the file pane only ever renders the file *as it exists at the playhead* — a deleted line has no position left to anchor a `-` marker to once it's gone from `new`.
**Decision:** Ship glow + decay (hue = direction, luminance = decay) for M2 without the +/- gutter sign; `diff::compute_ghosts` only marks lines present in the new content. Not a scope cut, silently — flagging it here because there's no obvious single-pane rendering target for a pure deletion yet.
**Consequences:** Additions and edits glow correctly today. A deletion's *absence* is currently invisible at the deleted position (the surrounding lines simply shift). Revisit alongside M3's blame gutter, which already reserves left-margin space — a transient "− N lines removed here" marker is the likely design, not attempted now.

## 2026-07-05 — Blame gutter toggle moved from `b` to `B`

**Context:** The roadmap's own M3 bullets independently proposed `b` for two different things: "Blame gutter (toggle `b`)" and "Jump motions: `w`/`b` by day" (the latter matching the whitepaper's original `w`/`b` day-jump pairing). Both can't hold the same key.
**Decision:** Move the blame toggle to `B` (shift), keeping lowercase `b` for jump-backward-a-day so the whitepaper's `w`/`b` motion pairing stays intact. Chosen over the reverse (remapping the jump) because `w`/`b` is an established vim-ism worth preserving verbatim, while blame's key was never load-bearing elsewhere.
**Consequences:** No functional loss either way; documented in `input.rs`'s module comment and a dedicated test (`blame_toggle_uses_shift_b_not_lowercase_b`) so the collision can't silently reappear if either binding is touched later.

## 2026-07-05 — Blame reuses the prefetch thread's coalescing pattern instead of a debounce timer

**Context:** Architecture calls for blame to be "computed once at the playhead when scrubbing pauses" — a full `git2` blame is too slow to run on every scrub step. The obvious design spawns a request on every move and debounces with an idle timer before actually blaming.
**Decision:** No debounce timer. The blame worker thread (`repo::blame::run`) blocks for a request, then drains any further queued requests before acting — identical to the prefetch thread's "coalesce to the latest hint" loop from M2. Combined with a generation counter (bumped per request, stamped on results, mismatches dropped on receipt), this means whichever position the user is actually resting on when the worker gets free time is what gets blamed and displayed; positions scrubbed *through* during continuous movement are naturally superseded before their blame ever finishes.
**Consequences:** One fewer piece of state (no `Instant`/timer bookkeeping in the main loop), and it's the same mental model as prefetch — a second instance of one pattern rather than a new one. `AppState.blame` is set to `None` on every move (invalidated immediately, not left stale) so a fast scrubber never sees blame attributed to the wrong commit, at the cost of a briefly blank gutter while the async result is in flight.

## 2026-07-05 — Fuzzy search is subsequence matching, not scored fuzzy-finding

**Context:** Roadmap M3 asks for "`/` fuzzy-search commit messages." Tools like fzf implement fuzzy matching with match-quality scoring (contiguous runs, word-boundary bonuses, etc.) — a meaningfully bigger feature than commit-message search needs.
**Decision:** `app::search_target` implements the simplest legitimate definition of "fuzzy": every character of the query appears in order (not necessarily contiguous) in the commit summary, case-insensitive. No scoring; the first match found searching forward from the playhead (wrapping) wins.
**Consequences:** Good enough for commit-message-length text and trivially testable (`search_target_finds_nearest_fuzzy_match_forward_and_wraps`). A pathological query could match a summary "by accident" (e.g. `"ab"` matching `"a...b"` far apart) with no relevance ranking to fall back on — acceptable at commit-message scale; revisit only if it proves annoying in practice.

## 2026-07-05 — Verification scope: no linux.git in the sandbox

**Context:** M3's accept criterion is "blame never blocks scrubbing; navigation works on linux.git without stalls." linux.git is a multi-GB clone requiring network access this environment doesn't have.
**Decision:** Verify the architecture-level guarantee instead of the literal repo: blame runs on its own thread behind a channel (the render loop only ever does a non-blocking `try_recv`), so a slow blame computation structurally *cannot* block a frame regardless of repo size. Correctness (author/age attribution, tag/merge detection, jump math, fuzzy search) is covered by unit tests against small synthetic repos built with real `git2` commits, and the full binary was smoke-tested headlessly against a small real repo with a tag and multiple authors.
**Consequences:** The scaling claim ("works on linux.git without stalls") is architecturally sound but not empirically measured in this environment. Flagging this rather than claiming a validation that didn't happen; worth an actual large-repo run before the M4 release milestone.

## 2026-07-05 — Lib + bin split

**Context:** Criterion benches and integration tests can only reach a crate's *public API*. A bin-only crate exposes nothing, so `diff::compute_ghosts`, `SnapshotCache`, `syntax::highlight` etc. couldn't be benched.
**Decision:** Split into a library crate (`src/lib.rs`, all modules `pub`) plus a thin binary (`src/main.rs`) that drives the event loop over it. The binary's `Engine`/`run` stay in `main.rs`; everything else is library.
**Consequences:** Benches/tests use `tenetui::…`. Public-API items no longer trip `dead_code`, so a few `#[allow(dead_code)]` markers became unnecessary. Minor: internal helpers are now technically public, acceptable for a single-binary tool.

## 2026-07-05 — Syntax highlighting runs async (bench-driven), custom muted palette

**Context:** M4 wants syntect highlighting, but `draw()` is pure and the frame budget is 16ms. The `hot_paths` bench settled the architecture: `syntect` (fancy-regex, pure Rust) highlights ~500 lines in **20ms**, 5k in **206ms**, 20k in **924ms** — far too slow to run synchronously on the scrub path, even for small files. (diff and snapshot lookups, by contrast, are 66µs–3ms and stay inline.)
**Decision:** Highlight on a dedicated background thread with the same coalesce-to-latest + generation-guard pattern as blame/prefetch. On a move the pane shows plain text instantly (`highlighted = None`); the worker's result colorizes it a beat later, and stale results (superseded by a newer move) are dropped by generation mismatch. Colors come from a **custom muted palette** (`theme::syntax_rgb`): cold, low-chroma tones that never use saturated red or blue, so those stay reserved for time-direction and ghost trails (the *Tenet* identity). Ghost glow overrides syntax on changed lines.
**Consequences:** The render loop is non-blocking regardless of file size — the real invariant, better than "fast enough for small files." Trade-off: during fast scrub/playback, intermediate commits may flash plain before coloring (imperceptible for small files, graceful for huge ones). fancy-regex was kept over oniguruma to avoid a second C dependency on top of libgit2; onig would be ~2–4× faster but still can't be synchronous, so it wouldn't change the async decision. No per-oid highlight cache yet — re-highlighting on revisit is free on the main thread (it's the worker's problem); add an LRU if the worker thread proves a CPU hog on huge files.

## 2026-07-05 — Keys become data: a `Keymap` overlaid with config, not a hardcoded match

**Context:** M4 adds user-rebindable keys (`[keybinds]` in the config file). The existing `input::map_key` was a hardcoded `match`, which can't be overridden at runtime.
**Decision:** Replace the match with a `Keymap` (`HashMap<KeyChord, Action>`). `Keymap::default()` holds the built-in bindings in one table; `apply_overlay` layers the config's `key-string → action-name` pairs on top, warning-and-skipping any entry whose key or action doesn't parse. `KeyChord` normalizes a `KeyEvent` to `(ctrl, code)` — only Ctrl is tracked; Shift rides on the char's case and other modifiers are ignored, exactly matching the old match's behavior. Actions carry `name()`/`from_name()`/`describe()` for config round-tripping and the help overlay.
**Consequences:** Still honors the "keys in one place" convention (defaults are one table; config overlays it, the search-mode table stays separate). The help overlay renders `keymap.keys_for(action)` so it always shows the *live* bindings, config rebinds included. A bad config binding is isolated (skipped with a warning) rather than fatal. Config scope is exactly the roadmap's list — keybinds, speed, cache size — with `deny_unknown_fields` so a typo'd key is a visible parse error, not a silent no-op.

## 2026-07-05 — Rename following: per-commit path on `CommitMeta`, detection only where the file appears

**Context:** M5 `rotas`. The walk compared the blob at a *fixed* path against the parent, so history stopped dead at a `git mv`.
**Decision:** The walk tracks the file's path as it goes back: `CommitMeta` gains a `path` field (the name *at that commit*), and when the file is present in a commit but absent from its parent, full-tree rename detection (`diff.find_similar`) checks whether it arrived via a move — if so, the walk continues under the former name. Rename detection is gated on that "file appears going backward" condition, so the common (no-rename) case never pays for it. Churn across a rename is a direct blob-to-blob diff (`Patch::from_blobs`) so a pure move reads as zero churn, not a full re-add. Everything that materializes content keys off the per-commit path: `snapshot`/blame requests carry it, and prefetch now takes `(Oid, path)` pairs. The snapshot cache key stays `oid` alone — one lineage per session, so oid still uniquely identifies content.
**Consequences:** A moved file's history extends across the move; blame, highlighting, and playback all work on both sides. The header shows the path *at the playhead*, so the file's former name resurfaces as you scrub back past the move — a quietly nice moment. First-parent only, so a rename that happened on a merged-in branch isn't followed (consistent with the rest of the walk); cross-file *function* tracking remains out of scope (that's `arepo`). Rename detection uses libgit2's default 50% similarity threshold.

## 2026-07-05 — Temporal pincer: a `Deck` abstraction, two of them

**Context:** M5 `sator` — the app's visual thesis: two playheads side by side, one running forward (red), one inverted (blue), advancing together. Per-playhead state (`playhead`, `current`, `ghosts`, `direction`, `scroll`, `scroll_target`, `highlighted`) was scattered across flat `AppState` fields, which can't represent two independent positions.
**Decision:** Extract that per-playhead state into a `Deck`. `AppState` now holds `decks: Vec<Deck>` (one normally, two in pincer mode), a `focus` index, and a `pincer` flag; the playback/blame/mode fields stay app-level. `set_playhead`/`set_highlighted`/`request_highlight`/`jump_to` all take a deck index; `ease_scroll` animates every deck. Entering pincer clones the focused deck and pins roles (deck 0 = forward, deck 1 = inverted); `t` (turnstile) toggles it, `Tab` switches focus, and playback's `space` runs both decks at once (deck 0 +1, deck 1 −1) until both hit their ends. `ui::filepane::render` takes a `&Deck` (+ theme + optional blame) so the same widget renders either pane into either half of a split; the timeline grows a second pivot and neutral-steel "between the jaws" span. Highlighting gained a per-deck generation + a `deck` tag on requests/results, and the worker coalesces *per deck* so the two panes never starve each other.
**Consequences:** The pincer is the marquee feature and the README's second image, built with zero new dependencies. The Deck refactor rippled through most modules but left the async-worker patterns intact (prefetch still hints only the focused/primary position — the other deck's jumps are cache-miss git2 lookups, fine at ~72µs; blame follows focus only, as planned). Between-jaws timeline cells read as steel, which leans slightly cold/blue by palette design — acceptable, since red vs. blue dominance still cleanly marks the two directions. Single-deck mode is exactly `decks[0]` with `pincer = false`, so the common path is unchanged in behavior.

## 2026-07-05 — Volatile-files overview as a second screen, not a mode of `AppState`

**Context:** M5 `opera` — an entry screen ranking the repo's hottest files, shown when no file arg is given; `Enter` opens the player on the selection. The player's `AppState` is entirely file-scoped; bolting a repo-wide list onto it would muddy that.
**Decision:** Two screens with separate state, coordinated by a top-level loop in `main` (`run_app` → `run_overview` / `run_player`), not a variant inside `AppState`. `repo::volatility` scans a bounded window (last 500 commits) accumulating per-file touch count + line churn + last-touch time via per-delta patches, ranked churn-first. The overview (`ui::overview`) has its own tiny `OverviewState` and its own key handling (`j`/`k`/`Enter`/`q`) — not the player keymap, since it's a different interaction. Each screen opens its own short-lived `Repository`. Quitting a player launched from the overview returns to the overview (pick another file); quitting the overview exits. Churn bars are steel-only — red/blue stays reserved for time-direction inside the player.
**Consequences:** `AppState` stays file-scoped and the player loop is untouched (extracted verbatim into `run_player`). The scan cost is per-delta patch generation bounded by the 500-commit window; if it's ever slow on a huge repo the documented fallback is ranking by touch count alone (no patches). A file given directly still goes straight to the player, so nothing changes for the common invocation. Age uses `SystemTime::now()` captured once into `OverviewState` (not called per-frame), keeping `render` a pure read.

## 2026-07-05 — Function tracking: feature-gated tree-sitter; scoped pane now, timeline-collapse deferred

**Context:** M5 `arepo` — `git log -L`'s spiritual equivalent, scoping the view to one function. tree-sitter + a grammar are a heavy dependency (compile time + binary size), and the full vision (timeline collapses to only the commits touching the function) needs parsing *every* snapshot.
**Decision:** Gate it behind a `functions` cargo feature (`--features functions`; Rust grammar only for now) so the default `cargo install tenetui` stays light — when off, `functions::functions_in` compiles to a stub returning nothing and `F` is inert. `F` parses the current snapshot (a fast tree-sitter walk for `function_item` nodes → name + line range) and opens a picker; selecting one enters scoped mode, where the file pane clamps to that function's range, re-resolved *by name* in each snapshot as you scrub (a placeholder shows where it isn't present), `Esc` unscopes. Scoping applies per deck, so it composes with the pincer. **Deferred:** collapsing the *timeline* to the function's commits — that requires a background parse-every-snapshot worker (a fourth coalesce+generation job); shipping the picker + scoped pane first delivers the "follow one function as you scrub" experience, which is the bulk of the value.
**Consequences:** The base binary is unchanged in size/deps; the feature is opt-in and CI covers both configurations. Parsing runs on the interactive `jump_to`/scope-entry path, not `draw()` — a single-file tree-sitter parse is a few ms, within the update budget like the diff. Known v1 limits (from the plan, accepted): duplicate function names resolve to the first match; a function that moves *between files* isn't followed; only Rust is supported. The timeline still shows the whole file's history while scoped — a visible next step, not a silent gap.

## 2026-07-05 — The space-time map: mockup translated into the identity, not adopted verbatim

**Context:** M6 `entropy`. The maintainer supplied a mockup of a sci-fi "space-time grid" console — six saturated hues, dense dashboard chrome, fictional gauges (temporal integrity, paradox risk). Adopting it verbatim would contradict the visual-identity ADR (red/blue as the only saturated, semantic colors; restraint; real data) — flagged per the workflow rule, and the maintainer chose the translation: keep the mockup's geometry and drama, render it in the Tenet code, as one new screen rather than an app-wide reskin.
**Decision:** A braille-canvas map view (`m`), drawn with ratatui `Canvas` + `Marker::Braille` (2×4 sub-cell dots): a ring-and-spoke grid warped toward the center (rings at `r^1.35`), commits plotted by direction (past left/blue, future right/red — `theme::timeline_cell` intensity from churn), deterministic hash-scatter for the constellation fan (no RNG, stable across frames), dashed bezier arcs radiating from the nearest commits to a white-hot pivot labeled with the mockup's own caption — "you are here… all paths radiate from this moment" — which was already this app's thesis. Node cap ~120 + landmarks; label placement is side-aware with greedy collision skipping. The right panel is real commit data only; the pincer timeline strip and status bar stay visible, so `h`/`l`/jumps/playback keep working and re-anchor the whole web live.
**Consequences:** Zero new dependencies, zero engine changes — the map is a pure render mode over existing `AppState` (all painting is math over precomputed state; no I/O in `draw()`). Known limits, accepted: braille color is per *cell*, so crossing paths share a color where they cross (layout spreads nodes to keep it rare); no glow/bloom in terminals (intensity ramps instead); in pincer mode the map shows the focused deck (dual-pivot map listed as follow-up). The fictional gauges were dropped, not restyled — a real stats strip is the honest successor if wanted.

## 2026-07-05 — Motion pack: decaying-animation state on the existing tick, no timers

**Context:** M7 `turnstile`. The whitepaper's "Motion & motifs" section promised a palindrome cold open and a turnstile flip that were never built; playback also read as static outside the pane itself. Terminals can't blur or tween, so all motion must be discrete frames of pure color math.
**Decision:** Animations are plain decaying state in `AppState`, advanced by `app::anim_tick` — intro frames (`u8`), per-deck turnstile frames, and a timeline `heat: HashMap<commit, u8>` seeded with the position the playhead just left; the map trail is a per-deck `VecDeque` of recent positions (position-indexed fade, no timer). The event loop's poll timeout becomes the third leg of the existing one-tick-source pattern: playback speed while playing, ~33ms while `state.animating()`, 250ms idle. Draw stays pure — each effect is a color ramp over this state (cold open = edge-converging timeline reveal + `theme::fade` dim→steel text ramp; turnstile = whole-pane hue wash at the new direction; heat = intensity floor near the pole color; trail = fading pivot-white dots re-projected against the current playhead). Any key skips the intro. Render precedence in the file pane: fade > turnstile > ghost > syntax > plain.
**Consequences:** Zero new threads, timers, or dependencies; animations cost one HashMap/VecDeque tick per frame and stop consuming CPU the moment they decay out (`animating()` returns false → back to the 250ms idle poll). Key-repeat can outpace the 33ms tick, which just holds effects at full heat until input pauses — acceptable, self-correcting. clippy's too-many-arguments on the file pane forced a `PaneView` flags struct — a better API than the parameter creep it replaced. Word-level ghosting and the dual-pivot pincer map remain the next motion steps (tracked in M7).

