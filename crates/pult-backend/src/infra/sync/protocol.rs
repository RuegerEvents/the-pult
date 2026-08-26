use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

use pult_schema::{
    events::operation::{NodeId, Operation, VectorClock},
    path::Path,
};

// 2 added node_id to HelloAck. Without it the connecting side had no way to learn
// who it had just connected to and used the leader's id instead, so two nodes
// connecting to the same leader collided in the peer map.
// 3 put the joiner's clock in Hello and made OperationBatch carry operations, so a
// peer that reconnects can be told what it missed instead of being sent the show.
// 4 put the responder's clock in HelloAck, so catch-up runs both ways and the two
// sides converge whichever of them was behind.
pub const PROTOCOL_VERSION: u32 = 4;

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024; // 8 MiB safety cap

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncMessage {
    Hello {
        node_id: NodeId,
        protocol_version: u32,
        session_id: Uuid,
        show_id: Uuid,
        /// What the joiner already knows. The responder uses it to decide between
        /// replaying the operations it missed and sending a whole snapshot.
        #[serde(default)]
        clock: VectorClock,
    },
    HelloAck {
        accepted: bool,
        /// Who is answering. This is the peer the connecting side is talking to,
        /// which is not necessarily the leader.
        node_id: NodeId,
        leader_node_id: NodeId,
        /// What the responder knows, so the connecting side can replay anything the
        /// responder is missing. A reconnecting node is not always the stale one.
        #[serde(default)]
        clock: VectorClock,
        rejection_reason: Option<String>,
    },
    LeaderChanged {
        new_leader_node_id: NodeId,
    },
    /// Who is in the session. Sent by the leader whenever its peer set changes.
    ///
    /// Every node holding the same membership is what lets an election need no
    /// messages: when the leader goes, each survivor removes it from the same list
    /// and picks the same replacement.
    SessionMembers {
        members: Vec<NodeId>,
    },
    /// Ask for everything the holder of `known` has not seen.
    OperationRequest {
        known: VectorClock,
    },
    /// Operations the receiver was missing, oldest first. Replayed in order, they
    /// land on the same state the sender has.
    OperationBatch {
        operations: Vec<Operation>,
    },
    /// Full ShowState snapshot sent by leader immediately after HelloAck.
    /// The joiner applies it so it starts with current data, not an empty slate.
    StateSnapshot {
        state: serde_json::Value,
    },
    /// Field/entity update — replicate to all nodes.
    /// Lifecycle is inferred from path + schema on the receiving end.
    SyncedBroadcast {
        node_id: NodeId,
        path: Path,
        value: serde_json::Value,
        clock: VectorClock,
    },
    Heartbeat {
        seq: u64,
    },
    HeartbeatAck {
        seq: u64,
    },
}

/// Frames are length-prefixed JSON. JSON handles all serde types cleanly (untagged enums,
/// serde_json::Value, heterogeneous maps) without bincode's deserialize_any restrictions.
pub async fn write_frame(w: &mut (impl AsyncWrite + Unpin), msg: &SyncMessage) -> Result<()> {
    let bytes = serde_json::to_vec(msg)?;
    let len = bytes.len() as u32;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_frame(r: &mut (impl AsyncRead + Unpin)) -> Result<SyncMessage> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        anyhow::bail!("frame too large: {len} bytes");
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}
