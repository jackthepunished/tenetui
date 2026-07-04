//! Line diff + ghost decay bookkeeping. On every playhead transition, the caller
//! diffs the old and new snapshot content and calls [`compute_ghosts`] to update
//! the decay map; the renderer (`ui::filepane`) turns decay into color via
//! `theme::ghost`. Kept as plain data here — no rendering, no git2, matching the
//! module split in docs/architecture.md.

use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;

/// Scrub steps a changed line stays lit before fading fully to base — "changed
/// lines glow, then fade over the next few scrub steps" (whitepaper).
pub const GHOST_MAX_DECAY: u8 = 5;

/// Age `existing` by one step (dropping anything that's fully decayed), then
/// diff `old` → `new` and relight every line that changed, keyed by its line
/// number *in `new`* (0-indexed) — that's the only content the file pane can
/// still point at. A pure deletion has no such line to anchor a glow to, so it
/// doesn't get one; the surrounding lines' churn is what remains visible.
pub fn compute_ghosts(old: &str, new: &str, existing: &HashMap<usize, u8>) -> HashMap<usize, u8> {
    let mut next: HashMap<usize, u8> = existing
        .iter()
        .filter_map(|(&line, &decay)| {
            let decayed = decay.saturating_sub(1);
            (decayed > 0).then_some((line, decayed))
        })
        .collect();

    let mut new_line = 0usize;
    for change in TextDiff::from_lines(old, new).iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => new_line += 1,
            ChangeTag::Insert => {
                next.insert(new_line, GHOST_MAX_DECAY);
                new_line += 1;
            }
            ChangeTag::Delete => {}
        }
    }
    next
}

/// The lowest line number among lines that just changed *this* transition
/// (full decay) — the auto-scroll follow target. `None` if this transition
/// touched no lines the pane can still see (e.g. a pure deletion, or no change).
pub fn freshest_changed_line(ghosts: &HashMap<usize, u8>) -> Option<usize> {
    ghosts
        .iter()
        .filter(|&(_, &decay)| decay == GHOST_MAX_DECAY)
        .map(|(&line, _)| line)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_line_glows_at_max_decay() {
        let ghosts = compute_ghosts("a\nb\n", "a\nX\nb\n", &HashMap::new());
        assert_eq!(ghosts.get(&1), Some(&GHOST_MAX_DECAY));
        assert_eq!(ghosts.len(), 1);
    }

    #[test]
    fn modified_line_glows_at_its_new_position() {
        // similar's line diff represents an edit as delete-old + insert-new;
        // the new content's line 1 ("B") should relight even though it's a
        // "replace" from the user's point of view, not a pure insert.
        let ghosts = compute_ghosts("a\nb\nc\n", "a\nB\nc\n", &HashMap::new());
        assert_eq!(ghosts.get(&1), Some(&GHOST_MAX_DECAY));
        assert_eq!(ghosts.len(), 1);
    }

    #[test]
    fn unchanged_transition_only_ages_existing_glow() {
        let existing = HashMap::from([(3usize, 3u8)]);
        let ghosts = compute_ghosts("same\n", "same\n", &existing);
        assert_eq!(ghosts.get(&3), Some(&2));
    }

    #[test]
    fn glow_expires_after_max_decay_steps() {
        let mut ghosts = HashMap::from([(0usize, 1u8)]);
        // One more no-op transition should decrement 1 -> 0 -> dropped.
        ghosts = compute_ghosts("x\n", "x\n", &ghosts);
        assert!(ghosts.is_empty(), "{ghosts:?}");
    }

    #[test]
    fn pure_deletion_relights_nothing() {
        // "b" is removed outright; there's no line in `new` to glow.
        let ghosts = compute_ghosts("a\nb\nc\n", "a\nc\n", &HashMap::new());
        assert!(ghosts.is_empty(), "{ghosts:?}");
    }

    #[test]
    fn freshest_changed_line_picks_the_lowest_max_decay_line() {
        let ghosts = HashMap::from([
            (5usize, GHOST_MAX_DECAY),
            (2usize, GHOST_MAX_DECAY),
            (7usize, 2u8),
        ]);
        assert_eq!(freshest_changed_line(&ghosts), Some(2));
    }

    #[test]
    fn freshest_changed_line_is_none_without_a_fresh_glow() {
        assert_eq!(freshest_changed_line(&HashMap::new()), None);
        assert_eq!(freshest_changed_line(&HashMap::from([(3usize, 2u8)])), None);
    }
}
