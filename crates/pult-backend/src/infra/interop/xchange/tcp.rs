//! TCP mode: mDNS discovery, and the protocol's own framing over short connections.
//!
//! # The group is the address
//!
//! The specification asks a client to register **both** `_mvrxchange._tcp.local.` and
//! a sub service `<group>._mvrxchange._tcp.local.`, and that is what this does — so a
//! console browsing the plain service sees everybody on the network, and browsing its
//! own group's sub service sees the stations it can actually exchange with. The panel
//! is better for having both: "there is a grandMA3 here, in another group" is a far
//! more useful thing to be told than nothing at all.
//!
//! `mdns-sd` accepts a group as the domain half of the type because its own validation
//! looks at the *last* label before `.local.` — `MyStation.Default._mvrxchange._tcp.`
//! ends in `_tcp`, which is all it asks for. That is a fact about the library as much
//! as about the protocol, and it is why this works without a fork.
//!
//! # A connection is short
//!
//! "Each TCP interaction is a temporary, short-lived connection", and the answer comes
//! back on the same one. So an outbound message opens a socket, writes, reads one
//! reply and closes; an inbound one is read, answered and left for the other end to
//! close. Nothing here holds a connection open between messages, which is why there is
//! no reconnection logic and no liveness timer: discovery *is* the liveness.

use std::net::SocketAddr;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use pult_mvr_xchange::{frame, Decoder, Message, Payload, SERVICE_TYPE};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use super::{Discovered, Endpoint, Reply, XchangeCommand};

/// How long to wait for the other end of a connection we opened.
///
/// A whole rig is megabytes over a venue's network, so this is generous; what it
/// guards against is a station that accepted the connection and then said nothing,
/// which would otherwise hold a commit announcement for ever.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// What TCP mode holds while it is running.
pub struct Tcp {
    daemon: ServiceDaemon,
    /// The fullnames registered, so they can be withdrawn again. A console that opened
    /// three shows in a row would otherwise be three responders on the network.
    registered: Vec<String>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    pub port: u16,
}

impl Tcp {
    /// Stop advertising and stop listening.
    ///
    /// Unregistering is best-effort and the tasks are aborted: what matters is that
    /// the port is released before a replacement tries to bind it, and that is what
    /// dropping the listener does.
    pub fn stop(self) {
        for fullname in &self.registered {
            let _ = self.daemon.unregister(fullname);
        }
        for task in self.tasks {
            task.abort();
        }
        if let Err(e) = self.daemon.shutdown() {
            debug!("[xchange] mDNS shutdown: {e}");
        }
    }
}

/// The two service types this console registers and browses: everybody, and the group.
pub fn service_types(group: &str) -> (String, String) {
    (SERVICE_TYPE.to_string(), format!("{group}.{SERVICE_TYPE}"))
}

/// Bind a port, advertise on it, and start listening and browsing.
pub async fn start(
    group: &str,
    station_uuid: Uuid,
    station_name: &str,
    max_file_bytes: u64,
    to_manager: mpsc::Sender<XchangeCommand>,
    net: crate::infra::net::NetHandle,
) -> Result<Tcp, String> {
    // Told a cable this machine has not got: the exchange does not go on the wire.
    // A previz seat finding this console on the house LAN when the operator said the
    // lighting one is exactly what the setting is for, so this one refuses rather
    // than falling back — see `infra::net`.
    let address = net
        .bind(
            pult_schema::types::network::NetService::MvrXchange,
            "MVR-xchange",
            net.prefs().mvr_xchange.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    let listener = TcpListener::bind((address.unwrap_or(std::net::Ipv4Addr::UNSPECIFIED), 0))
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let daemon = ServiceDaemon::new().map_err(|e| format!("no mDNS daemon: {e}"))?;
    crate::infra::net::restrict_mdns(&daemon, address);
    let (everybody, ours) = service_types(group);

    // The instance name is this console's station name. Two stations of one show never
    // both advertise — only the leader is on the wire — so there is nothing here to
    // collide with itself.
    let instance = sanitise(station_name);
    let properties = [("StationName", station_name), ("StationUUID", &station_uuid.to_string())];

    let mut registered = Vec::new();
    for ty in [&everybody, &ours] {
        let info = ServiceInfo::new(ty, &instance, &host_name(), (), port, &properties[..])
            .map_err(|e| format!("cannot describe the service: {e}"))?
            .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        match daemon.register(info) {
            Ok(()) => registered.push(fullname),
            Err(e) => warn!("[xchange] could not advertise {ty}: {e}"),
        }
    }

    let mut tasks = Vec::new();
    for (ty, in_group) in [(everybody, false), (ours, true)] {
        match daemon.browse(&ty) {
            Ok(rx) => {
                let to_manager = to_manager.clone();
                // The receiver is a crossbeam channel on the daemon's own thread, so
                // it is read on a thread and forwarded, exactly as `infra/session`
                // does with the same library.
                let (tx, mut events) = mpsc::channel::<ServiceEvent>(64);
                std::thread::spawn(move || {
                    while let Ok(event) = rx.recv() {
                        if tx.blocking_send(event).is_err() {
                            break;
                        }
                    }
                });
                tasks.push(tokio::spawn(async move {
                    while let Some(event) = events.recv().await {
                        forward(event, in_group, &to_manager).await;
                    }
                }));
            }
            Err(e) => warn!("[xchange] could not browse {ty}: {e}"),
        }
    }

    tasks.push(tokio::spawn(listen(listener, max_file_bytes, to_manager)));

    Ok(Tcp { daemon, registered, tasks, port })
}

async fn forward(event: ServiceEvent, in_group: bool, to_manager: &mpsc::Sender<XchangeCommand>) {
    match event {
        ServiceEvent::ServiceResolved(info) => {
            let txt = |key: &str| info.get_property_val_str(key).unwrap_or_default().to_string();
            let Ok(station_uuid) = txt("StationUUID").parse::<Uuid>() else {
                // A responder with no station uuid is not an MVR-xchange client we can
                // do anything with: every message names a station, and one with no
                // name cannot be told apart from the next.
                debug!("[xchange] ignoring {} — no StationUUID", info.get_fullname());
                return;
            };
            let Some(addr) = info
                .get_addresses()
                .iter()
                .find(|ip| ip.is_ipv4())
                .map(|ip| SocketAddr::new(*ip, info.get_port()))
            else {
                return;
            };
            let name = match txt("StationName") {
                empty if empty.is_empty() => info.get_fullname().to_string(),
                name => name,
            };
            let _ = to_manager
                .send(XchangeCommand::Discovered(Discovered {
                    station_uuid,
                    station_name: name,
                    fullname: info.get_fullname().to_string(),
                    addr,
                    in_group,
                }))
                .await;
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            let _ = to_manager.send(XchangeCommand::Undiscovered { fullname }).await;
        }
        _ => {}
    }
}

/// Accept connections and answer what arrives on them.
async fn listen(
    listener: TcpListener,
    max_file_bytes: u64,
    to_manager: mpsc::Sender<XchangeCommand>,
) {
    loop {
        let Ok((stream, addr)) = listener.accept().await else { continue };
        let to_manager = to_manager.clone();
        tokio::spawn(async move {
            if let Err(e) = serve(stream, addr, max_file_bytes, &to_manager).await {
                // At debug: a port on an open network is scanned, and every scan
                // would otherwise be a warning in an operator's log.
                debug!("[xchange] connection from {addr} ended: {e}");
            }
        });
    }
}

async fn serve(
    mut stream: TcpStream,
    addr: SocketAddr,
    max_file_bytes: u64,
    to_manager: &mpsc::Sender<XchangeCommand>,
) -> Result<(), String> {
    let mut decoder = Decoder::new(max_file_bytes);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let read = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
        if read == 0 {
            return Ok(());
        }
        decoder.feed(&buf[..read]);
        while let Some(payload) = decoder.next().map_err(|e| e.to_string())? {
            // A file arriving unasked is not something this protocol has a message
            // for, and buffering one would be a way to fill a disk. The only file we
            // accept is the answer to a request we made, which arrives on the socket
            // that made it.
            let Payload::Json(bytes) = payload else {
                return Err("a file arrived on a connection nothing had asked on".into());
            };
            let message = Message::from_json(&bytes).map_err(|e| e.to_string())?;
            let (tx, rx) = oneshot::channel();
            to_manager
                .send(XchangeCommand::Incoming {
                    message,
                    from: Endpoint::Tcp(addr),
                    reply: tx,
                })
                .await
                .map_err(|_| "the exchange has stopped".to_string())?;
            if let Ok(Some(reply)) = rx.await {
                write(&mut stream, reply).await?;
            }
        }
    }
}

async fn write(stream: &mut TcpStream, reply: Reply) -> Result<(), String> {
    let framed = match reply {
        Reply::Message(message) => {
            frame::encode(frame::Kind::Json, &message.to_json().map_err(|e| e.to_string())?)
        }
        Reply::File(bytes) => frame::encode(frame::Kind::File, &bytes),
    };
    stream.write_all(&framed).await.map_err(|e| e.to_string())
}

/// Send one message to one station, and hand back whatever came back.
///
/// The reply may be a file, which is what makes this the fetch path as well as the
/// announcement path: `MVR_REQUEST` answered with bytes is the same exchange as
/// `MVR_COMMIT` answered with an acknowledgement.
pub async fn send(
    addr: SocketAddr,
    message: &Message,
    max_file_bytes: u64,
) -> Result<Option<Payload>, String> {
    let attempt = async {
        let mut stream = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
        let framed = frame::encode(frame::Kind::Json, &message.to_json().map_err(|e| e.to_string())?);
        stream.write_all(&framed).await.map_err(|e| e.to_string())?;

        let mut decoder = Decoder::new(max_file_bytes);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let read = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
            if read == 0 {
                // The other end closed without answering. Legal for a message that
                // wants no answer, and the caller decides whether it wanted one.
                return Ok(None);
            }
            decoder.feed(&buf[..read]);
            if let Some(payload) = decoder.next().map_err(|e| e.to_string())? {
                return Ok(Some(payload));
            }
        }
    };

    match tokio::time::timeout(REPLY_TIMEOUT, attempt).await {
        Ok(result) => result,
        Err(_) => Err(format!("{addr} did not answer within {REPLY_TIMEOUT:?}")),
    }
}

/// An mDNS instance name, which may not carry a dot.
///
/// A show called "Hamlet, Act 1. Revised" is an ordinary thing to call a show and not
/// an ordinary thing to put in a DNS label.
fn sanitise(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '.' || c.is_control() { '-' } else { c })
        .take(48)
        .collect();
    if cleaned.trim().is_empty() {
        "pult".to_string()
    } else {
        cleaned
    }
}

fn host_name() -> String {
    let raw = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "pult".to_string());
    let base = raw.trim_end_matches(".local").trim_end_matches('.');
    format!("{}.local.", sanitise(base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_group_is_the_domain_half_of_the_type() {
        let (everybody, ours) = service_types("Default");
        assert_eq!(everybody, "_mvrxchange._tcp.local.");
        assert_eq!(ours, "Default._mvrxchange._tcp.local.");
    }

    /// The thing that makes the sub service work at all: `mdns-sd` validates the last
    /// label before `.local.`, so a group in the middle is accepted. If a future
    /// version of the library tightens that, this is where it will be noticed.
    #[test]
    fn mdns_accepts_a_group_as_part_of_the_service_type() {
        let (_, ours) = service_types("Default");
        let info = ServiceInfo::new(&ours, "Festival", "pult.local.", (), 7700, &[("StationName", "Festival")][..]);
        assert!(info.is_ok(), "mdns-sd refused a group sub service: {:?}", info.err());
        assert_eq!(info.unwrap().get_fullname(), "Festival.Default._mvrxchange._tcp.local.");
    }

    #[test]
    fn a_show_name_with_a_dot_in_it_is_still_an_instance_name() {
        assert_eq!(sanitise("Hamlet, Act 1. Revised"), "Hamlet, Act 1- Revised");
        assert_eq!(sanitise(""), "pult");
        assert_eq!(sanitise("   "), "pult");
        assert_eq!(sanitise(&"x".repeat(200)).len(), 48);
    }

    #[tokio::test]
    async fn a_message_sent_to_a_listener_is_answered_on_the_same_connection() {
        let (tx, mut rx) = mpsc::channel(8);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(listen(listener, 1024 * 1024, tx));

        // Something to answer with, standing in for the manager.
        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                if let XchangeCommand::Incoming { message, reply, .. } = cmd {
                    assert_eq!(message.type_name(), "MVR_LEAVE");
                    let _ = reply.send(Some(Reply::Message(Message::LeaveRet {
                        response: pult_mvr_xchange::message::Response::ok(),
                    })));
                }
            }
        });

        let answer = send(addr, &Message::Leave { from_station: Uuid::nil() }, 1024 * 1024)
            .await
            .unwrap()
            .expect("an answer");
        let Payload::Json(bytes) = answer else { panic!("a file came back") };
        assert_eq!(Message::from_json(&bytes).unwrap().type_name(), "MVR_LEAVE_RET");
    }

    #[tokio::test]
    async fn a_file_is_answered_as_a_file() {
        let (tx, mut rx) = mpsc::channel(8);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(listen(listener, 1024 * 1024, tx));
        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                if let XchangeCommand::Incoming { reply, .. } = cmd {
                    let _ = reply.send(Some(Reply::File(b"PK\x03\x04 an archive".to_vec())));
                }
            }
        });

        let answer =
            send(addr, &Message::Request { file_uuid: None, from_station: None }, 1024 * 1024)
                .await
                .unwrap()
                .expect("an answer");
        assert_eq!(answer, Payload::File(b"PK\x03\x04 an archive".to_vec()));
    }

    /// The cap is the whole of what stands between an unauthenticated LAN and this
    /// station's memory, so it has to hold on the *listening* side too.
    #[tokio::test]
    async fn a_message_over_the_cap_closes_the_connection_rather_than_being_read() {
        let (tx, mut rx) = mpsc::channel(8);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(listen(listener, 16, tx));
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let mut stream = TcpStream::connect(addr).await.unwrap();
        let big = frame::encode(frame::Kind::File, &vec![0u8; 4096]);
        // Only the header: the refusal must not need the payload to arrive.
        let _ = stream.write_all(&big[..28]).await;
        let mut buf = [0u8; 8];
        assert_eq!(stream.read(&mut buf).await.unwrap(), 0, "the connection should be closed");
    }
}
