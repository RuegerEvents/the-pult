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
    infra::showfile,
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
}

/// Bring a console up.
///
/// The listener is bound before anything advertises an address, because a
/// `port: 0` station has to know its own port before it can tell its peers where
/// to fetch assets from.
pub async fn start(config: Config) -> Result<Running> {
    let pool = Arc::new(showfile::open(&config.showfile).await?);
    // Opened after `open` has migrated the file, and never used for reading.
    let write_pool = match showfile::open_for_writing(&config.showfile).await {
        Ok(second) => Some(Arc::new(second)),
        Err(e) => {
            // Not fatal: the writer falls back to sharing the read pool, which is how
            // it behaves for an in-memory show anyway. A console that cannot open a
            // second handle to its own showfile should still open the show.
            tracing::warn!("[start] could not open a second showfile handle: {e}");
            None
        }
    };
    // Recorded beside the showfile, so an output that names this station still
    // belongs to it tomorrow.
    let node_id = config
        .node_id
        .map(NodeId)
        .unwrap_or_else(|| identity::load_or_create(&config.showfile));

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
    tokio::spawn(device_mgr.run());
    spawn_mdns_browser(device_handle.clone());

    // Which outputs exist is show data now. The manager reconciles against the
    // `outputs` collection, and the engine hands it that collection whenever it
    // changes — including once at load, so a saved show comes up sending.
    let (output_mgr, output, frame_costs) = OutputManager::new(
        node_id,
        engine_handle.clone(),
        Some((device_directory, device_handle.clone())),
    );
    let output_mgr = output_mgr.watchable(viewers.clone(), broadcast.clone());
    tokio::spawn(output_mgr.run());
    engine.set_output(output);

    engine_handle.0.send(EngineCommand::LoadFromShowfile).await?;
    tokio::spawn(engine.run());

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
        // one: a show that cannot be saved is what it exists to see coming.
        std::path::PathBuf::from(&config.showfile),
    );
    tokio::spawn(reporter.run());

    // Only the leader prunes: two nodes deleting each other's rows on different
    // schedules is a fight rather than a cleanup.
    let pruner = engine_handle.clone();
    let pruner_sync = sync_handle.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(REPORT_INTERVAL * 5);
        loop {
            ticker.tick().await;
            if pruner_sync.leader().await == Some(node_id) {
                prune_stale(&pruner, chrono::Duration::seconds(30)).await;
            }
        }
    });

    // And the browsers' own rows, swept for the ones that went quiet without hanging
    // up. Not the leader's job and it could not be: a browser is on one station's
    // socket and no other station has ever heard of it.
    tokio::spawn(crate::infra::clients::sweep(clients.clone(), REPORT_INTERVAL * 5));

    let (session_mgr, session_handle) = SessionManager::new(
        node_id,
        config.sync_port,
        engine_handle.clone(),
        sync_handle.clone(),
    );
    // If the leader disappears and this node wins the election, the session layer
    // has to start advertising so newcomers find the show here.
    sync_mgr.on_promotion(session_mgr.promotion_sender());
    tokio::spawn(sync_mgr.run());
    tokio::spawn(session_mgr.run());

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
        },
        config.plugin_dirs.clone(),
        // The asset store a carried bundle lives in.
        Some(pool.clone()),
        config.plugin_data.clone(),
        node_id,
    );
    tokio::spawn(plugin_mgr.run());
    let plugin_handle_for_running = plugin_handle.clone();
    let sync_handle_for_running = sync_handle.clone();

    let state = AppState {
        engine: operator_handle.clone(),
        pool,
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
        tokio::spawn(async move {
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
        });
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
    })
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
