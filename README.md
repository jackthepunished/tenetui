# tenetui

**Scrub through a file's git history like a video timeline — forward and inverted.**

tenetui turns a file's history into a scrubbing timeline you *move through*, the
way you'd scrub a video. Move the playhead and the main pane shows the file
exactly as it existed at that commit; press play and watch it grow, get
refactored, and shrink in seconds. Named after *Tenet*: history plays forward
and inverted, and what happened, happened.

![tenetui demo](assets/demo.gif)

> The GIF is produced from [`demo.tape`](demo.tape) with
> [vhs](https://github.com/charmbracelet/vhs): `cargo build --release && vhs demo.tape`.

## Why

`git log -p` and `git log -L` give you walls of diff with no spatial continuity.
`git blame` shows only the *last* touch per line, hiding the history underneath.
The mental model you actually want is temporal — history as a continuous medium
you move through, not a list of artifacts you inspect one by one. tenetui is a
*player*, where `tig`/`gitui`/`lazygit` are commit browsers.

## Install

```sh
cargo install tenetui
```

## Usage

```sh
tenetui <repo> <file>
# e.g. from inside a checkout:
tenetui . src/main.rs

# omit the file to open the volatile-files overview and pick one:
tenetui .
```

Scrub with `h`/`l`, press `space` to play, `?` for the full key list. That's it.

## Keys

| Key | Action |
| --- | --- |
| `h` / `l` (or `←`/`→`) | scrub one commit back (inverted) / forward |
| `space` | play / pause (continues in the last scrub direction) |
| `+` / `-` | faster / slower playback |
| `w` / `b` | jump forward / back a day |
| `{` / `}` | jump back / forward a week |
| `g` / `G` | first commit / last commit (HEAD) |
| `/` | fuzzy-search commit messages |
| `B` | toggle the blame gutter |
| `t` | temporal pincer: two playheads, forward + inverted, side by side |
| `Tab` | switch focus between the pincer panes |
| `j` / `k` | scroll the file |
| `?` | help overlay |
| `q` | quit |

In pincer mode, `space` runs both playheads at once — the forward pane steps
toward HEAD (red) while the inverted pane runs toward the root (blue), the two
jaws of the timeline closing as they go.

Changed lines glow — **red** when moving forward, **blue** when inverted — and
fade over the next few steps, leaving a comet trail during playback. The timeline
is a temporal pincer: red toward the future, blue toward the past, a white-hot
pivot at the playhead.

## Configuration

Optional, at `~/.config/tenetui/config.toml` (and platform equivalents):

```toml
speed_ms = 150       # initial playback cadence (ms per commit)
cache_size = 512     # snapshot LRU capacity

[keybinds]           # key = action-name, layered over the defaults
x = "quit"
"ctrl-r" = "scrub_forward"
```

A missing file or a bad value falls back to defaults with a warning — the config
never stops the app.

## The look

The aesthetic is drawn from the film. In *Tenet*, Christopher Nolan color-codes
the two directions of time: **red for forward entropy, blue for inverted.** That
maps exactly onto scrubbing toward HEAD (forward) versus toward the root
(inverted), so red and blue are the app's only two saturated colors, over a cold
steel base. True color by default, degrading gracefully to 256- and 16-color
terminals — where red and blue still exist, so the meaning survives.

## Build from source

```sh
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
cargo bench            # criterion benches for the hot paths
```

## License

Dual-licensed under either [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option.
