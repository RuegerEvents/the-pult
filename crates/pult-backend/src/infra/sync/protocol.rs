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

/// Read one frame.
///
/// **Not cancel-safe.** The length and the body are two reads, so dropping this
/// future between them leaves the body in the socket with nothing to say how long
/// it is, and every frame after that is read at the wrong offset. Never put a call
/// to this directly in a `tokio::select!` branch — give it a task of its own and
/// select on a channel, which is what `run_peer_loop` does.
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


#[cfg(test)]
mod tests {
    use super::*;
    use pult_schema::events::operation::NodeId;
    use tokio::io::AsyncWriteExt;

    fn a_heartbeat(seq: u64) -> SyncMessage {
        SyncMessage::Heartbeat { seq }
    }

    #[tokio::test]
    async fn a_frame_round_trips() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        write_frame(&mut client, &a_heartbeat(7)).await.unwrap();

        let got = read_frame(&mut server).await.unwrap();

        assert!(matches!(got, SyncMessage::Heartbeat { seq: 7 }));
    }

    #[tokio::test]
    async fn two_frames_come_back_in_order() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        write_frame(&mut client, &a_heartbeat(1)).await.unwrap();
        write_frame(&mut client, &SyncMessage::HeartbeatAck { seq: 2 }).await.unwrap();

        assert!(matches!(read_frame(&mut server).await.unwrap(), SyncMessage::Heartbeat { seq: 1 }));
        assert!(matches!(
            read_frame(&mut server).await.unwrap(),
            SyncMessage::HeartbeatAck { seq: 2 }
        ));
    }

    /// The hazard `run_peer_loop` is built around, demonstrated.
    ///
    /// A cancelled `read_frame` has already taken the length prefix off the wire and
    /// has nowhere to put it back, so the next read treats the body as a length and
    /// the stream never recovers. In the peer loop this looked like a connection that
    /// died for no reason, rarely, under load — a whole afternoon of "flaky tests".
    ///
    /// If someone moves the read back into the `select!`, this test still passes, but
    /// it is here so the next person finds out why the task exists.
    #[tokio::test]
    async fn a_cancelled_read_desynchronises_the_stream() {
        let (mut client, mut server) = tokio::io::duplex(1024);

        // Only the length prefix, so the read has to wait for a body that is not
        // there yet — the moment a `select!` would cancel it.
        let frame = serde_json::to_vec(&a_heartbeat(1)).unwrap();
        client.write_all(&(frame.len() as u32).to_be_bytes()).await.unwrap();
        client.flush().await.unwrap();

        let cancelled = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            read_frame(&mut server),
        )
        .await;
        assert!(cancelled.is_err(), "the read has to be pending for this to prove anything");

        // The rest of the first frame, then a second, whole one.
        client.write_all(&frame).await.unwrap();
        write_frame(&mut client, &SyncMessage::HeartbeatAck { seq: 2 }).await.unwrap();

        // The four bytes that said how long the first frame was are gone, so this
        // reads the body as a length and gets nonsense rather than either message.
        let next = read_frame(&mut server).await;
        let recovered = matches!(
            next,
            Ok(SyncMessage::Heartbeat { seq: 1 }) | Ok(SyncMessage::HeartbeatAck { seq: 2 })
        );
        assert!(!recovered, "a cancelled read cannot be resumed; got {next:?}");
    }

    #[tokio::test]
    async fn a_frame_claiming_to_be_enormous_is_refused() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        client.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        client.flush().await.unwrap();

        assert!(read_frame(&mut server).await.is_err());
    }

    #[tokio::test]
    async fn a_hello_survives_the_wire_with_its_node_id() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let node_id = NodeId::new();
        write_frame(
            &mut client,
            &SyncMessage::Hello {
                node_id,
                protocol_version: PROTOCOL_VERSION,
                session_id: uuid::Uuid::new_v4(),
                show_id: uuid::Uuid::new_v4(),
                clock: Default::default(),
            },
        )
        .await
        .unwrap();

        match read_frame(&mut server).await.unwrap() {
            SyncMessage::Hello { node_id: got, protocol_version, .. } => {
                assert_eq!(got, node_id);
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            other => panic!("expected Hello, got {other:?}"),
        }
    }
}
