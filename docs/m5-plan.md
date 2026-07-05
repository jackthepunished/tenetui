# tenetui — M5 Implementation Plan (Stretch)

Status: planned, not started. M5 is post-v1 and the roadmap marks it
*unordered*; this plan proposes an order (adjustable) and expands each item
into a design sketch. Append decisions that change to decisions.md as usual.

Naming note: v1's commit history reads `big bang → inversion → afterimage →
foreknowledge → exposure → protocol → continuity → premiere`. M5's four items
map neatly onto the four non-TENET words of the SATOR square (the palindrome
the film's title comes from): **ROTAS** (wheels — a file rolling to a new
path), **SATOR** (the man who runs the temporal pincer), **OPERA** (works —
the whole repo's body of work), **AREPO** (the strange one). Suggested commit
names below follow that.

## Proposed order

1. **R — Rename following** (`rotas`) — smallest, pure correctness, touches the
   data model everything else reads; do it before features stack on top.
2. **P — Temporal pincer mode** (`sator`) — zero new dependencies, the visual
   thesis of the whole tool (see the visual-identity ADR), the marquee shot.
3. **V — Volatile-files overview** (`opera`) — new entry screen, self-contained.
4. **F — Function-level tracking** (`arepo`) — biggest dependency + design
   risk; last so its unknowns can't stall the other three.

Each item is independently shippable (0.2.0, 0.3.0, …).

---

## R · Rename following across file moves (`rotas`)

Today `repo::walk::timeline` compares the blob at a *fixed* path against the
first parent, so history stops dead at a `git mv`.

- **Detection:** while walking backward, when the tracked path exists in the
  commit but not in its parent, run `diff_tree_to_tree(parent, commit)` with
  `find_similar(renames = true)`; if a RENAMED delta lands on our path, switch
  to the delta's old path for all older commits.
- **Data model:** `CommitMeta` gains `path: String` (the file's path *as of
  that commit*). Everything that materializes content must use the per-commit
  path, not the session path: `snapshot_at` callers, the prefetch thread
  (`Vec<Oid>` → `Vec<(Oid, String)>`), and blame requests. The snapshot cache
  key stays `oid` — one lineage per session, so oid alone still uniquely
  identifies content.
- **UI:** the header shows the path at the playhead, which now changes as you
  scrub across a rename — a quietly great moment (the file's old name
  resurfaces as you invert past the move).
- **Tests:** fixture repo with an index rename (`git mv` equivalent via git2);
  assert the timeline crosses the move, snapshots resolve on both sides, and
  the pre-move commits carry the old path.
- **Accept:** a moved file's history extends past the move; blame and
  playback work on both sides of it.

## P · Temporal pincer mode (`sator`)

Two playheads, two panes: forward-red on the left, inverted-blue on the right.
Per the visual-identity ADR this is the app's thesis, not a gimmick.

- **The one real refactor:** extract per-playhead state out of `AppState` into
  a `Deck { playhead, current, ghosts, direction, scroll, scroll_target,
  highlighted }`. Single-deck mode is `decks[0]`; pincer mode activates
  `decks[1]`. `app::update`/`set_playhead`/`ease_scroll` take `&mut Deck`.
  `ui::filepane` renders a deck into a rect (it's already 90% parameterized).
- **Controls:** `t` (the turnstile) toggles pincer mode, seeding both decks at
  the current position. `Tab` switches focus; `h`/`l` scrub the focused deck
  (for lining up two eras). `space` runs the true pincer: each tick advances
  the forward deck +1 *and* the inverted deck −1 — converging or diverging
  through history simultaneously. Playback stops when both decks hit their
  ends.
- **Timeline:** render both pivots (red-hot and blue-hot). Hue field: cells
  left of the inverted pivot are blue, right of the forward pivot red, the
  span between them cools to steel — the pincer's jaws visibly closing during
  playback. (Exact ramp spec decided at the widget; keep the field anchored to
  the two pivots, never a third color.)
- **Supporting systems:** prefetch hints both playheads (send two hints; the
  worker already coalesces — extend to warm a window per hint). Blame follows
  the focused deck only. Ghosts are per-deck and already directional, so the
  left pane trails red and the right trails blue for free.
- **Accept:** side-by-side panes, one keypress runs both directions at once,
  and a screenshot of it is self-explanatory — this frame is the README's
  second image.

## V · Volatile-files overview (`opera`)

Entry point when no file is given: which files churn the most?

- **CLI:** the `file` arg becomes optional. `tenetui .` opens the overview.
- **Scan:** `repo::volatility(repo, max_commits = 500)` — walk from HEAD,
  tree-diff each commit against its first parent, accumulate per-path touch
  counts + insert/delete totals, return the top ~100 by churn with last-touch
  time. Bounded by commit count to respect the <1s cold-open target. Perf
  fallback flagged now: if line stats (patch generation) prove too slow on
  wide repos, rank by touch count alone — still a legitimate volatility
  signal, no patches needed.
- **UI:** new `ui/overview.rs` — ranked rows: churn bar in partial blocks
  (steel luminance only; red/blue stays reserved for time-direction), path,
  touches, last-touch age. `j`/`k` select, `Enter` opens the player on that
  file (reusing the exact startup path), `q` quits.
- **State:** a top-level `enum Screen { Overview(OverviewState), Player(...) }`
  in `main` rather than growing `AppState` — transitions rebuild the player
  via the existing init path, and `AppState` stays a player concern.
- **Accept:** `tenetui <repo>` with no file ranks the hot files and `Enter`
  drops into the timeline for the selection.

## F · Function-level tracking (`arepo`)

`git log -L`'s spiritual equivalent: scope the whole experience to one
function. The riskiest item — a heavy dependency and real design ambiguity —
which is why it goes last.

- **Dependency:** `tree-sitter` + grammars, **feature-gated** (e.g.
  `--features functions`, Rust grammar first) so the base
  `cargo install tenetui` stays light. Grammars dominate compile time and
  binary size; that cost must stay opt-in until proven worth defaulting.
- **UX:** `F` opens a picker (reusing the `/`-search interaction) listing the
  functions tree-sitter finds in the current snapshot; choosing one enters
  scoped mode. Esc exits scoped mode.
- **Scoped mode:** for each snapshot, parse + query for the function by name →
  line range. Timeline collapses to commits whose transition diff intersects
  that range (computed with the existing `similar` machinery). The file pane
  clamps to the range plus context; ghosts unchanged. A commit where the
  function doesn't exist yet renders the "not yet born" placeholder — the
  same state-not-error philosophy as missing files.
- **Perf:** parsing every snapshot on activation is a background job (fourth
  instance of the coalesce + generation worker pattern); timeline markers fill
  in progressively as parses complete.
- **Known ambiguities, accepted up front:** duplicate names (impl blocks,
  overload-ish cases) resolve to the first match in v1; cross-rename function
  tracking (function moved *between files*) is out of scope.
- **Accept:** pick a function in tenetui's own repo, timeline collapses to its
  commits, scrubbing stays scoped, all off the frame path.

---

## Risks / open items

- **Pincer timeline hue field** — the two-pivot coloring needs a real spec at
  implementation time; the constraint that holds regardless: red and blue
  anchor to their pivots, everything else is steel.
- **tree-sitter weight** — feature gate is the mitigation; if grammar bloat is
  still ugly, `arepo` ships as a separate opt-in build documented in the
  README rather than a default feature.
- **Volatility scan cost** — bounded walk + the touch-count fallback.
- **Carried over from M3/M4:** a real large-repo validation run (linux.git
  scale) still hasn't happened in any environment — worth doing once on the
  maintainer's machine before M5 features pile on; and the README GIF
  (`vhs demo.tape`) + `cargo publish` remain maintainer actions.

## Not in scope

- Multi-file / whole-repo playback (the overview is an *entry point*, not a
  repo-wide player).
- Editing anything, ever (whitepaper non-goal).
