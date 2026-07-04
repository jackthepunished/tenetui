# tenetui

Terminal UI for scrubbing through a file's git history like a video timeline.
Rust + ratatui + crossterm + git2. Read-only tool — never mutate the target repo.

## Reference docs (read on demand, don't assume)

- @docs/whitepaper.md — product concept, interaction model, non-goals
- @docs/architecture.md — module layout, data flow, threading model
- @docs/roadmap.md — milestones with checkboxes; check items off as completed
- @docs/decisions.md — ADR log; append a dated entry for any non-trivial technical decision

## Commands

- Build: `cargo build`
- Run against a repo: `cargo run -- <path-to-repo> <file>`
- Test: `cargo test`
- Lint (must pass before considering work done): `cargo clippy -- -D warnings`
- Format: `cargo fmt`

## Architecture rules

- Rendering is pure: `fn draw(frame, &AppState)` reads state, never mutates it. All mutation happens in the event/update loop.
- git2 access lives only in `src/repo/`. UI code never touches git2 types directly — it consumes the snapshot/timeline structs from `repo::`.
- The snapshot cache and prefetcher run on a background thread; communicate via channels, never share `Repository` across threads (git2 `Repository` is not `Sync`).
- No `unwrap()` outside tests. Errors bubble via `anyhow::Result` to the top-level loop.
- Frame budget is 16 ms: no blocking I/O, no blame computation, no diffing inside `draw()`.

## Conventions

- Conventional commits (`feat:`, `fix:`, `perf:`, `docs:`).
- Every perf-sensitive change gets a criterion benchmark or a note in decisions.md explaining why not.
- Keyboard bindings are defined in one table in `src/input.rs` — never scatter key matching across widgets.

## Workflow

- Before implementing a roadmap milestone, restate the acceptance criteria from roadmap.md and confirm the plan.
- After completing a milestone item, tick its checkbox in roadmap.md in the same commit.
- If a decision contradicts the whitepaper, stop and ask — don't silently change scope.
