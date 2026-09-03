use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

use pult_schema::{
    events::operation::{Authorship, NodeId, Operation, VectorClock},
    path::Path,
};

// 2 added node_id to HelloAck. Without it the connecting side had no way to learn
// who it had just connected to and used the leader's id instead, so two nodes
// connecting to the same leader collided in the peer map.
// 3 put the joiner's clock in Hello and made OperationBatch carry operations, so a
// peer that reconnects can be told what it missed instead of being sent the show.
// 4 put the responder's clock in HelloAck, so catch-up runs both ways and the two
// sides converge whichever of them was behind.
// 5 added LogLines and LogRaise, so the booth can read the roof station's log.
// Deliberately not carried on SyncedBroadcast: a log line is not show state, has
// no vector clock and no author, and must not be replicated, persisted or undone.
pub const PROTOCOL_VERSION: u32 = 5;

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
        /// Who made the change, what it replaced, and which gesture it was part of.
        ///
        /// Flattened so the three read as fields of the message, which is what they
        /// were before they were a struct, and defaulted so a peer running an older
        /// build still parses it — its writes simply arrive unattributed, which is
        /// what they are.
        #[serde(flatten)]
        authorship: Authorship,
    },
    /// Log lines from the sender, at whatever it is currently publishing.
    ///
    /// One way and unacknowledged: a log that a peer must confirm having read is a
    /// log that can hold up the show. Lines carry the *sender's* `seq` and clock,
    /// which is what lets the receiving browser dedupe them exactly and notice a
    /// gap, and they are never relayed onward — every station is connected to every
    /// other, so a relay would only duplicate.
    LogLines {
        node_id: NodeId,
        lines: Vec<pult_schema::ws::LogLine>,
    },
    /// Ask the peer on the other end of *this* connection to publish more.
    ///
    /// `None` withdraws the ask. The receiver clamps it to its own capture level —
    /// it cannot send what it never kept — and the raise lives and dies with this
    /// connection, so nothing has to expire and nothing is left raised by a console
    /// that went away.
    LogRaise {
        level: Option<pult_schema::ws::LogLevel>,
    },
    Heartbeat {
        seq: u64,
    },
    HeartbeatAck {
        seq: u64,
    },
}

/// What has crossed one peer link, each way.
///
/// Counted around the socket rather than at the twelve places that write a frame,
/// which is the difference between a figure and a figure with holes in it: the
/// handshake, the catch-up batches, the heartbeats and a raised log all go through
/// the same bytes, and none of them has to remember to say so.
///
/// Atomics rather than a lock, because both halves of a connection are written from
/// their own task and the station reporter reads them from a third every couple of
/// seconds. Relaxed ordering: these are counters for a person to read, and a byte
/// landing in this window or the next one is not a question anybody can answer.
#[derive(Debug, Default)]
pub struct LinkBytes {
    pub sent: std::sync::atomic::AtomicU64,
    pub received: std::sync::atomic::AtomicU64,
}

impl LinkBytes {
    /// Take what has accumulated and start again, as `(sent, received)`.
    pub fn take(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.sent.swap(0, Relaxed), self.received.swap(0, Relaxed))
    }
}

/// A stream that counts what goes through it.
///
/// Wrapped around the `TcpStream` *before* it is split, so both halves count into the
/// same pair and nothing downstream changes: `write_frame` and `read_frame` take an
/// `AsyncWrite` and an `AsyncRead` and do not care what is underneath.
///
/// `S: Unpin` rather than a pin projection, which is what lets this be forty lines and
/// no new dependency. Every stream it is used on — a `TcpStream` and its halves — is
/// `Unpin`, and a wrapper that only counts has no self-referential state to protect.
///
/// It counts what the socket accepted, which is the honest place to count. The
/// four-byte length prefix is in the figure because it is on the wire; the TCP, IP and
/// Ethernet headers under all of it are not, so a cable carries a little more than
/// this says.
pub struct Counted<S> {
    inner: S,
    bytes: std::sync::Arc<LinkBytes>,
}

impl<S> Counted<S> {
    pub fn new(inner: S, bytes: std::sync::Arc<LinkBytes>) -> Self {
        Counted { inner, bytes }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let polled = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(n)) = &polled {
            self.bytes.sent.fetch_add(*n as u64, std::sync::atomic::Ordering::Relaxed);
        }
        polled
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &polled {
            let read = buf.filled().len().saturating_sub(before);
            self.bytes.received.fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
        }
        polled
    }
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
