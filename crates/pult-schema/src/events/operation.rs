use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{lifecycle::Lifecycle, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A Lamport-style vector clock for causal ordering of distributed operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VectorClock(pub HashMap<NodeId, u64>);

impl VectorClock {
    pub fn increment(&mut self, node: NodeId) -> u64 {
        let counter = self.0.entry(node).or_insert(0);
        *counter += 1;
        *counter
    }

    pub fn merge(&mut self, other: &VectorClock) {
        for (node, &counter) in &other.0 {
            let entry = self.0.entry(*node).or_insert(0);
            if counter > *entry {
                *entry = counter;
            }
        }
    }

    pub fn happens_before(&self, other: &VectorClock) -> bool {
        let mut at_least_one_less = false;
        for (node, &other_counter) in &other.0 {
            let self_counter = self.0.get(node).copied().unwrap_or(0);
            if self_counter > other_counter {
                return false;
            }
            if self_counter < other_counter {
                at_least_one_less = true;
            }
        }
        for (node, &self_counter) in &self.0 {
            if !other.0.contains_key(node) && self_counter > 0 {
                return false;
            }
        }
        at_least_one_less
    }
}

/// A single mutation wrapped with distributed metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: Uuid,
    pub node_id: NodeId,
    /// Monotonically increasing sequence number on the originating node.
    pub seq: u64,
    pub clock: VectorClock,
    pub lifecycle: Lifecycle,
    pub path: Path,
    pub value: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

impl Operation {
    pub fn new(
        node_id: NodeId,
        seq: u64,
        clock: VectorClock,
        lifecycle: Lifecycle,
        path: Path,
        value: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            node_id,
            seq,
            clock,
            lifecycle,
            path,
            value,
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_clock_happens_before() {
        let a = NodeId(Uuid::new_v4());
        let b = NodeId(Uuid::new_v4());

        let mut clock1 = VectorClock::default();
        clock1.increment(a);

        let mut clock2 = VectorClock::default();
        clock2.increment(a);
        clock2.increment(b);

        assert!(clock1.happens_before(&clock2));
        assert!(!clock2.happens_before(&clock1));
    }
}
