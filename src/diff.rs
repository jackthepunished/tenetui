//! Line diff + ghost decay bookkeeping. On every playhead transition, the caller
//! diffs the old and new snapshot content and calls [`compute_ghosts`] to update
//! the ghost state; the renderer (`ui::filepane`) turns decay into color via
//! `theme::ghost`. Kept as plain data here — no rendering, no git2, matching the
//! module split in docs/architecture.md.
//!
//! Ghosts are word-level: a changed line records not just its decay but *which
//! character ranges* changed, so the file pane can burn the changed words hot
//! and half-glow the rest of the line (much more legible during playback).

use similar::{ChangeTag, DiffTag, TextDiff};
use std::collections::HashMap;

/// Scrub steps a changed line stays lit before fading fully to base — "changed
/// lines glow, then fade over the next few scrub steps" (whitepaper).
pub const GHOST_MAX_DECAY: u8 = 5;

/// Ghost state for one snapshot: which lines are lit (and how strongly), plus
/// the changed word ranges within each. Keyed by 0-indexed line number *in the
/// new content* — the only lines the pane can still point at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ghosts {
    /// Line → remaining decay (`GHOST_MAX_DECAY` = fresh, 0 = gone).
    decay: HashMap<usize, u8>,
    /// Line → changed char ranges (start, end) within it. A lit line absent
    /// here (or with an empty vec) glows whole — a pure insertion with no old
    /// counterpart to word-diff against.
    words: HashMap<usize, Vec<(usize, usize)>>,
}

impl Ghosts {
    pub fn is_empty(&self) -> bool {
        self.decay.is_empty()
    }

    /// Remaining decay for a line, if lit.
    pub fn decay_of(&self, line: usize) -> Option<u8> {
        self.decay.get(&line).copied()
    }

    pub fn contains(&self, line: usize) -> bool {
        self.decay.contains_key(&line)
    }

    /// The changed char ranges within a lit line — empty means "whole line".
    pub fn hot_ranges(&self, line: usize) -> &[(usize, usize)] {
        self.words.get(&line).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The lowest line that just changed *this* transition (full decay) — the
    /// auto-scroll follow target. `None` if nothing the pane can see changed.
    pub fn freshest_changed_line(&self) -> Option<usize> {
        self.decay
            .iter()
            .filter(|&(_, &d)| d == GHOST_MAX_DECAY)
            .map(|(&line, _)| line)
            .min()
    }

    /// Test-only constructor from a bare decay map (no word ranges).
    #[cfg(test)]
    pub fn from_decay(decay: HashMap<usize, u8>) -> Ghosts {
        Ghosts {
            decay,
            words: HashMap::new(),
        }
    }
}

/// Age `existing` by one step (dropping fully-decayed lines and their word
/// ranges), then diff `old` → `new` and relight every changed line at
/// `GHOST_MAX_DECAY`, recording the changed word ranges. A pure deletion has no
/// line in `new` to anchor a glow to, so it doesn't get one.
pub fn compute_ghosts(old: &str, new: &str, existing: &Ghosts) -> Ghosts {
    let mut decay: HashMap<usize, u8> = existing
        .decay
        .iter()
        .filter_map(|(&line, &d)| {
            let d = d.saturating_sub(1);
            (d > 0).then_some((line, d))
        })
        .collect();
    // Carry word ranges only for lines that survived the aging.
    let mut words: HashMap<usize, Vec<(usize, usize)>> = existing
        .words
        .iter()
        .filter(|(line, _)| decay.contains_key(line))
        .map(|(&line, ranges)| (line, ranges.clone()))
        .collect();

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let diff = TextDiff::from_lines(old, new);

    for op in diff.ops() {
        match op.tag() {
            DiffTag::Insert => {
                // Whole new lines with no old counterpart → glow entire line.
                for j in op.new_range() {
                    decay.insert(j, GHOST_MAX_DECAY);
                    words.remove(&j);
                }
            }
            DiffTag::Replace => {
                let old_range = op.old_range();
                let new_range = op.new_range();
                for (k, j) in new_range.clone().enumerate() {
                    decay.insert(j, GHOST_MAX_DECAY);
                    // Pair with the old line at the same offset (if any) and
                    // word-diff to find the hot ranges; extra new lines beyond
                    // the old block are pure inserts (whole line).
                    match old_range.clone().nth(k) {
                        Some(oi) => {
                            let ranges = word_ranges(old_lines[oi], new_lines[j]);
                            if ranges.is_empty() {
                                words.remove(&j);
                            } else {
                                words.insert(j, ranges);
                            }
                        }
                        None => {
                            words.remove(&j);
                        }
                    }
                }
            }
            DiffTag::Delete | DiffTag::Equal => {}
        }
    }

    Ghosts { decay, words }
}

/// Char ranges in `new_line` that are inserted/changed relative to `old_line`,
/// via a word-level diff. Adjacent ranges are merged so runs stay contiguous.
fn word_ranges(old_line: &str, new_line: &str) -> Vec<(usize, usize)> {
    let diff = TextDiff::from_words(old_line, new_line);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut col = 0usize; // char position in new_line
    for change in diff.iter_all_changes() {
        let len = change.value().chars().count();
        match change.tag() {
            ChangeTag::Equal => col += len,
            ChangeTag::Insert => {
                match ranges.last_mut() {
                    Some(last) if last.1 == col => last.1 += len,
                    _ => ranges.push((col, col + len)),
                }
                col += len;
            }
            ChangeTag::Delete => {} // doesn't advance the new-line column
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(old: &str, new: &str) -> Ghosts {
        compute_ghosts(old, new, &Ghosts::default())
    }

    #[test]
    fn inserted_line_glows_at_max_decay() {
        let g = lit("a\nb\n", "a\nX\nb\n");
        assert_eq!(g.decay_of(1), Some(GHOST_MAX_DECAY));
        assert_eq!(g.decay.len(), 1);
        // A whole inserted line has no specific hot range.
        assert!(g.hot_ranges(1).is_empty());
    }

    #[test]
    fn modified_line_records_only_the_changed_word_range() {
        // "quick" → "slow" in the middle of the line; only that span is hot.
        let g = lit("the quick fox\n", "the slow fox\n");
        assert_eq!(g.decay_of(0), Some(GHOST_MAX_DECAY));
        let ranges = g.hot_ranges(0);
        assert!(!ranges.is_empty(), "expected a hot range, got none");
        // The hot range should cover "slow" (chars 4..8), not the whole line.
        let (a, b) = ranges[0];
        assert!(
            a >= 4 && b <= 9,
            "hot range {a}..{b} should sit on the changed word"
        );
        assert!(
            b - a < "the slow fox".len(),
            "should not glow the whole line"
        );
    }

    #[test]
    fn unchanged_transition_only_ages_existing_glow() {
        let existing = Ghosts::from_decay(HashMap::from([(3usize, 3u8)]));
        let g = compute_ghosts("same\n", "same\n", &existing);
        assert_eq!(g.decay_of(3), Some(2));
    }

    #[test]
    fn glow_and_word_ranges_expire_together() {
        let mut g = Ghosts {
            decay: HashMap::from([(0usize, 1u8)]),
            words: HashMap::from([(0usize, vec![(0, 3)])]),
        };
        g = compute_ghosts("x\n", "x\n", &g);
        assert!(g.is_empty(), "{g:?}");
        assert!(
            g.words.is_empty(),
            "word ranges must expire with the line: {g:?}"
        );
    }

    #[test]
    fn pure_deletion_relights_nothing() {
        let g = lit("a\nb\nc\n", "a\nc\n");
        assert!(g.is_empty(), "{g:?}");
    }

    #[test]
    fn freshest_changed_line_picks_the_lowest_max_decay_line() {
        let g = Ghosts::from_decay(HashMap::from([
            (5usize, GHOST_MAX_DECAY),
            (2usize, GHOST_MAX_DECAY),
            (7usize, 2u8),
        ]));
        assert_eq!(g.freshest_changed_line(), Some(2));
    }

    #[test]
    fn freshest_changed_line_is_none_without_a_fresh_glow() {
        assert_eq!(Ghosts::default().freshest_changed_line(), None);
        assert_eq!(
            Ghosts::from_decay(HashMap::from([(3usize, 2u8)])).freshest_changed_line(),
            None
        );
    }
}
