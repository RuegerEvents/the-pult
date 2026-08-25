use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lifecycle {
    /// Local to this backend node. Synced to connected frontends but not to peer nodes.
    Local,
    /// Broadcast to all peer backends and all frontends. Not persisted.
    Synced,
    /// Written to SQLite and replicated to all peers and frontends.
    Persisted,
}
