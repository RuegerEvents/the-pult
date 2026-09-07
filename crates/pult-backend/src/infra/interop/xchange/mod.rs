//! MVR-xchange on this station: who is in the group, what they have, and the two acts
//! an operator can perform — commit, and apply.
//!
//! `pult-mvr-xchange` is the protocol on paper. This is the console's half: the mDNS
//! daemon, the sockets, the cache, and the rules about who is allowed to do what. The
//! rules, in one place, because every one of them was a decision:
//!
//! **Only the leader is on the wire.** One exchange client per show — see
//! [`pult_schema::types::xchange`] for why. Every station holds the state and only one
//! holds the connections.
//!
//! **A commit is deliberate.** A button and a comment, never a change to the rig. The
//! whole rig, never a subset and never targeted: `ForStationsUUID` is honoured on the
//! way in and always empty on the way out.
//!
//! **Applying is one act and one gesture.** Apply fetches the bytes and runs the same
//! import `POST /api/import/mvr` runs, attributed to the operator who asked — from
//! whichever station they asked at. If a sequence is live the caller is told what,
//! once, and asks again to go ahead: a console cannot put a dialog in the room, so the
//! decision goes back to the person who can see it.
//!
//! **Nothing is fetched that nobody asked for.** An announcement is a claim that bytes
//! exist; this station reads them only when somebody applies.
//!
//! **Admission is a `MVR_JOIN` and a size cap.** The protocol has no authentication of
//! any kind, so what stands between a machine on the venue's wifi and this console is
//! that cap, enforced against the declared length before a byte is buffered, and a log
//! line for every join, request and refusal.

pub mod cache;
mod tcp;
pub mod ws;

use std::collections::BTreeMap;
use std::net::SocketAddr;

use pult_mvr_xchange::{
    message::{Commit, Response},
    Message, Payload, MVR_VERSION,
};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        xchange::station_uuid_for, PendingHost, XchangeAsk, XchangeCommit, XchangeIdle,
        XchangeMode, XchangeSettings, XchangeState, XchangeStation,
    },
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::engine::EngineHandle;
use crate::infra::{assets::AssetStore, sync::SyncHandle};

pub use cache::{CachedCommit, CommitCache};
pub use ws::HostRegistry;

/// How this console names itself to a group.
const PROVIDER: &str = "the-pult";

/// Where a message came from, and where an answer goes back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A station reached over TCP mode, at the address it advertised.
    Tcp(SocketAddr),
    /// A client of the group this console is hosting.
    WsClient(u64),
    /// The host this console has joined.
    WsHost,
}

/// What goes back down the connection a message arrived on.
#[derive(Debug, Clone)]
pub enum Reply {
    Message(Message),
    File(Vec<u8>),
}

/// A station found on mDNS.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub station_uuid: Uuid,
    pub station_name: String,
    pub fullname: String,
    pub addr: SocketAddr,
    /// Found under this show's own group, rather than under the plain service.
    pub in_group: bool,
}

pub enum XchangeCommand {
    /// The show, its settings, or which station is leading may have changed.
    Reconsider,
    Ask { ask: XchangeAsk, reply: Option<oneshot::Sender<Result<serde_json::Value, String>>> },
    Incoming { message: Message, from: Endpoint, reply: oneshot::Sender<Option<Reply>> },
    /// A file arrived on a connection. Only ever wanted when something is waiting.
    FileArrived { from: Endpoint, bytes: Vec<u8> },
    Discovered(Discovered),
    Undiscovered { fullname: String },
    ClientGone { id: u64 },
    HostLost(String),
    Stop,
}

#[derive(Clone)]
pub struct XchangeHandle(pub mpsc::Sender<XchangeCommand>);

impl XchangeHandle {
    /// Work out again what should be running. Cheap, and safe to send at any time.
    pub async fn reconsider(&self) {
        let _ = self.0.send(XchangeCommand::Reconsider).await;
    }

    /// Do something, and wait for the answer.
    pub async fn ask(&self, ask: XchangeAsk) -> Result<serde_json::Value, String> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(XchangeCommand::Ask { ask, reply: Some(tx) })
            .await
            .map_err(|_| "the exchange has stopped".to_string())?;
        rx.await.unwrap_or_else(|_| Err("the exchange dropped the question".into()))
    }

    /// Do something asked for by an operator at another station.
    ///
    /// No answer goes back: the relay is one way, and what a follower's operator sees
    /// is the state changing — or, if it failed, the `warn` line, which crosses the
    /// sync link to every station's System Log by the path task 48 already built.
    pub async fn ask_without_answer(&self, ask: XchangeAsk) {
        let _ = self.0.send(XchangeCommand::Ask { ask, reply: None }).await;
    }

    pub async fn stop(&self) {
        let _ = self.0.send(XchangeCommand::Stop).await;
    }
}

/// What this station's own configuration says.
pub struct XchangeLimits {
    /// Whether this machine may take part at all — the `preferences.toml` veto.
    pub allowed: bool,
    /// How many of this station's own commits are kept.
    pub keep: usize,
    /// The largest file this station will read, in bytes.
    pub max_file_bytes: u64,
}

/// What is running, where.
enum Transport {
    Tcp(tcp::Tcp),
    /// A host somebody else runs, that this console has joined.
    Joined(ws::Link),
    /// The group this console is hosting.
    Hosting,
}

/// A station as this console currently knows it.
#[derive(Debug, Clone)]
struct Known {
    name: String,
    provider: String,
    endpoint: Endpoint,
    fullname: Option<String>,
    joined: bool,
    in_group: bool,
    /// Whether [`Known::endpoint`] is somewhere this console can open a connection to.
    ///
    /// In TCP mode it usually is not. A message arriving over TCP comes from the
    /// *ephemeral* port the sender dialled out of, and every interaction is a short
    /// connection — so the address a join was received from is dead the moment it
    /// closes. The listening address comes from mDNS and from nowhere else, which is
    /// why discovery is not optional in that mode however much is already known.
    ///
    /// A WebSocket endpoint is always dialable: the connection *is* the address, and
    /// it stays open.
    dialable: bool,
}

/// What this console is waiting for a binary frame to be.
///
/// One at a time, and that is a fact about the protocol rather than a simplification:
/// a `MVR_REQUEST` carries no correlation id, so a second fetch in flight would have no
/// way of telling which answer was whose.
enum Awaiting {
    /// A commit this console is applying.
    Ours {
        file_uuid: Uuid,
        user_id: Uuid,
        /// Who was asked. A file arriving from anywhere else is not the answer to
        /// this question, and taking it would let anything on the group put a rig in
        /// front of an operator who asked a different station for one.
        from: Endpoint,
        reply: Option<oneshot::Sender<Result<serde_json::Value, String>>>,
    },
    /// Somebody else's file, on its way through a group we host.
    Relay { to: Endpoint },
}

pub struct XchangeManager {
    station_label: String,
    /// Which cable the exchange goes out on. Only the leader is ever on the wire, so
    /// this is asked once per transport start rather than held open.
    net: crate::infra::net::NetHandle,
    engine: EngineHandle,
    assets: AssetStore,
    sync: Option<SyncHandle>,
    registry: HostRegistry,
    limits: XchangeLimits,
    rx: mpsc::Receiver<XchangeCommand>,
    self_tx: mpsc::Sender<XchangeCommand>,

    show: Option<(Uuid, String)>,
    settings: XchangeSettings,
    leading: bool,
    transport: Option<Transport>,
    cache: CommitCache,
    stations: BTreeMap<Uuid, Known>,
    /// Everybody's commits, keyed by file uuid. Ours are rebuilt from the cache, so
    /// this station never announces a file it cannot serve.
    commits: BTreeMap<Uuid, XchangeCommit>,
    pending_host: Option<PendingHost>,
    awaiting: Option<Awaiting>,
    idle: Option<XchangeIdle>,
    /// Whether this station has ever said what it is doing.
    ///
    /// A console that comes up already in its settled state — the ordinary case, with
    /// the exchange switched off — changes nothing on its first look and would take
    /// the early return below without ever publishing, leaving every panel showing the
    /// default `XchangeState` and no reason in it. Found by a test asking a station
    /// with the exchange off why it was off, and being told nothing.
    published: bool,
}

impl XchangeManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        station_label: String,
        engine: EngineHandle,
        assets: AssetStore,
        registry: HostRegistry,
        cache_dir: std::path::PathBuf,
        limits: XchangeLimits,
        net: crate::infra::net::NetHandle,
    ) -> (Self, XchangeHandle) {
        let (tx, rx) = mpsc::channel(64);
        let cache = CommitCache::new(cache_dir, limits.keep);
        (
            XchangeManager {
                station_label,
                net,
                engine,
                assets,
                sync: None,
                registry,
                limits,
                rx,
                self_tx: tx.clone(),
                show: None,
                settings: XchangeSettings::default(),
                leading: true,
                transport: None,
                cache,
                stations: BTreeMap::new(),
                commits: BTreeMap::new(),
                pending_host: None,
                awaiting: None,
                idle: Some(XchangeIdle::NotEnabled),
                published: false,
            },
            XchangeHandle(tx),
        )
    }

    pub fn set_sync(&mut self, sync: SyncHandle) {
        self.sync = Some(sync);
    }

    pub async fn run(mut self) {
        // Anything that could change whether this should be running, or under what
        // name: the show's own settings, and which station is leading.
        for pattern in ["show", "session"] {
            let engine = self.engine.clone();
            let tx = self.self_tx.clone();
            tokio::spawn(async move {
                use futures::StreamExt;
                let mut updates =
                    engine.subscribe_pattern(pult_schema::path::PathPattern::new(pattern)).await;
                while updates.next().await.is_some() {
                    if tx.send(XchangeCommand::Reconsider).await.is_err() {
                        break;
                    }
                }
            });
        }

        self.reconsider().await;

        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                XchangeCommand::Stop => break,
                XchangeCommand::Reconsider => self.reconsider().await,
                XchangeCommand::Ask { ask, reply } => self.handle_ask(ask, reply).await,
                XchangeCommand::Incoming { message, from, reply } => {
                    let answer = self.handle_incoming(message, from).await;
                    let _ = reply.send(answer);
                    self.publish().await;
                }
                XchangeCommand::FileArrived { from, bytes } => {
                    self.handle_file(from, bytes).await;
                }
                XchangeCommand::Discovered(found) => {
                    self.handle_discovered(found).await;
                }
                XchangeCommand::Undiscovered { fullname } => {
                    self.stations.retain(|_, known| known.fullname.as_deref() != Some(&fullname));
                    self.publish().await;
                }
                XchangeCommand::ClientGone { id } => {
                    self.stations.retain(|_, known| known.endpoint != Endpoint::WsClient(id));
                    self.publish().await;
                }
                XchangeCommand::HostLost(why) => {
                    warn!("[xchange] {why}");
                    self.tear_down();
                    self.idle = Some(XchangeIdle::Failed(why));
                    self.publish().await;
                }
            }
        }
        self.tear_down();
        // A station going away says so rather than leaving a row on somebody's screen.
        self.idle = Some(XchangeIdle::NoShow);
        self.publish().await;
    }

    // ── What should be running ────────────────────────────────────────────────

    /// Read the show and the session, and start or stop accordingly.
    async fn reconsider(&mut self) {
        let show = self.read_show().await;
        let leading = !self.read_is_follower().await;
        let settings = show.as_ref().map(|(_, _, s)| s.clone()).unwrap_or_default();
        let show_id_name = show.map(|(id, name, _)| (id, name));

        let unchanged = show_id_name == self.show
            && settings == self.settings
            && leading == self.leading
            && self.transport.is_some() == self.should_run(&settings, &show_id_name, leading);
        if unchanged && self.published {
            return;
        }
        if unchanged {
            // Nothing to start or stop, but nobody has been told anything yet.
            self.idle = if self.should_run(&settings, &show_id_name, leading) {
                None
            } else {
                Some(self.why_not(&settings, &show_id_name, leading))
            };
            self.publish().await;
            return;
        }

        self.show = show_id_name;
        self.settings = settings;
        self.leading = leading;

        // Everything is torn down and rebuilt rather than reconciled. A group name is
        // the mDNS address, a mode is a different protocol and a URL is a different
        // machine, so there is no change here that leaves a connection meaningfully
        // still valid — and a reconciler for the one case that does would be a second
        // description of what "running" means.
        self.tear_down();
        self.stations.clear();
        self.commits.clear();
        self.pending_host = None;
        self.awaiting = None;

        let settings = self.settings.clone();
        let show = self.show.clone();
        if !self.should_run(&settings, &show, self.leading) {
            self.idle = Some(self.why_not(&settings, &show, self.leading));
            self.publish().await;
            return;
        }

        self.idle = None;
        // Our own commits are whatever this station actually holds — see the cache.
        self.load_our_commits();
        match self.start_transport().await {
            Ok(()) => {
                info!(
                    "[xchange] {} in group {:?} as {}",
                    match self.settings.mode {
                        XchangeMode::Tcp => "advertising",
                        XchangeMode::WebSocket => "joined",
                        XchangeMode::WebSocketHost => "hosting",
                    },
                    self.settings.group,
                    self.station_name(),
                );
            }
            Err(why) => {
                warn!("[xchange] could not start: {why}");
                self.idle = Some(XchangeIdle::Failed(why));
            }
        }
        self.publish().await;
    }

    fn should_run(
        &self,
        settings: &XchangeSettings,
        show: &Option<(Uuid, String)>,
        leading: bool,
    ) -> bool {
        self.limits.allowed && settings.enabled && show.is_some() && leading
    }

    fn why_not(
        &self,
        settings: &XchangeSettings,
        show: &Option<(Uuid, String)>,
        leading: bool,
    ) -> XchangeIdle {
        // Ordered so the most specific true thing is what a person is told. A console
        // with no show open is not "not enabled", it is a console with no show.
        if show.is_none() {
            XchangeIdle::NoShow
        } else if !self.limits.allowed {
            XchangeIdle::RefusedByStation
        } else if !settings.enabled {
            XchangeIdle::NotEnabled
        } else if !leading {
            XchangeIdle::AnotherStationIsLeading
        } else {
            XchangeIdle::NotEnabled
        }
    }

    async fn start_transport(&mut self) -> Result<(), String> {
        match self.settings.mode {
            XchangeMode::Tcp => {
                let tcp = tcp::start(
                    &self.settings.group,
                    self.station_uuid(),
                    &self.station_name(),
                    self.limits.max_file_bytes,
                    self.self_tx.clone(),
                    self.net.clone(),
                )
                .await?;
                self.transport = Some(Transport::Tcp(tcp));
            }
            XchangeMode::WebSocket => {
                if self.settings.url.trim().is_empty() {
                    return Err("no host to join: the show names no URL".into());
                }
                let link = ws::join(self.settings.url.trim(), self.self_tx.clone()).await?;
                link.tell(ws::Out::Json(self.join_message()));
                self.transport = Some(Transport::Joined(link));
            }
            XchangeMode::WebSocketHost => {
                self.registry.claim(self.self_tx.clone());
                self.transport = Some(Transport::Hosting);
            }
        }
        Ok(())
    }

    fn tear_down(&mut self) {
        match self.transport.take() {
            Some(Transport::Tcp(tcp)) => {
                // Everybody who might be listening is told, before the responder goes.
                self.tell_group(Message::Leave { from_station: self.station_uuid() });
                tcp.stop();
            }
            Some(Transport::Joined(link)) => {
                link.tell(ws::Out::Json(Message::Leave { from_station: self.station_uuid() }));
                link.stop();
            }
            Some(Transport::Hosting) => self.registry.release(),
            None => {}
        }
    }

    // ── Identity ──────────────────────────────────────────────────────────────

    fn station_uuid(&self) -> Uuid {
        self.show.as_ref().map(|(id, _)| station_uuid_for(*id)).unwrap_or_else(Uuid::nil)
    }

    /// The show's name, which is what the thing on the wire *is*.
    fn station_name(&self) -> String {
        self.show.as_ref().map(|(_, name)| name.clone()).unwrap_or_default()
    }

    fn join_message(&self) -> Message {
        Message::Join {
            provider: PROVIDER.to_string(),
            ver_major: MVR_VERSION.0,
            ver_minor: MVR_VERSION.1,
            station_uuid: self.station_uuid(),
            station_name: self.station_name(),
            commits: self.our_commits_on_the_wire(),
        }
    }

    /// What this station offers: its own commits, and — when hosting — the group's.
    ///
    /// The two are different claims and the difference is deliberate. Peer to peer,
    /// every station answers for itself. As a host we are the only path between
    /// clients, so a joiner learns the group's history from us or not at all.
    fn commits_to_offer(&self) -> Vec<Commit> {
        if matches!(self.transport, Some(Transport::Hosting)) {
            self.commits.values().map(|c| self.as_commit(c)).collect()
        } else {
            self.our_commits_on_the_wire()
        }
    }

    fn our_commits_on_the_wire(&self) -> Vec<Commit> {
        self.commits.values().filter(|c| c.ours && c.here).map(|c| self.as_commit(c)).collect()
    }

    fn as_commit(&self, commit: &XchangeCommit) -> Commit {
        Commit {
            ver_major: MVR_VERSION.0,
            ver_minor: MVR_VERSION.1,
            file_size: commit.file_size,
            file_uuid: commit.file_uuid,
            station_uuid: commit.station_uuid,
            for_stations: Vec::new(),
            comment: commit.comment.clone(),
            file_name: commit.file_name.clone(),
        }
    }

    fn load_our_commits(&mut self) {
        let us = self.station_uuid();
        let name = self.station_name();
        self.commits.retain(|_, c| !c.ours);
        for held in self.cache.list() {
            self.commits.insert(
                held.file_uuid,
                XchangeCommit {
                    file_uuid: held.file_uuid,
                    station_uuid: us,
                    station_name: name.clone(),
                    comment: held.comment,
                    file_name: held.file_name,
                    file_size: held.file_size,
                    ours: true,
                    here: true,
                    at_ms: held.at_ms,
                },
            );
        }
    }

    // ── Talking to the group ──────────────────────────────────────────────────

    /// Tell everybody, in whichever mode is running.
    ///
    /// TCP dials each station in turn on its own task, because a station that has gone
    /// away without withdrawing its mDNS record would otherwise hold up the rest.
    fn tell_group(&self, message: Message) {
        match &self.transport {
            Some(Transport::Tcp(_)) => {
                for known in self.stations.values().filter(|k| k.in_group && k.dialable) {
                    if let Endpoint::Tcp(addr) = known.endpoint {
                        let message = message.clone();
                        let limit = self.limits.max_file_bytes;
                        tokio::spawn(async move {
                            if let Err(e) = tcp::send(addr, &message, limit).await {
                                debug!("[xchange] {addr} did not take the message: {e}");
                            }
                        });
                    }
                }
            }
            Some(Transport::Joined(link)) => link.tell(ws::Out::Json(message)),
            Some(Transport::Hosting) => self.registry.tell_others(None, ws::Out::Json(message)),
            None => {}
        }
    }

    /// Answer, or push, down one connection.
    fn tell(&self, endpoint: &Endpoint, reply: Reply) {
        match endpoint {
            Endpoint::WsClient(id) => self.registry.tell(*id, reply.into()),
            Endpoint::WsHost => {
                if let Some(Transport::Joined(link)) = &self.transport {
                    link.tell(reply.into());
                }
            }
            // A TCP answer goes back on the connection that carried the question, and
            // that connection is held by `tcp::serve` rather than by this manager.
            // Nothing else can reach it, which is why relaying is a hosted-group act.
            Endpoint::Tcp(addr) => {
                debug!("[xchange] nothing to push to {addr} outside a connection");
            }
        }
    }

    // ── What arrives ──────────────────────────────────────────────────────────

    async fn handle_incoming(&mut self, message: Message, from: Endpoint) -> Option<Reply> {
        match message {
            Message::Join { station_uuid, station_name, provider, commits, .. } => {
                info!("[xchange] {station_name} ({station_uuid}) joined");
                self.remember(station_uuid, &station_name, &provider, from, true);
                for commit in commits {
                    self.note_commit(commit, station_uuid);
                }
                Some(Reply::Message(Message::JoinRet {
                    response: Response::ok(),
                    provider: PROVIDER.to_string(),
                    ver_major: MVR_VERSION.0,
                    ver_minor: MVR_VERSION.1,
                    station_uuid: self.station_uuid(),
                    station_name: self.station_name(),
                    commits: self.commits_to_offer(),
                }))
            }
            Message::JoinRet { station_uuid, station_name, provider, commits, .. } => {
                self.remember(station_uuid, &station_name, &provider, from, true);
                for commit in commits {
                    self.note_commit(commit, station_uuid);
                }
                None
            }
            Message::Leave { from_station } => {
                if let Some(known) = self.stations.get_mut(&from_station) {
                    known.joined = false;
                }
                Some(Reply::Message(Message::LeaveRet { response: Response::ok() }))
            }
            Message::Commit(commit) => {
                let owner = commit.station_uuid;
                let announced = commit.file_uuid;
                // A commit addressed to other stations is not ours to list. The sender
                // took the trouble to say who it was for.
                if !commit.is_for(self.station_uuid()) {
                    debug!("[xchange] commit {announced} is for other stations");
                    return Some(Reply::Message(Message::CommitRet { response: Response::ok() }));
                }
                info!("[xchange] commit {announced} from {owner}: {}", commit.comment);
                // As a host, we are the only path between clients: a commit nobody
                // relays is a commit nobody but us ever hears about.
                if matches!(self.transport, Some(Transport::Hosting)) {
                    let except = match from {
                        Endpoint::WsClient(id) => Some(id),
                        _ => None,
                    };
                    self.registry.tell_others(except, ws::Out::Json(Message::Commit(commit.clone())));
                }
                self.note_commit(commit, owner);
                Some(Reply::Message(Message::CommitRet { response: Response::ok() }))
            }
            Message::Request { file_uuid, .. } => self.answer_request(file_uuid, from),
            Message::NewSessionHost { service_name, service_url } => {
                self.note_pending_host(service_name, service_url, &from)
            }
            // Answers to things we sent. Nothing here needs an answer of its own, and
            // answering one is how two implementations talk to each other for ever.
            Message::CommitRet { response }
            | Message::LeaveRet { response }
            | Message::RequestRet { response }
            | Message::NewSessionHostRet { response } => {
                if !response.ok {
                    warn!("[xchange] refused: {}", response.message);
                    // A refused fetch has to release whatever is waiting for it, or
                    // the next apply is told a fetch is already in flight for ever.
                    self.fail_awaiting(&response.message).await;
                }
                None
            }
            Message::Unknown => None,
        }
    }

    /// Somebody wants a file.
    fn answer_request(&mut self, file_uuid: Option<Uuid>, from: Endpoint) -> Option<Reply> {
        // No uuid asks for our latest, which is what the specification says a bare
        // request means.
        let wanted = match file_uuid {
            Some(id) => id,
            None => match self.cache.latest() {
                Some(latest) => latest.file_uuid,
                None => {
                    return Some(Reply::Message(Message::RequestRet {
                        response: Response::failed("this station has committed nothing"),
                    }))
                }
            },
        };

        if let Some(bytes) = self.cache.get(wanted) {
            info!("[xchange] serving {wanted} ({} bytes)", bytes.len());
            return Some(Reply::File(bytes));
        }

        // Hosting: the requester cannot reach the owner except through us, so the
        // request is passed on and the bytes stream back. Nothing is kept — the cache
        // is what *this* station committed, and holding what passes through would make
        // a console into a file store for other people's rigs.
        let owner = self.commits.get(&wanted).map(|c| c.station_uuid);
        if matches!(self.transport, Some(Transport::Hosting)) {
            if let Some(endpoint) = owner.and_then(|o| self.stations.get(&o)).map(|k| k.endpoint.clone())
            {
                if self.awaiting.is_some() {
                    return Some(Reply::Message(Message::RequestRet {
                        response: Response::failed("this host is already passing a file through"),
                    }));
                }
                debug!("[xchange] relaying a request for {wanted}");
                self.awaiting = Some(Awaiting::Relay { to: from });
                self.tell(
                    &endpoint,
                    Reply::Message(Message::Request {
                        file_uuid: Some(wanted),
                        from_station: owner,
                    }),
                );
                return None;
            }
        }

        Some(Reply::Message(Message::RequestRet {
            response: Response::failed("the MVR is not available on this client"),
        }))
    }

    fn note_pending_host(
        &mut self,
        service_name: String,
        service_url: String,
        from: &Endpoint,
    ) -> Option<Reply> {
        let named = !service_name.trim().is_empty();
        let urled = !service_url.trim().is_empty();
        if named == urled {
            // The specification is explicit: exactly one, and both is an error.
            return Some(Reply::Message(Message::NewSessionHostRet {
                response: Response::failed("set exactly one of ServiceName and ServiceURL"),
            }));
        }

        let (from_station, from_name) = self
            .stations
            .iter()
            .find(|(_, known)| &known.endpoint == from)
            .map(|(id, known)| (*id, known.name.clone()))
            .unwrap_or((Uuid::nil(), String::new()));

        warn!("[xchange] {from_name} asks this group to move to {service_name}{service_url}");
        self.pending_host = Some(PendingHost {
            from_station,
            from_name,
            service_name: service_name.trim().to_string(),
            service_url: service_url.trim().to_string(),
        });

        // **Not** `OK: true`. In this protocol that answer means "I have moved", and
        // this console has not: an unauthenticated station on the LAN naming a host to
        // connect to is a redirect, and a person decides. Saying so is more use to the
        // asker than a lie either way.
        Some(Reply::Message(Message::NewSessionHostRet {
            response: Response::failed("waiting for an operator to accept the move"),
        }))
    }

    fn remember(
        &mut self,
        station_uuid: Uuid,
        name: &str,
        provider: &str,
        endpoint: Endpoint,
        joined: bool,
    ) {
        if station_uuid == self.station_uuid() {
            return;
        }
        // A WebSocket connection is its own address and stays open; a TCP one is the
        // ephemeral port somebody dialled out of and is gone the moment it closes.
        let dialable = !matches!(endpoint, Endpoint::Tcp(_));
        let entry = self.stations.entry(station_uuid).or_insert_with(|| Known {
            name: name.to_string(),
            provider: provider.to_string(),
            endpoint: endpoint.clone(),
            fullname: None,
            joined,
            in_group: true,
            dialable,
        });
        if !name.is_empty() {
            entry.name = name.to_string();
        }
        if !provider.is_empty() {
            entry.provider = provider.to_string();
        }
        // Never trade a listening address for the socket a message happened to arrive
        // on: mDNS is the only thing that ever says where a TCP station can be reached,
        // and overwriting it here left a commit being announced to a closed port.
        if dialable || !entry.dialable {
            entry.endpoint = endpoint;
            entry.dialable = dialable;
        }
        entry.joined = entry.joined || joined;
    }

    fn note_commit(&mut self, commit: Commit, owner: Uuid) {
        if !commit.is_for(self.station_uuid()) {
            return;
        }
        let station_name =
            self.stations.get(&owner).map(|k| k.name.clone()).unwrap_or_else(String::new);
        let ours = owner == self.station_uuid();
        // Never overwrite one of our own: what this station holds is what the cache
        // says it holds, and a copy of our own commit coming back round from a host
        // must not make us claim a file we have since pruned.
        if self.commits.get(&commit.file_uuid).is_some_and(|c| c.ours) {
            return;
        }
        self.commits.insert(
            commit.file_uuid,
            XchangeCommit {
                file_uuid: commit.file_uuid,
                station_uuid: owner,
                station_name,
                comment: commit.comment,
                file_name: commit.file_name,
                file_size: commit.file_size,
                ours,
                here: false,
                at_ms: chrono::Utc::now().timestamp_millis(),
            },
        );
    }

    async fn handle_discovered(&mut self, found: Discovered) {
        if found.station_uuid == self.station_uuid() {
            return;
        }
        let entry = self.stations.entry(found.station_uuid).or_insert_with(|| Known {
            name: found.station_name.clone(),
            provider: String::new(),
            endpoint: Endpoint::Tcp(found.addr),
            fullname: Some(found.fullname.clone()),
            joined: false,
            in_group: found.in_group,
            dialable: true,
        });
        entry.name = found.station_name.clone();
        // Discovery is the one thing that says where a TCP station listens.
        entry.endpoint = Endpoint::Tcp(found.addr);
        entry.dialable = true;
        entry.fullname = Some(found.fullname);
        entry.in_group |= found.in_group;

        // Only a station in our own group is worth joining, and joining is what makes
        // us known to it: mDNS says a client exists, `MVR_JOIN` says we are in its
        // group and hands over what we have.
        if found.in_group && !entry.joined {
            let message = self.join_message();
            let limit = self.limits.max_file_bytes;
            let tx = self.self_tx.clone();
            tokio::spawn(async move {
                match tcp::send(found.addr, &message, limit).await {
                    Ok(Some(Payload::Json(bytes))) => {
                        if let Ok(message) = Message::from_json(&bytes) {
                            let (reply, _ignored) = oneshot::channel();
                            let _ = tx
                                .send(XchangeCommand::Incoming {
                                    message,
                                    from: Endpoint::Tcp(found.addr),
                                    reply,
                                })
                                .await;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => debug!("[xchange] could not join {}: {e}", found.addr),
                }
            });
        }
        self.publish().await;
    }

    // ── Files ─────────────────────────────────────────────────────────────────

    async fn handle_file(&mut self, from: Endpoint, bytes: Vec<u8>) {
        match self.awaiting.take() {
            Some(Awaiting::Relay { to }) => {
                debug!("[xchange] passing {} bytes through", bytes.len());
                self.tell(&to, Reply::File(bytes));
            }
            Some(Awaiting::Ours { file_uuid, user_id, from: asked, reply }) => {
                if asked != from {
                    debug!("[xchange] a file arrived from {from:?}, not from {asked:?}");
                    // Put the wait back: the station that was asked may still answer.
                    self.awaiting =
                        Some(Awaiting::Ours { file_uuid, user_id, from: asked, reply });
                    return;
                }
                self.import(file_uuid, user_id, bytes, reply).await;
            }
            None => debug!("[xchange] a file arrived that nothing was waiting for"),
        }
    }

    async fn fail_awaiting(&mut self, why: &str) {
        match self.awaiting.take() {
            Some(Awaiting::Ours { reply: Some(reply), .. }) => {
                let _ = reply.send(Err(why.to_string()));
            }
            Some(Awaiting::Relay { to }) => self.tell(
                &to,
                Reply::Message(Message::RequestRet { response: Response::failed(why) }),
            ),
            _ => {}
        }
    }

    async fn import(
        &mut self,
        file_uuid: Uuid,
        user_id: Uuid,
        bytes: Vec<u8>,
        reply: Option<oneshot::Sender<Result<serde_json::Value, String>>>,
    ) {
        let outcome =
            super::mvr::read_rig(&self.engine, &self.assets, &bytes, user_id).await.map(|report| {
                info!(
                    "[xchange] applied {file_uuid}: {} created, {} updated, {} no longer mentioned",
                    report.created,
                    report.updated,
                    report.missing.len()
                );
                serde_json::json!({
                    "created": report.created,
                    "updated": report.updated,
                    "missing": report.missing,
                    "warnings": report.warnings,
                })
            });

        if let Err(why) = &outcome {
            // A relayed ask has nobody to answer, so the log is where it goes — and a
            // `warn` reaches every station's System Log, including the one whose
            // operator asked for this.
            warn!("[xchange] could not apply {file_uuid}: {why}");
        }
        match reply {
            Some(reply) => {
                let _ = reply.send(outcome);
            }
            None => {}
        }
        self.publish().await;
    }

    // ── What an operator asks for ─────────────────────────────────────────────

    async fn handle_ask(
        &mut self,
        ask: XchangeAsk,
        reply: Option<oneshot::Sender<Result<serde_json::Value, String>>>,
    ) {
        // A follower relays to the leader rather than acting: only one station is on
        // the wire, and it is not this one.
        if !self.is_running() {
            if let (false, Some(sync)) = (self.leading, self.sync.clone()) {
                sync.relay_xchange(ask).await;
                answer(reply, Ok(serde_json::json!({ "relayed": true })));
                return;
            }
            answer(reply, Err(self.not_running_because()));
            return;
        }

        match ask {
            XchangeAsk::Commit { comment, user_id } => self.commit(comment, user_id, reply).await,
            XchangeAsk::Apply { file_uuid, user_id } => self.apply(file_uuid, user_id, reply).await,
            XchangeAsk::FollowHost { follow } => self.follow_host(follow, reply).await,
        }
    }

    async fn commit(
        &mut self,
        comment: String,
        _user_id: Uuid,
        reply: Option<oneshot::Sender<Result<serde_json::Value, String>>>,
    ) {
        if comment.trim().is_empty() {
            answer(reply, Err("a commit needs a comment: an empty one is one nobody can tell from the last".into()));
            return;
        }

        // The whole rig, always. One meaning for a commit and nothing to explain — and
        // a commit whose scope varies is one a receiver can misread as a deletion.
        let bytes =
            match super::mvr::write_rig(&self.engine, &self.assets, &Default::default()).await {
                Ok(bytes) => bytes,
                Err(why) => {
                    answer(reply, Err(format!("could not write the rig: {why}")));
                    return;
                }
            };

        let file_uuid = Uuid::new_v4();
        let at_ms = chrono::Utc::now().timestamp_millis();
        let file_name = format!(
            "{}-{}.mvr",
            self.station_name().replace(['/', '\\'], "-"),
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );
        let held = CachedCommit {
            file_uuid,
            file_size: bytes.len() as u64,
            comment: comment.clone(),
            file_name: file_name.clone(),
            at_ms,
        };
        if let Err(e) = self.cache.put(&held, &bytes) {
            answer(reply, Err(format!("could not keep the commit: {e}")));
            return;
        }

        // Reloaded rather than inserted: what this station announces is what the cache
        // says it holds, and this commit may have pruned an older one out of it.
        self.load_our_commits();

        info!("[xchange] committed {file_uuid} ({} bytes): {comment}", bytes.len());
        self.tell_group(Message::Commit(Commit {
            ver_major: MVR_VERSION.0,
            ver_minor: MVR_VERSION.1,
            file_size: bytes.len() as u64,
            file_uuid,
            station_uuid: self.station_uuid(),
            for_stations: Vec::new(),
            comment,
            file_name: file_name.clone(),
        }));

        self.publish().await;
        answer(
            reply,
            Ok(serde_json::json!({
                "fileUuid": file_uuid,
                "fileName": file_name,
                "fileSize": bytes.len(),
            })),
        );
    }

    async fn apply(
        &mut self,
        file_uuid: Uuid,
        user_id: Uuid,
        reply: Option<oneshot::Sender<Result<serde_json::Value, String>>>,
    ) {
        let Some(commit) = self.commits.get(&file_uuid).cloned() else {
            answer(reply, Err("no commit by that id".into()));
            return;
        };

        // Already here — ours, or one applied before. No fetch at all.
        if let Some(bytes) = self.cache.get(file_uuid) {
            self.import(file_uuid, user_id, bytes, reply).await;
            return;
        }

        if self.awaiting.is_some() {
            answer(reply, Err("a file is already being fetched; the protocol carries no way to tell two answers apart".into()));
            return;
        }

        let Some(known) = self.stations.get(&commit.station_uuid).cloned() else {
            answer(reply, Err("the station that made that commit is no longer here".into()));
            return;
        };

        let request =
            Message::Request { file_uuid: Some(file_uuid), from_station: Some(commit.station_uuid) };

        match known.endpoint {
            // The answer comes back on the connection the question went out on, so the
            // whole exchange is one task and there is nothing to correlate.
            Endpoint::Tcp(addr) => {
                let limit = self.limits.max_file_bytes;
                let tx = self.self_tx.clone();
                self.awaiting =
                    Some(Awaiting::Ours { file_uuid, user_id, from: Endpoint::Tcp(addr), reply });
                tokio::spawn(async move {
                    match tcp::send(addr, &request, limit).await {
                        Ok(Some(Payload::File(bytes))) => {
                            let _ = tx
                                .send(XchangeCommand::FileArrived { from: Endpoint::Tcp(addr), bytes })
                                .await;
                        }
                        Ok(Some(Payload::Json(bytes))) => {
                            // A refusal, which the manager turns back into an answer
                            // for whoever is waiting.
                            if let Ok(message) = Message::from_json(&bytes) {
                                let (r, _ignored) = oneshot::channel();
                                let _ = tx
                                    .send(XchangeCommand::Incoming {
                                        message,
                                        from: Endpoint::Tcp(addr),
                                        reply: r,
                                    })
                                    .await;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let (r, _ignored) = oneshot::channel();
                            let _ = tx
                                .send(XchangeCommand::Incoming {
                                    message: Message::RequestRet {
                                        response: Response::failed(e),
                                    },
                                    from: Endpoint::Tcp(addr),
                                    reply: r,
                                })
                                .await;
                        }
                    }
                });
            }
            endpoint => {
                self.awaiting = Some(Awaiting::Ours {
                    file_uuid,
                    user_id,
                    from: endpoint.clone(),
                    reply,
                });
                self.tell(&endpoint, Reply::Message(request));
            }
        }
    }

    async fn follow_host(
        &mut self,
        follow: bool,
        reply: Option<oneshot::Sender<Result<serde_json::Value, String>>>,
    ) {
        let Some(pending) = self.pending_host.take() else {
            answer(reply, Err("nothing has asked this group to move".into()));
            return;
        };
        if !follow {
            info!("[xchange] declined the move to {}{}", pending.service_name, pending.service_url);
            self.publish().await;
            answer(reply, Ok(serde_json::json!({ "followed": false })));
            return;
        }

        // Following is a change to the *show's* settings, not a private state of this
        // manager — every station of the session has to agree about which group it is
        // in, and the next reconsider is what actually moves the connections.
        let (mode, group, url) = if pending.service_url.is_empty() {
            // `xxxx._mvrxchange._tcp.local.` — the group is the part in front.
            let group = pending
                .service_name
                .trim_end_matches(pult_mvr_xchange::SERVICE_TYPE)
                .trim_end_matches('.')
                .to_string();
            (XchangeMode::Tcp, group, String::new())
        } else {
            (XchangeMode::WebSocket, self.settings.group.clone(), pending.service_url.clone())
        };

        let next = XchangeSettings { enabled: true, group, mode, url };
        let path = vec![PathSegment::Key("show".into()), PathSegment::Key("mvr_xchange".into())];
        let value = match serde_json::to_value(&next) {
            Ok(value) => value,
            Err(e) => {
                answer(reply, Err(e.to_string()));
                return;
            }
        };
        if let Err(e) = self.engine.set(path, Lifecycle::Persisted, value).await {
            answer(reply, Err(format!("could not follow: {e}")));
            return;
        }
        info!("[xchange] following the group to {:?}", next);
        answer(reply, Ok(serde_json::json!({ "followed": true })));
    }

    fn is_running(&self) -> bool {
        self.transport.is_some()
    }

    fn not_running_because(&self) -> String {
        match &self.idle {
            Some(XchangeIdle::NoShow) => "no show is open".into(),
            Some(XchangeIdle::NotEnabled) => "MVR-xchange is switched off for this show".into(),
            Some(XchangeIdle::RefusedByStation) => {
                "this station's preferences do not allow MVR-xchange".into()
            }
            Some(XchangeIdle::AnotherStationIsLeading) => {
                "another station of this session holds the exchange".into()
            }
            Some(XchangeIdle::Failed(why)) => why.clone(),
            None => "the exchange is not running".into(),
        }
    }

    // ── Saying what is happening ──────────────────────────────────────────────

    async fn publish(&mut self) {
        self.published = true;
        let state = self.state();
        let Ok(value) = serde_json::to_value(&state) else { return };

        let path = vec![PathSegment::Key("xchange".into())];
        let _ = self.engine.set(path, Lifecycle::Local, value.clone()).await;

        // Only the station holding the connections has anything to say, which is what
        // keeps a follower from publishing its own empty state over the leader's.
        if state.running {
            if let Some(sync) = &self.sync {
                sync.publish_xchange(value).await;
            }
        }
    }

    fn state(&self) -> XchangeState {
        let mut stations: Vec<XchangeStation> = self
            .stations
            .iter()
            .map(|(id, known)| XchangeStation {
                station_uuid: *id,
                station_name: known.name.clone(),
                provider: known.provider.clone(),
                address: match &known.endpoint {
                    Endpoint::Tcp(addr) => addr.to_string(),
                    Endpoint::WsClient(id) => format!("websocket client {id}"),
                    Endpoint::WsHost => self.settings.url.clone(),
                },
                is_us: false,
                joined: known.joined,
            })
            .collect();

        // This console's own row, because a group includes us and a panel that hid us
        // would leave an operator unable to see what their station tells the room.
        if self.is_running() {
            stations.insert(
                0,
                XchangeStation {
                    station_uuid: self.station_uuid(),
                    station_name: self.station_name(),
                    provider: PROVIDER.to_string(),
                    address: self.address(),
                    is_us: true,
                    joined: true,
                },
            );
        }

        let mut commits: Vec<XchangeCommit> = self.commits.values().cloned().collect();
        // Newest first: a commit list is read from the top, and the one somebody wants
        // is almost always the one that just arrived.
        commits.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));

        XchangeState {
            running: self.is_running(),
            idle: self.idle.clone(),
            on_station: self.station_label.clone(),
            settings: self.settings.clone(),
            station_uuid: self.station_uuid(),
            station_name: self.station_name(),
            address: self.address(),
            stations,
            commits,
            pending_host: self.pending_host.clone(),
        }
    }

    fn address(&self) -> String {
        match &self.transport {
            Some(Transport::Tcp(tcp)) => format!("port {}", tcp.port),
            Some(Transport::Joined(link)) => link.url.clone(),
            Some(Transport::Hosting) => {
                format!("/mvrxchange ({} connected)", self.registry.client_count())
            }
            None => String::new(),
        }
    }

    // ── Reading the show ──────────────────────────────────────────────────────

    async fn read_show(&self) -> Option<(Uuid, String, XchangeSettings)> {
        let value = self.engine.get(vec![PathSegment::Key("show".into())]).await.ok()?;
        let id = value.get("id")?.as_str()?.parse().ok()?;
        let name = value.get("name")?.as_str().unwrap_or_default().to_string();
        let settings = value
            .get("mvr_xchange")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Some((id, name, settings))
    }

    async fn read_is_follower(&self) -> bool {
        self.engine
            .get(vec![PathSegment::Key("session".into())])
            .await
            .ok()
            .and_then(|v| v.get("is_follower").and_then(serde_json::Value::as_bool))
            .unwrap_or(false)
    }
}

fn answer(
    reply: Option<oneshot::Sender<Result<serde_json::Value, String>>>,
    outcome: Result<serde_json::Value, String>,
) {
    if let Some(reply) = reply {
        let _ = reply.send(outcome);
    } else if let Err(why) = outcome {
        // Nobody to tell, so the log is where it goes — and at `warn`, which crosses
        // the sync link to the station whose operator asked.
        warn!("[xchange] {why}");
    }
}

#[cfg(test)]
mod tests;
