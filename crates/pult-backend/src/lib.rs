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
    pub serve: JoinHandle<Result<()>>,
}

/// Bring a console up.
///
/// The listener is bound before anything advertises an address, because a
/// `port: 0` station has to know its own port before it can tell its peers where
/// to fetch assets from.
pub async fn start(config: Config) -> Result<Running> {
    let pool = Arc::new(showfile::open(&config.showfile).await?);
    // Recorded beside the showfile, so an output that names this station still
    // belongs to it tomorrow.
    let node_id = config
        .node_id
        .map(NodeId)
        .unwrap_or_else(|| identity::load_or_create(&config.showfile));

    // Bound first: `--port 0` is a real case for a second console on one machine,
    // and the station row published below has to carry the port that was given
    // out rather than the zero that was asked for.
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(config.bind, config.port)).await?;
    let http_addr = listener.local_addr()?;

    let (engine_tx, engine_rx) = mpsc::channel::<EngineCommand>(256);
    let engine_handle = EngineHandle(engine_tx);

    let (mut sync_mgr, sync_handle, sync_addr) =
        SyncManager::bind(node_id, config.sync_port, engine_handle.clone()).await?;
    info!("peer sync on {sync_addr}");

    let (mut engine, broadcast) =
        ShowEngine::new_with_rx(node_id, engine_rx, pool.clone(), Some(sync_handle.clone()));

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
        engine_handle.clone(),
        broadcast.clone(),
        crate::api::rpcs::LocalRpcDeps {
            session: session_handle.clone(),
            devices: device_handle.clone(),
            engine: engine_handle.clone(),
        },
        config.plugin_dirs.clone(),
        // The asset store a carried bundle lives in.
        Some(pool.clone()),
        config.plugin_data.clone(),
        node_id,
    );
    tokio::spawn(plugin_mgr.run());
    let plugin_handle_for_running = plugin_handle.clone();

    let state = AppState {
        engine: engine_handle.clone(),
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
        config: config.clone(),
        http_port: http_addr.port(),
    };

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
