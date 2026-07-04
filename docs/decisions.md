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
