# Demo recording for the README GIF.
# Produce it with: cargo build --release && vhs demo.tape
#
# Drives tenetui against its own history — scrubbing tenetui's own theme.rs.

Output assets/demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1200
Set Height 700
Set Padding 12
Set TypingSpeed 60ms

# Launch, hidden, and let the timeline + first frame settle.
Hide
Type "target/release/tenetui . src/theme.rs"
Enter
Sleep 1500ms
Show
Sleep 1s

# Jump to the first commit, then play forward — watch the file evolve, changed
# lines glowing red and fading as the playhead sweeps toward HEAD.
Type "g"
Sleep 1200ms
Space
Sleep 4500ms
Space
Sleep 800ms

# Scrub back a few commits by hand: the glow turns blue (inverted).
Left@350ms 4
Sleep 1200ms

# Blame gutter: author + age per line, at this point in history.
Type "B"
Sleep 2500ms
Type "B"
Sleep 500ms

# Fuzzy-search commit messages and jump to a match.
Type "/"
Sleep 400ms
Type "color"
Sleep 600ms
Enter
Sleep 1800ms

# The help overlay, then quit.
Type "?"
Sleep 2500ms
Escape
Sleep 400ms
Type "q"
Sleep 500ms
