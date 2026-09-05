//! WebSocket mode: joining somebody's host, and being one.
//!
//! The framing here is RFC 6455's own — a text frame is JSON and a binary frame is an
//! MVR file — so none of [`pult_mvr_xchange::frame`] is involved. That is the whole
//! difference between the two modes at this level.
//!
//! # Hosting is a route, not a listener
//!
//! A host is "the IP of the service running the websocket server", and this console
//! already runs one on the port serving its own page. So hosting is `/mvrxchange` on
//! that router, and the address to give somebody is the one they already use to reach
//! the console. Nothing new is bound and nothing new has to be found.
//!
//! # What a host owes its clients
//!
//! In WebSocket mode the clients cannot see each other — every message goes through
//! the middle. Three consequences, all of them decided rather than fallen into:
//!
//! - **`MVR_JOIN_RET` carries the group's history**, not just ours. A previz joining
//!   at four o'clock learns about the three o'clock file from us or not at all.
//! - **A request for somebody else's file is relayed**, because there is no other path
//!   between them.
//! - **Relayed bytes are not cached.** The commit cache is what *this station*
//!   committed; holding everything that passes through would quietly make a console
//!   into a file store for other people's rigs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pult_mvr_xchange::Message;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use super::{Endpoint, Reply, XchangeCommand};

/// What a connection can be told to send.
#[derive(Debug, Clone)]
pub enum Out {
    Json(Message),
    File(Vec<u8>),
    Close,
}

/// The clients of the group this console is hosting.
///
/// Built before the router is, and handed to both — so `/mvrxchange` exists for the
/// life of the process and *answers* only while a manager has claimed it. A route that
/// appeared and disappeared with the exchange would mean rebuilding the router when a
/// show is opened, which is not a thing this server does.
#[derive(Clone, Default)]
pub struct HostRegistry(Arc<Inner>);

#[derive(Default)]
struct Inner {
    /// Where a client's messages go. `None` when this console is not hosting, which is
    /// what makes the route refuse rather than accept into nothing.
    manager: Mutex<Option<mpsc::Sender<XchangeCommand>>>,
    clients: Mutex<HashMap<u64, mpsc::Sender<Out>>>,
    next_id: AtomicU64,
}

impl HostRegistry {
    /// Start answering, with messages going to this manager.
    pub fn claim(&self, manager: mpsc::Sender<XchangeCommand>) {
        *self.0.manager.lock().unwrap() = Some(manager);
    }

    /// Stop answering, and close what is connected.
    ///
    /// The clients are told rather than dropped: a host that vanishes silently leaves
    /// somebody else's software reconnecting to a group that no longer exists.
    pub fn release(&self) {
        *self.0.manager.lock().unwrap() = None;
        for (_, sender) in self.0.clients.lock().unwrap().drain() {
            let _ = sender.try_send(Out::Close);
        }
    }

    pub fn hosting(&self) -> bool {
        self.0.manager.lock().unwrap().is_some()
    }

    fn manager(&self) -> Option<mpsc::Sender<XchangeCommand>> {
        self.0.manager.lock().unwrap().clone()
    }

    fn add(&self, sender: mpsc::Sender<Out>) -> u64 {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.0.clients.lock().unwrap().insert(id, sender);
        id
    }

    fn remove(&self, id: u64) {
        self.0.clients.lock().unwrap().remove(&id);
    }

    /// Send to one client.
    pub fn tell(&self, id: u64, out: Out) {
        let sender = self.0.clients.lock().unwrap().get(&id).cloned();
        if let Some(sender) = sender {
            // Dropped rather than queued if a client is not keeping up: a group
            // announcement must never be what holds up the console.
            let _ = sender.try_send(out);
        }
    }

    /// Send to every client but one — which is what a host does with a commit.
    pub fn tell_others(&self, except: Option<u64>, out: Out) {
        let senders: Vec<(u64, mpsc::Sender<Out>)> =
            self.0.clients.lock().unwrap().iter().map(|(id, s)| (*id, s.clone())).collect();
        for (id, sender) in senders {
            if Some(id) != except {
                let _ = sender.try_send(out.clone());
            }
        }
    }

    pub fn client_count(&self) -> usize {
        self.0.clients.lock().unwrap().len()
    }
}

/// Serve one client of the group this console is hosting.
///
/// Called from the axum route, which owns the upgrade. A socket that arrives while
/// nothing is hosting is closed at once rather than parked: a client left holding an
/// open connection to a console that is not in the group would wait for ever.
pub async fn serve_client(mut socket: axum::extract::ws::WebSocket, registry: HostRegistry) {
    use axum::extract::ws::Message as Frame;
    use futures::{SinkExt, StreamExt};

    let Some(manager) = registry.manager() else {
        let _ = socket.send(Frame::Close(None)).await;
        return;
    };

    let (tx, mut outgoing) = mpsc::channel::<Out>(32);
    let id = registry.add(tx);
    let (mut sink, mut stream) = socket.split();

    let sending = tokio::spawn(async move {
        while let Some(out) = outgoing.recv().await {
            let frame = match out {
                Out::Json(message) => match message.to_json() {
                    Ok(bytes) => Frame::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
                    Err(e) => {
                        warn!("[xchange] could not write a message: {e}");
                        continue;
                    }
                },
                Out::File(bytes) => Frame::Binary(bytes.into()),
                Out::Close => {
                    let _ = sink.send(Frame::Close(None)).await;
                    break;
                }
            };
            if sink.send(frame).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(frame)) = stream.next().await {
        match frame {
            Frame::Text(text) => match Message::from_json(text.as_bytes()) {
                Ok(message) => {
                    let (reply_tx, reply_rx) = oneshot::channel();
                    if manager
                        .send(XchangeCommand::Incoming {
                            message,
                            from: Endpoint::WsClient(id),
                            reply: reply_tx,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                    if let Ok(Some(reply)) = reply_rx.await {
                        registry.tell(id, reply.into());
                    }
                }
                Err(e) => debug!("[xchange] a client sent something that is not a message: {e}"),
            },
            // A client sending a file unasked. In a hosted group this is how the
            // *answer* to a relayed request arrives, so it goes to the manager, which
            // knows whether anything was waiting for it.
            Frame::Binary(bytes) => {
                let _ = manager
                    .send(XchangeCommand::FileArrived {
                        from: Endpoint::WsClient(id),
                        bytes: bytes.to_vec(),
                    })
                    .await;
            }
            Frame::Close(_) => break,
            _ => {}
        }
    }

    registry.remove(id);
    sending.abort();
    let _ = manager.send(XchangeCommand::ClientGone { id }).await;
}

impl From<Reply> for Out {
    fn from(reply: Reply) -> Out {
        match reply {
            Reply::Message(message) => Out::Json(message),
            Reply::File(bytes) => Out::File(bytes),
        }
    }
}

/// The connection to a host this console has joined.
pub struct Link {
    to_host: mpsc::Sender<Out>,
    task: tokio::task::JoinHandle<()>,
    pub url: String,
}

impl Link {
    pub fn tell(&self, out: Out) {
        let _ = self.to_host.try_send(out);
    }

    pub fn stop(self) {
        let _ = self.to_host.try_send(Out::Close);
        self.task.abort();
    }
}

/// Join a host, and keep the connection until told to stop.
///
/// One attempt, and a failure is reported rather than retried. A console that
/// reconnected on its own would be a console quietly rejoining a group somebody took
/// it out of, and the panel is where a person decides to try again.
pub async fn join(url: &str, to_manager: mpsc::Sender<XchangeCommand>) -> Result<Link, String> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Frame;

    let (socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("could not reach {url}: {e}"))?;
    let (mut sink, mut stream) = socket.split();
    let (tx, mut outgoing) = mpsc::channel::<Out>(32);

    let url_owned = url.to_string();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                out = outgoing.recv() => {
                    let Some(out) = out else { break };
                    let frame = match out {
                        Out::Json(message) => match message.to_json() {
                            Ok(bytes) => Frame::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
                            Err(e) => { warn!("[xchange] could not write a message: {e}"); continue }
                        },
                        Out::File(bytes) => Frame::Binary(bytes.into()),
                        Out::Close => { let _ = sink.send(Frame::Close(None)).await; break }
                    };
                    if sink.send(frame).await.is_err() { break }
                }
                frame = stream.next() => {
                    match frame {
                        Some(Ok(Frame::Text(text))) => match Message::from_json(text.as_bytes()) {
                            Ok(message) => {
                                let (reply_tx, reply_rx) = oneshot::channel();
                                if to_manager
                                    .send(XchangeCommand::Incoming {
                                        message,
                                        from: Endpoint::WsHost,
                                        reply: reply_tx,
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                // The answer goes back the way it came, on the one
                                // connection this console has to the group.
                                if let Ok(Some(reply)) = reply_rx.await {
                                    let frame = match Out::from(reply) {
                                        Out::Json(m) => match m.to_json() {
                                            Ok(b) => Frame::Text(String::from_utf8_lossy(&b).into_owned().into()),
                                            Err(_) => continue,
                                        },
                                        Out::File(b) => Frame::Binary(b.into()),
                                        Out::Close => break,
                                    };
                                    if sink.send(frame).await.is_err() { break }
                                }
                            }
                            Err(e) => debug!("[xchange] the host sent something unreadable: {e}"),
                        },
                        Some(Ok(Frame::Binary(bytes))) => {
                            let _ = to_manager
                                .send(XchangeCommand::FileArrived {
                                    from: Endpoint::WsHost,
                                    bytes: bytes.to_vec(),
                                })
                                .await;
                        }
                        Some(Ok(_)) => {}
                        // The host went away. Said once, and not retried.
                        Some(Err(_)) | None => {
                            let _ = to_manager
                                .send(XchangeCommand::HostLost(format!("the host at {url_owned} closed the connection")))
                                .await;
                            break;
                        }
                    }
                }
            }
        }
    });

    Ok(Link { to_host: tx, task, url: url.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registry_answers_nothing_until_it_is_claimed() {
        let registry = HostRegistry::default();
        assert!(!registry.hosting());
        let (tx, _rx) = mpsc::channel(1);
        registry.claim(tx);
        assert!(registry.hosting());
        registry.release();
        assert!(!registry.hosting());
    }

    #[tokio::test]
    async fn a_host_tells_everybody_but_the_sender() {
        let registry = HostRegistry::default();
        let (a_tx, mut a) = mpsc::channel(4);
        let (b_tx, mut b) = mpsc::channel(4);
        let a_id = registry.add(a_tx);
        let _b_id = registry.add(b_tx);

        registry.tell_others(Some(a_id), Out::Json(Message::Unknown));
        assert!(a.try_recv().is_err(), "the sender is not told its own commit");
        assert!(b.try_recv().is_ok());
    }

    #[tokio::test]
    async fn releasing_closes_what_is_connected() {
        let registry = HostRegistry::default();
        let (tx, mut client) = mpsc::channel(4);
        registry.add(tx);
        registry.release();
        assert!(matches!(client.try_recv(), Ok(Out::Close)));
        assert_eq!(registry.client_count(), 0);
    }
}
