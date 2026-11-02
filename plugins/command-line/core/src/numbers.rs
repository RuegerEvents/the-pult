//! Which fixtures a typed number means.
//!
//! `fixture 3` is the fixture whose *number* is 3 — `Fixture::fixture_number`, the one
//! on its label and in the Fixtures panel — and never the third row of the patch: a
//! row's place moves whenever something is patched or deleted above it, and the wing
//! is typed from memory. A station numbers every fixture it patches without one, so a
//! fixture with no number is one from before that, and answers to nothing until it has
//! one.

use crate::Range;

/// The fixtures numbered `from` to `to`, as indices into `numbers`, in number order and
/// patch order within a number — two fixtures may share one, and `fixture 5` then means
/// both. Gaps are not an error: `1 thru 10` is whatever of those numbers exists.
pub fn numbered(numbers: &[Option<u32>], range: Range) -> Vec<usize> {
    let (from, to) = (range.from.min(range.to), range.from.max(range.to));
    let mut hits: Vec<(u32, usize)> = numbers
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.filter(|&n| (from..=to).contains(&(n as usize))).map(|n| (n, i)))
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_is_the_label_not_the_row() {
        let numbers = [Some(101), None, Some(3), Some(102)];
        assert_eq!(numbered(&numbers, Range { from: 3, to: 3 }), vec![2]);
        assert_eq!(numbered(&numbers, Range { from: 1, to: 1 }), Vec::<usize>::new());
        assert_eq!(numbered(&numbers, Range { from: 100, to: 200 }), vec![0, 3]);
    }

    #[test]
    fn a_shared_number_is_every_fixture_carrying_it_in_patch_order() {
        let numbers = [Some(5), Some(2), Some(5)];
        assert_eq!(numbered(&numbers, Range { from: 1, to: 9 }), vec![1, 0, 2]);
    }
}
