# tenetui — Architecture

## Module layout

```
src/
  main.rs        — CLI parsing, terminal setup/teardown, top-level loop
  app.rs         — AppState (single source of truth) + update(state, event)
  input.rs       — key → Action mapping table (the ONLY place keys are matched)
  ui/
    mod.rs       — draw(frame, &AppState); pure, no mutation
    timeline.rs  — heatmap timeline widget
    filepane.rs  — file view with ghost highlighting + blame gutter
    statusbar.rs
  repo/
    mod.rs       — public structs: CommitMeta, Snapshot, BlameInfo
    walk.rs      — history walk → Vec<CommitMeta>
    snapshot.rs  — tree lookup + LRU cache
    prefetch.rs  — background thread, channel-driven
    blame.rs     — async blame on scrub pause
  diff.rs        — line diff + ghost decay bookkeeping
```

## Data flow

```
crossterm event ─→ input.rs (Action) ─→ app::update ─→ AppState mutated
                                             │
                              (playhead moved? send PrefetchHint)
                                             ▼
                                   repo::prefetch thread
                                             │
                              (SnapshotReady msg via channel)
                                             ▼
                        app::update merges into cache ─→ ui::draw next frame
```

- One event loop, one tick source (for playback animation). `update` handles both key events and channel messages; `draw` is called once per loop iteration.
- `AppState` owns: timeline vec, playhead index, snapshot cache handle, ghost decay map, playback state, viewport scroll, blame result (Option).

## Threading

- Main thread: event loop + rendering.
- Prefetch thread: owns its *own* `git2::Repository` handle (Repository is not Sync — never share it). Receives playhead hints, materializes snapshots, sends them back.
- Blame thread (M3): spawned per-request with a generation counter; stale results (generation mismatch) are dropped on receipt.

## Performance invariants

- `draw()` does zero I/O and zero diffing — everything it needs is precomputed in AppState.
- Cache key: (commit oid, file path). Values are `Arc<str>` so snapshots are shared, not cloned.
- Diffs are computed on transition (in update, cached path) or by prefetch thread, never per frame.
- Ghost decay is O(changed lines), stored as `HashMap<LineNo, u8>` decremented per scrub step.

## Error philosophy

- `anyhow` everywhere; top-level loop catches, restores terminal, prints error.
- A missing snapshot (file didn't exist at that commit) is a *state*, not an error — render an "file not yet created" placeholder.
