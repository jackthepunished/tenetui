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
