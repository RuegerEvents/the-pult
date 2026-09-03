//! The console, as a library.
//!
//! [`start`] brings a whole station up — showfile, engine, peer sync, devices,
//! outputs and the HTTP server — and hands back a [`Running`] describing where it
//! landed. The `pult-backend` binary is a command line around it, and the desktop
//! app in `pult-gui` is a window around the same call, so there is one definition
//! of what starting a console means.

pub mod api;
pub mod config;
pub mod engine;
pub mod error;
pub mod handle;
pub mod infra;
pub mod logging;
pub mod model;
pub mod state;

use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{routing::get, Router};
use pult_schema::events::operation::NodeId;
use tokio::{sync::mpsc, task::JoinHandle};
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    api::ws::{ws_handler, SubscriptionRegistry},
    engine::{EngineCommand, EngineHandle, ShowEngine},
    infra::connectors::OutputManager,
    infra::devices::{spawn_mdns_browser, DeviceManager},
    infra::identity,
    infra::plugins::PluginManager,
    infra::session::SessionManager,
    infra::showfile::{self, bundle::Bundle},
    infra::stations::{prune_stale, StationReporter, REPORT_INTERVAL},
    infra::sync::SyncManager,
    state::AppState,
};

pub use crate::config::Config;

/// The version of the console, as the crate records it. Reported over
/// `/api/config` so a frontend can say what it is talking to.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A station that is up. Holding this is what keeps it running: dropping
/// `serve` aborts the HTTP server.
pub struct Running {
    /// Where HTTP and the WebSocket are listening, with the port the OS actually
    /// gave out rather than the one that was asked for.
    pub http_addr: SocketAddr,
    pub sync_addr: SocketAddr,
    pub node_id: NodeId,
    pub engine: EngineHandle,
    pub plugins: crate::infra::plugins::PluginsHandle,
    /// The link to this station's peers, for a caller that wants to join one
    /// without going through discovery — which is what the tests want.
    pub sync: crate::infra::sync::SyncHandle,
    /// This station's log, where it was given one.
    pub log: Option<crate::logging::LogHandle>,
    /// Which of this station's clients are watching which peer's log.
    pub log_watchers: crate::logging::Watchers,
    /// Who is watching what this station's outputs — or a peer's — are putting on
    /// the wire.
    pub viewers: crate::infra::connectors::Viewers,
    /// Where a view is pushed at the browsers, and where one from a peer arrives.
    pub updates: crate::engine::UpdateBroadcast,
    pub serve: JoinHandle<Result<()>>,
    /// Which show this station has open, if it has one.
    pub bundle: Option<Bundle>,
    /// The show acts, for a caller that wants to take one without a browser to take
    /// it from — which is what the tests want, the same way `sync` is here so a test
    /// can join a peer without going through discovery.
    pub shows: ShowsHandle,
    /// Where a show act arrives. A station cannot open a show — opening one is this
    /// station stopping and another starting in its place — so it says what it was
    /// asked for and lets [`Console`] do it.
    switch: mpsc::Receiver<ShowSwitch>,
    /// Everything `start` spawned besides `serve`, so stopping is a thing that can
    /// actually be done rather than a thing that is hoped for.
    tasks: Vec<JoinHandle<()>>,
    /// The two that are *asked* to stop rather than aborted, kept apart so shutting
    /// down can wait for exactly them: the engine, so its writer commits what it is
    /// holding, and the session layer, so its mDNS service comes off the network.
    /// Everything else is aborted, and waiting on tasks that never end would be a
    /// timeout on every switch rather than a wait for anything.
    asked_to_stop: Vec<JoinHandle<()>>,
    /// Both handles on the showfile. Closed rather than dropped, which is what
    /// checkpoints the WAL — a `-wal` left beside a bundle somebody is about to copy
    /// is a copy that is missing the last few writes.
    pools: Vec<Arc<sqlx::SqlitePool>>,
    /// This station's mDNS browse daemon, which runs on a thread of its own and so
    /// outlives every task unless it is told not to.
    devices_mdns: Option<mdns_sd::ServiceDaemon>,
    /// For telling the session layer to take its service off the network.
    session: crate::infra::session::SessionHandle,
}

/// What a show act asks the console to do next.
///
/// Every one of these is "stop this station and start another one", because a
/// station is built around one showfile from `start` down: the pools, the engine's
/// state, the asset store, the plugins the roster asked for. Swapping the show
/// underneath all that would be a second definition of what opening a show means,
/// and the two would drift.
#[derive(Debug, Clone)]
pub enum ShowSwitch {
    Open {
        path: std::path::PathBuf,
        /// A session to join once the new station is up, for the welcome screen's
        /// "join what is already on the network": a station has to have a show
        /// before it can be handed one.
        then_join: Option<Uuid>,
    },
    Close,
    SaveAs {
        path: std::path::PathBuf,
    },
}

/// What the show RPCs reach: somewhere to send an act, and what to say about the
/// show that is open.
#[derive(Clone)]
pub struct ShowsHandle {
    tx: mpsc::Sender<ShowSwitch>,
    pub bundle: Option<Bundle>,
    pub shows_dir: Option<std::path::PathBuf>,
}

impl ShowsHandle {
    /// A handle with no console behind it: a station started by [`start`] directly,
    /// which is what a test does. Asking it anything says so rather than pretending —
    /// the same shape `logging::detached` takes for a station with no log.
    pub fn detached() -> ShowsHandle {
        let (tx, _nobody) = mpsc::channel(1);
        ShowsHandle { tx, bundle: None, shows_dir: None }
    }

    /// Ask the console to do this next.
    ///
    /// Answering before it happens is the point: the act is this station stopping,
    /// so a caller that waited for it to finish would be waiting on its own socket
    /// being closed. The client sees a disconnect and reconnects, which it already
    /// does for every other reason a station goes away.
    pub async fn ask(&self, switch: ShowSwitch) -> Result<(), String> {
        self.tx
            .send(switch)
            .await
            .map_err(|_| "this console cannot open shows".to_string())
    }
}

impl Running {
    /// Stop everything this station started, and wait for the disk to settle.
    ///
    /// The order matters. The engine and the session layer are *asked* first, so the
    /// engine's writer commits what it is holding and the session takes its mDNS
    /// service off the network — a station that vanished while still advertising is
    /// one a browser goes on offering to join. Everything else is aborted, because
    /// nothing else is holding state that outlives the process. The pools are closed
    /// last, which checkpoints the WAL: a bundle about to be copied has to be whole.
    pub async fn shutdown(self) {
        // Asked rather than aborted: this is what flushes the writer's queue and
        // what takes the mDNS service off the network.
        let _ = self.engine.0.send(crate::engine::EngineCommand::Stop).await;
        let _ = self.session.0.send(crate::infra::session::SessionCommand::Stop).await;
        let mut asked = self.asked_to_stop;
        // Bounded, because a station that will not stop must not keep an operator
        // from their next show.
        let _ = tokio::time::timeout(STOPPING_WITHIN, async {
            for task in &mut asked {
                let _ = task.await;
            }
        })
        .await;

        if let Some(mdns) = &self.devices_mdns {
            let _ = mdns.shutdown();
        }
        self.serve.abort();
        for task in self.tasks.iter().chain(asked.iter()) {
            task.abort();
        }
        // `is_finished` guards the two above, which may already have stopped on their
        // own and been awaited: a `JoinHandle` polled twice panics.
        // **Awaited**, not merely aborted. `abort` is a request that lands at the
        // task's next suspension point, and until it does, the listener is still
        // bound — so the station starting in this one's place would try to bind a
        // port that has not been given back yet, fail, and come up with no show at
        // all. That is a real bug this line is the fix for, and it looked like a
        // console that simply never came back.
        let _ = self.serve.await;
        for task in self.tasks.into_iter().chain(asked) {
            if !task.is_finished() {
                let _ = task.await;
            }
        }

        for pool in &self.pools {
            pool.close().await;
        }
    }
}

/// How long a station gets to stop tidily before it is stopped untidily.
const STOPPING_WITHIN: std::time::Duration = std::time::Duration::from_secs(2);

/// A console, which outlives the stations it runs.
///
/// The distinction this draws is the whole of what makes opening a show possible.
/// [`start`] brings up a *station*: one showfile, one engine, one set of sockets.
/// A `Console` is the process around it — it holds the configuration, keeps the port
/// across a switch so a tablet does not have to be told a new address, and starts
/// the next station when the last one is asked to make way.
///
/// Both binaries run one. Tests call [`start`] directly, because a test wants a
/// station and not a process.
pub struct Console {
    config: Config,
    running: Option<Running>,
}

impl Console {
    pub async fn start(config: Config) -> Result<Console> {
        let running = start(config.clone()).await?;
        remember_this_show(&running);
        Ok(Console { config, running: Some(running) })
    }

    /// The show acts, for a caller that has no browser to take one from — a test,
    /// or a binary that wants to open a show without one.
    pub fn shows(&self) -> ShowsHandle {
        self.running
            .as_ref()
            .map(|running| running.shows.clone())
            .unwrap_or_else(ShowsHandle::detached)
    }

    pub fn http_addr(&self) -> SocketAddr {
        self.running.as_ref().map(|r| r.http_addr).unwrap_or_else(|| {
            SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0))
        })
    }

    /// Run until the HTTP server stops, opening whatever shows it is asked for on
    /// the way.
    pub async fn serve(mut self) -> Result<()> {
        loop {
            let asked = {
                let Some(running) = self.running.as_mut() else { return Ok(()) };
                tokio::select! {
                    served = &mut running.serve => return served?,
                    asked = running.switch.recv() => match asked {
                        Some(asked) => asked,
                        // Nothing can ask any more, which means the station is gone.
                        None => return Ok(()),
                    },
                }
            };
            self.switch_to(asked).await?;
        }
    }

    async fn switch_to(&mut self, asked: ShowSwitch) -> Result<()> {
        let Some(running) = self.running.take() else { return Ok(()) };
        let was = self.config.show.clone();

        // The ports the OS actually gave out, pinned before the listener goes: a
        // console started with `--port 0` must not move to a different port every
        // time somebody opens a show, because the address is what an operator typed
        // into the tablet at the back of the room.
        self.config.port = running.http_addr.port();
        self.config.sync_port = running.sync_addr.port();
        let bundle = running.bundle.clone();
        running.shutdown().await;

        let mut then_join = None;
        match asked {
            ShowSwitch::Open { path, then_join: join } => {
                self.config.show = Some(path);
                then_join = join;
            }
            ShowSwitch::Close => self.config.show = None,
            ShowSwitch::SaveAs { path } => match bundle.as_ref() {
                Some(bundle) => match bundle.copy_to(&path) {
                    Ok(copy) => {
                        // A copy is a new show to the network. Two bundles carrying
                        // one id would find each other over mDNS and merge.
                        if let Err(e) = showfile::bundle::becomes_its_own_show(&copy).await {
                            warn!("[shows] the copy kept the original's id: {e:#}");
                        }
                        self.config.show = Some(copy.path().to_path_buf());
                    }
                    Err(e) => warn!("[shows] could not save a copy: {e:#}"),
                },
                None => warn!("[shows] there is no show to save a copy of"),
            },
        }

        let running = match start(self.config.clone()).await {
            Ok(running) => running,
            Err(e) => {
                // The show that was asked for will not open. Falling back to the one
                // that was open would be worse than it sounds — it was closed, and
                // whatever is wrong with the new one is wrong with the operator's
                // *intent* — so the console comes up with no show and says so, which
                // is a state the welcome screen already knows how to be.
                warn!("[shows] {e:#}");
                self.config.show = None;
                let recovered = start(self.config.clone()).await;
                match recovered {
                    Ok(running) => running,
                    Err(e) => {
                        // Twice is not a show problem. Put the old path back so
                        // whoever reads the config sees what was being attempted.
                        self.config.show = was;
                        return Err(e);
                    }
                }
            }
        };
        remember_this_show(&running);
        if let Some(session_id) = then_join {
            if let Err(e) = running.session.join_session(session_id).await {
                warn!("[shows] could not join that session: {e}");
            }
        }
        self.running = Some(running);
        Ok(())
    }
}

/// Put this show at the top of the recently opened list.
fn remember_this_show(running: &Running) {
    if let Some(bundle) = &running.bundle {
        showfile::recent::remember(bundle.path());
    }
}

/// Bring a console up.
///
/// The listener is bound before anything advertises an address, because a
/// `port: 0` station has to know its own port before it can tell its peers where
/// to fetch assets from.
pub async fn start(config: Config) -> Result<Running> {
    // A console with no show open is a real state, and the one it comes up in when
    // nobody named a show: everything below runs, against a database that is never
    // written anywhere. See `Config::show`.
    let bundle = match &config.show {
        Some(path) => Some(Bundle::open_or_create(path)?),
        None => None,
    };
    let pool = Arc::new(match &bundle {
        Some(bundle) => showfile::open(&bundle.db_path()).await?,
        None => showfile::open_in_memory().await?,
    });
    // Opened after `open` has migrated the file, and never used for reading.
    let write_pool = match &bundle {
        None => None,
        Some(bundle) => match showfile::open_for_writing(&bundle.db_path()).await {
            Ok(second) => Some(Arc::new(second)),
            Err(e) => {
                // Not fatal: the writer falls back to sharing the read pool, which is
                // how it behaves for an in-memory show anyway. A console that cannot
                // open a second handle to its own showfile should still open the show.
                tracing::warn!("[start] could not open a second showfile handle: {e}");
                None
            }
        },
    };
    // Both handles, so shutdown can close them and checkpoint the WAL.
    let pools: Vec<Arc<sqlx::SqlitePool>> =
        std::iter::once(pool.clone()).chain(write_pool.clone()).collect();
    // The bytes, which are files in the bundle, and the rows that describe them. A
    // console with no show open has nowhere to put any, and this is what says so.
    let assets = crate::infra::assets::AssetStore::new(
        bundle.as_ref().map(|bundle| bundle.assets_dir()),
        pool.clone(),
    );
    // With the machine rather than with the show: a folder is a thing somebody
    // copies, and an id that travelled with it would make the second machine to open
    // the show a second claimant of the first one's outputs.
    let node_id = config
        .node_id
        .map(NodeId)
        .unwrap_or_else(|| identity::load_or_create(config.identity.as_deref()));

    // Stamped on the log as early as there is an answer, because everything from
    // here down is worth attributing — a port that will not bind and a showfile
    // that complains both happen in the next few lines, and a line carrying the nil
    // uuid is a line no peer's panel can place.
    if let Some(log) = &config.log {
        log.set_node_id(node_id.0);
    }

    // Bound first: `--port 0` is a real case for a second console on one machine,
    // and the station row published below has to carry the port that was given
    // out rather than the zero that was asked for.
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(config.bind, config.port)).await?;
    let http_addr = listener.local_addr()?;

    // Every task this station starts, so it can be stopped again. Opening a show is
    // this station stopping and another one taking its place, and a `tokio::spawn`
    // nobody kept a handle on would outlive both.
    let mut tasks: Vec<JoinHandle<()>> = Vec::new();
    // Where a show act goes. Small: these arrive one at a time from a person, and a
    // queue of them would be a queue of consoles to become.
    let (switch_tx, switch_rx) = mpsc::channel::<ShowSwitch>(4);
    let shows = ShowsHandle {
        tx: switch_tx,
        bundle: bundle.clone(),
        shows_dir: shows_dir(&config),
    };

    let log_watchers = crate::logging::Watchers::default();
    // Who is looking at what leaves this station. Owned here rather than by the
    // output manager, because both ends of the question reach it: a browser asks
    // through an RPC, and a peer asks down a sync link, and the connector cannot
    // tell them apart.
    let viewers = crate::infra::connectors::Viewers::default();

    let (engine_tx, engine_rx) = mpsc::channel::<EngineCommand>(256);
    // A queue per source class in front of that channel, so a plugin in a write loop
    // or a peer replaying twenty minutes of oplog cannot crowd out the operator whose
    // fader is waiting. See `engine::admission`.
    let admission = crate::engine::admission::start(engine_tx);
    use crate::engine::admission::Source;

    // The console's own machinery: playback, flows, devices, the reporter, the
    // pruner. Frequent, cheap, and nobody is watching its latency.
    let engine_handle = EngineHandle::for_source(&admission, Source::Station);
    // A person at a desk or a tablet. The one class whose latency is felt.
    let operator_handle = EngineHandle::for_source(&admission, Source::Operator);
    // Another station replaying what this one missed: bulk, and latency-insensitive
    // by nature, since it is replaying the past.
    let peer_handle = EngineHandle::for_source(&admission, Source::Peer);
    // A guest. The class most likely to be a runaway loop, and the one whose flood
    // must land on itself.
    let plugin_handle_engine = EngineHandle::for_source(&admission, Source::Plugin);

    // What the browsers on this station say they are costing themselves. LOCAL, and
    // owned here rather than by the engine: a page's row is written by a report and
    // removed by a disconnect, neither of which is a change to the show.
    let clients = crate::infra::clients::ClientRegistry::new(engine_handle.clone());

    let (mut sync_mgr, sync_handle, sync_addr) =
        SyncManager::bind(node_id, config.sync_port, peer_handle.clone(), config.log.clone())
            .await?;
    info!("peer sync on {sync_addr}");

    let (mut engine, broadcast) =
        ShowEngine::new_with_write_pool(
            node_id,
            engine_rx,
            pool.clone(),
            // Its own connection to the same file. WAL lets it hold a group commit
            // open without a peer's catch-up read queueing behind it.
            write_pool,
            Some(sync_handle.clone()),
        );
    // Now that the broadcast exists, a peer link can carry a view both ways: an ask
    // arriving, and a drawn view going back to whoever asked.
    sync_mgr.watching_outputs(viewers.clone(), broadcast.clone());

    // Every node browses for OpenHaunt devices; only the one leading the session
    // adopts or commands any of them.
    let (device_mgr, device_handle, device_directory) =
        DeviceManager::new(node_id, engine_handle.clone(), config.openhaunt_broker_port);
    tasks.push(tokio::spawn(device_mgr.run()));
    let devices_mdns = spawn_mdns_browser(device_handle.clone());

    // Which outputs exist is show data now. The manager reconciles against the
    // `outputs` collection, and the engine hands it that collection whenever it
    // changes — including once at load, so a saved show comes up sending.
    let (output_mgr, output, frame_costs) = OutputManager::new(
        node_id,
        engine_handle.clone(),
        Some((device_directory, device_handle.clone())),
    );
    let output_mgr = output_mgr.watchable(viewers.clone(), broadcast.clone());
    tasks.push(tokio::spawn(output_mgr.run()));
    engine.set_output(output);

    // The bundle knows what a show with no row yet should be called, and nothing
    // else does. Told before the load, because the load is what seeds the row.
    if let Some(bundle) = &bundle {
        engine.set_seed_name(bundle.seed_name());
    }
    engine_handle.0.send(EngineCommand::LoadFromShowfile).await?;
    let engine_task = tokio::spawn(engine.run());

    // The flags survive as a way to seed an empty showfile. Anything already
    // configured wins: a flag should not quietly add a second output every start.
    seed_outputs_from_flags(&engine_handle, node_id, &config).await;

    // Every station publishes one row about itself, every couple of seconds, and
    // the latencies it has measured to the peers it is connected to.
    // A peer reaching this station for an asset needs the same host it syncs to,
    // on the HTTP port rather than the sync one.
    let peer_http_addr = format!("{}:{}", sync_addr.ip(), http_addr.port());
    let reporter = StationReporter::new(
        node_id,
        engine_handle.clone(),
        sync_addr,
        peer_http_addr,
        sync_mgr.peer_links(),
        frame_costs,
        // The disk figure is about the volume the show is written to, not the root
        // one: a show that cannot be saved is what it exists to see coming. A console
        // with no show open reports on the volume its shows would go to.
        bundle
            .as_ref()
            .map(|bundle| bundle.path().to_path_buf())
            .or_else(|| shows_dir(&config))
            .unwrap_or_else(std::env::temp_dir),
    );
    tasks.push(tokio::spawn(reporter.run()));

    // Only the leader prunes: two nodes deleting each other's rows on different
    // schedules is a fight rather than a cleanup.
    let pruner = engine_handle.clone();
    let pruner_sync = sync_handle.clone();
    tasks.push(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(REPORT_INTERVAL * 5);
        loop {
            ticker.tick().await;
            if pruner_sync.leader().await == Some(node_id) {
                prune_stale(&pruner, chrono::Duration::seconds(30)).await;
            }
        }
    }));

    // And the browsers' own rows, swept for the ones that went quiet without hanging
    // up. Not the leader's job and it could not be: a browser is on one station's
    // socket and no other station has ever heard of it.
    tasks.push(tokio::spawn(crate::infra::clients::sweep(clients.clone(), REPORT_INTERVAL * 5)));

    let (session_mgr, session_handle) = SessionManager::new(
        node_id,
        config.sync_port,
        engine_handle.clone(),
        sync_handle.clone(),
    );
    // If the leader disappears and this node wins the election, the session layer
    // has to start advertising so newcomers find the show here.
    sync_mgr.on_promotion(session_mgr.promotion_sender());
    tasks.push(tokio::spawn(sync_mgr.run()));
    let session_task = tokio::spawn(session_mgr.run());

    // Plugins come up last of the managers: they see a station that already
    // plays back and syncs, which is also the state a hot reload lands in.
    let (plugin_mgr, plugin_handle) = PluginManager::new(
        plugin_handle_engine.clone(),
        broadcast.clone(),
        crate::api::rpcs::LocalRpcDeps {
            session: session_handle.clone(),
            devices: device_handle.clone(),
            engine: operator_handle.clone(),
            log: config.log.clone(),
            log_watchers: log_watchers.clone(),
            sync: Some(sync_handle.clone()),
            // A plugin has no browser behind it, so it cannot watch a peer's log
            // "while it is looking" — there is nothing to stop looking. Which is
            // also why carrying the client registry here costs nothing: reporting
            // needs a caller, and this one never has any.
            caller: None,
            clients: Some(clients.clone()),
            node_id,
            // A plugin can ask what a wire is carrying the same way a browser can —
            // but not "while it is looking", having nothing to stop looking with,
            // which is what the missing `caller` above already says.
            viewers: viewers.clone(),
            // A plugin has no socket either, so there is nothing to count for one.
            ws_registry: None,
            shows: shows.clone(),
        },
        config.plugin_dirs.clone(),
        // Where a carried bundle's bytes live.
        Some(assets.clone()),
        config.plugin_data.clone(),
        node_id,
    );
    tasks.push(tokio::spawn(plugin_mgr.run()));
    let plugin_handle_for_running = plugin_handle.clone();
    let sync_handle_for_running = sync_handle.clone();
    let session_handle_for_running = session_handle.clone();

    let state = AppState {
        engine: operator_handle.clone(),
        assets: assets.clone(),
        sync: sync_handle,
        session: session_handle,
        devices: device_handle,
        plugins: plugin_handle,
        // Built even where nobody has a login: it costs a client with an empty cookie
        // jar, and asking it anything without one answers "set a login" rather than
        // going anywhere.
        share: crate::infra::interop::share::ShareHandle::new(),
        node_id,
        ws_registry: SubscriptionRegistry::default(),
        broadcast: broadcast.clone(),
        log_watchers: log_watchers.clone(),
        viewers: viewers.clone(),
        clients: clients.clone(),
        config: config.clone(),
        shows: shows.clone(),
        http_port: http_addr.port(),
    };

    // The log to the browsers: gathered for a moment, then pushed as one `Update`.
    //
    // Straight onto the update broadcast rather than through the engine, because a
    // log line is not show state — putting it through the actor would queue
    // diagnostics behind whatever the console is busy with, which is exactly when
    // somebody is reading them. It rides the existing `Update` message, so no
    // protocol shape was added and a browser subscribes to `logs` the way it
    // subscribes to `devices`.
    if let Some(log) = config.log.clone() {
        let broadcast = broadcast.clone();
        tasks.push(tokio::spawn(async move {
            let mut lines = log.subscribe();
            let mut tick =
                tokio::time::interval(std::time::Duration::from_millis(logging::COALESCE_MS));
            let mut pending: Vec<pult_schema::ws::LogLine> = Vec::new();
            loop {
                tokio::select! {
                    line = lines.recv() => match line {
                        Ok(line) => pending.push(line),
                        // A reader that fell behind says so by the jump in `seq`,
                        // which the panel shows as a count of what it missed.
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    _ = tick.tick(), if !pending.is_empty() => {
                        let batch = std::mem::take(&mut pending);
                        let path = vec![pult_schema::path::PathSegment::Key("logs".into())];
                        if let Ok(value) = serde_json::to_value(batch) {
                            // Nobody subscribed is nobody sent to: a browser with the
                            // panel closed costs nothing, which is what keeps task
                            // 44's "no updates during a fade" true.
                            let _ = broadcast.0.send((path, value));
                        }
                    }
                }
            }
        }));
    }

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .merge(crate::api::rest::routes())
        .merge(crate::api::rest::config_routes())
        .merge(crate::infra::plugins::asset_routes())
        // The console itself, last, so `/ws` and `/assets` are matched first and
        // every other path falls through to the single-page app.
        .fallback(crate::api::spa::handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    if !crate::api::spa::is_built() {
        warn!("[http] no frontend in this binary; run `npm --prefix frontend run build`");
    }
    info!("pult-backend listening on {http_addr}");
    let serve = tokio::spawn(async move {
        axum::serve(listener, app).await?;
        Ok(())
    });

    Ok(Running {
        http_addr,
        sync_addr,
        node_id,
        engine: engine_handle,
        plugins: plugin_handle_for_running,
        sync: sync_handle_for_running,
        log: config.log.clone(),
        log_watchers,
        viewers,
        updates: broadcast,
        serve,
        bundle,
        shows,
        switch: switch_rx,
        tasks,
        asked_to_stop: vec![engine_task, session_task],
        pools,
        devices_mdns,
        session: session_handle_for_running,
    })
}

/// Where this console keeps the shows nobody gave it a path for.
///
/// Three layers, most specific winning: what the caller was told, the station's own
/// preference, then the platform's data directory. The directory is made if it is not
/// there, and a console that cannot make it simply has nowhere to list — which the
/// welcome screen says, rather than refusing to come up.
pub fn shows_dir(config: &Config) -> Option<std::path::PathBuf> {
    let named = config
        .shows_dir
        .clone()
        .or_else(|| infra::preferences::load().shows_dir)
        .or_else(showfile::bundle::default_shows_dir)?;
    showfile::bundle::ensure_dir(&named)
}

/// Turn the `--artnet` / `--sacn` seeds into `outputs` entries, but only on a show
/// that has none. Once outputs are show data, a flag is a convenience for the
/// first run and a bug on every run after it.
async fn seed_outputs_from_flags(engine: &EngineHandle, node_id: NodeId, config: &Config) {
    use pult_schema::{
        lifecycle::Lifecycle,
        path::PathSegment,
        types::output::{OutputConfig, OutputKind},
    };

    if config.artnet.is_empty() && config.sacn.is_none() {
        return;
    }
    let existing = engine
        .get(vec![PathSegment::Key("outputs".into())])
        .await
        .ok()
        .and_then(|v| v.as_array().map(|a| a.len()))
        .unwrap_or(0);
    if existing > 0 {
        warn!("[output] this show already has outputs; ignoring the command line");
        return;
    }

    let mut seeds: Vec<OutputConfig> = Vec::new();
    for target in &config.artnet {
        seeds.push(OutputConfig {
            id: Uuid::new_v4(),
            name: format!("Art-Net {target}"),
            kind: OutputKind::Artnet,
            target: Some(target.to_string()),
            universes: vec![],
            enabled: true,
            node_id: Some(node_id),
        });
    }
    if let Some(target) = config.sacn {
        seeds.push(OutputConfig {
            id: Uuid::new_v4(),
            name: match target {
                Some(addr) => format!("sACN {addr}"),
                None => "sACN".to_string(),
            },
            kind: OutputKind::Sacn,
            target: target.map(|addr| addr.to_string()),
            universes: vec![],
            enabled: true,
            node_id: Some(node_id),
        });
    }

    for seed in seeds {
        info!("[output] seeding {} from the command line", seed.name);
        let path = vec![
            PathSegment::Key("outputs".into()),
            PathSegment::Key("__create".into()),
        ];
        let value = serde_json::to_value(&seed).unwrap_or_default();
        if let Err(e) = engine.set(path, Lifecycle::Persisted, value).await {
            warn!("[output] could not seed {}: {e}", seed.name);
        }
    }
}
