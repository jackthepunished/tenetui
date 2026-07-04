# tenetui — Whitepaper

*A terminal UI for scrubbing through git history like a video timeline. Named after Tenet: history plays forward and inverted, and what happened, happened.*

## Problem

Understanding how code evolved is a common, painful task. Developers routinely ask "when did this function get complicated?", "who introduced this pattern?", "what did this file look like before the refactor?". The existing tools answer these questions poorly:

- `git log -p` and `git log -L` produce walls of diff text with no spatial continuity.
- `git blame` shows only the *last* touch per line, hiding the history underneath.
- GUI tools (GitKraken, IDE timelines) exist but break terminal workflows and are heavyweight.

The mental model developers actually want is *temporal*: history as a continuous medium you move through, not a list of discrete artifacts you inspect one by one.

## Concept

tenetui renders a file's history as a scrubbing timeline, borrowed from video editing:

1. **Timeline (bottom)** — every commit touching the current file, rendered as a density heatmap. Color intensity encodes churn (insertions + deletions). Tags and merges get markers.
2. **Playhead** — a cursor on the timeline. Move it with `h`/`l` (commit-by-commit), `w`/`b` (jump by day/week), or number keys (percentage jump). The main pane always shows the file *exactly as it existed* at the playhead.
3. **Main pane** — the file's content at the selected commit, syntax highlighted. Lines changed by the current commit glow, then fade over the next few scrub steps ("diff ghosting"), so motion through history produces a visible trail of what changed where.
4. **Playback** — press `space` and the playhead advances automatically, animating the file's evolution. Adjustable speed. This is the demo moment: watching a file grow, get refactored, and shrink in seconds.
5. **Blame gutter** — optional left gutter showing author + age per line, recomputed as you scrub, so ownership visibly shifts over time.

## Why a TUI

- The audience (developers investigating history) lives in the terminal; context-switching to a GUI is the exact friction we remove.
- Scrubbing/playback is an interaction people associate with GUIs. Delivering it smoothly in a terminal is the novelty hook — the "wait, you can do that?" reaction that drives adoption.
- ratatui's immediate-mode rendering suits this: every frame is a pure function of (playhead position, file snapshot, ghost state), which keeps the architecture simple.

## Technical approach

- **Language/stack**: Rust, ratatui + crossterm for rendering, `git2` (libgit2 bindings) for repository access, `syntect` or `tree-sitter` for highlighting.
- **History walk**: on startup, walk `rev-list` for the target file, collecting commit metadata (oid, time, author, churn stats). This list *is* the timeline.
- **Snapshot cache**: file content at each commit is materialized lazily and cached (LRU). Scrubbing hits the cache; a background thread prefetches ±N commits around the playhead so playback never stalls.
- **Diff ghosting**: for each transition, compute a line-level diff (`similar` crate) and store changed-line ranges with a decay counter. The renderer maps decay to color interpolation.
- **Blame**: incremental — full `git2` blame is too slow per-frame, so blame is computed once at the playhead when scrubbing pauses, and invalidated on move.

## Performance targets

- Scrub latency < 16 ms per step on a 10k-commit repo (cached path).
- Cold open to interactive < 1 s on repos the size of ratatui itself.
- Memory bounded by cache size, not repo size.

## Visual identity — *Tenet*

Beauty is a product requirement, not a polish-phase afterthought. The tool's adoption hinges on a demo GIF that reads as "wait, that's a terminal?" — so the aesthetic is specified, not left to chance. Every milestone demo should be screenshot-ready.

The identity is not a generic "pretty gradient." It is drawn directly from the film the tool is named after. In *Tenet*, Christopher Nolan color-codes the two directions of time: **red for forward entropy, blue for inverted.** The red team and the blue team; the red room and the blue room; the temporal pincer. That code maps exactly onto what tenetui does — scrubbing toward HEAD is *forward*, scrubbing toward the root is *inverted* — so red/blue is **semantic, not decorative**, and it makes the M5 temporal pincer the visual thesis of the whole tool rather than a stretch goal.

**Principles**

- **Red = forward, blue = inverted.** The only two saturated colors in the app; everything else is cold. Direction of travel through history owns this axis.
- **Cold industrial base.** Gunmetal / slate / steel-grey neutrals — the film's desaturated grade — so red and blue are the only warm/cool signals and therefore pop.
- **Respect the canvas.** Never paint a full-screen background; inherit the terminal background so tenetui feels native, not like an app squatting in a terminal. Chrome is minimal — thin or absent borders, breathing room, a quiet statusbar. Restraint over box-drawing soup; the film is sparse, not busy.
- **Computed color, graceful fallback.** 24-bit by default (detected via `COLORTERM=truecolor`), degrading to a 256-color ramp, then to 16-color — where red and blue still exist, so the semantic spine survives full degradation. Every color is a sample from a named ramp at normalized `t ∈ [0,1]`, never a literal per callsite. Interpolated in Oklab so midtones don't go muddy.
- **Sub-cell resolution.** The timeline uses partial-block glyphs (`▁▂▃▄▅▆▇█`) so one row encodes churn magnitude as height *and* direction as hue.

**The timeline is a temporal pincer**

The playhead is the white-hot "now" pivot. Commits toward the future (newer, right of the playhead) carry a **red** bias; commits toward the past (older, left) carry a **blue** bias; churn magnitude drives block height and luminance. Scrubbing sweeps the red/blue boundary along the bar — the signature motion, the pincer made visible at all times.

**Directional ghosting**

Changed lines glow on the transition frame and decay over ~5 scrub steps, leaving a "comet trail" during playback. The glow *hue is set by scrub direction*: forward (`l`, forward playback) glows **red**; inverted (`h`, reverse playback) glows **blue** — reverse playback looks like the film's inverted-motion shots, cold and running backward. Addition vs. deletion is carried by luminance + a gutter sign (`+`/`-`), *not* hue, so the red/blue axis stays reserved for direction.

**Motion & motifs**

- **Palindrome.** TENET is a palindrome (the SATOR square). The cold-open animation converges *from both ends toward the center* — a pincer — rather than wiping left-to-right. Wordmark/splash may mirror.
- **Turnstile.** Reversing scrub direction plays a brief "turnstile" flip transition — the machine that inverts you.
- **No judder.** Playback ticks at a fixed cadence with zero per-frame allocation; auto-scroll eases toward the most-recently-changed region rather than snapping.

All of the above is pure color math over precomputed state, so it costs nothing against the 16 ms frame budget — `draw()` stays I/O- and diff-free. Color and capability detection live in one `theme` module (single source of truth).

## Differentiation

Nothing in the terminal does temporal scrubbing. `tig`, `gitui`, and `lazygit` are commit *browsers* — discrete navigation, diff-oriented. tenetui is a *player*. The closest analogues are web toys (GitHub's blame view, "git history" VS Code extensions) which lack playback, ghosting, and terminal-native workflow.

## Non-goals (v1)

- Not a general git client: no staging, committing, branching, push/pull.
- No multi-file or whole-repo playback (future work).
- No editing; strictly read-only.

## Success criteria

- A 15-second demo GIF that makes a developer immediately understand the tool.
- Useful in anger: answers "when did X change and why" faster than `git log -L`.
- < 5 s from `cargo install` to first scrub in any repo.
