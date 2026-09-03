//! The bounded buffer the panel reads.

use std::collections::VecDeque;

use pult_schema::ws::{LogLevel, LogLine};

/// The last so many lines, oldest first, from every station this one has heard
/// from.
///
/// One ring rather than one per station: a merged view is what the panel shows,
/// `log.tail` answers it in one call, and a line already carries the station that
/// said it. Ordering inside the ring is arrival order, which for this station's own
/// lines is emission order; a browser re-sorts by `at_ms` for the merged view and
/// dedupes by `(node_id, seq)`, so nothing here has to be clever about a peer whose
/// clock disagrees.
pub struct LogRing {
    lines: VecDeque<LogLine>,
    cap: usize,
}

impl LogRing {
    pub fn new(cap: usize) -> LogRing {
        LogRing { lines: VecDeque::with_capacity(cap.min(1024)), cap: cap.max(1) }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn push(&mut self, line: LogLine) {
        if self.lines.len() == self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    /// The most recent `limit` lines that a listener at `level` would keep, oldest
    /// first — which is the order a panel appends them in, so it does not have to
    /// reverse what it is given.
    pub fn tail(&self, limit: usize, level: Option<LogLevel>) -> Vec<LogLine> {
        let mut out: Vec<LogLine> = self
            .lines
            .iter()
            .rev()
            .filter(|l| level.is_none_or(|want| l.level.passes(want)))
            .take(limit)
            .cloned()
            .collect();
        out.reverse();
        out
    }
}
