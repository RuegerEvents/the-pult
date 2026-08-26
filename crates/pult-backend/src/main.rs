mod engine;
mod infra;
mod api;
mod model;
mod config;
mod state;
mod handle;
mod error;

use std::sync::Arc;

use anyhow::Result;
use axum::{routing::get, Router};
use clap::Parser;
use pult_schema::events::operation::NodeId;
use uuid::Uuid;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::{
    api::ws::{ws_handler, SubscriptionRegistry},
    config::Config,
    engine::{EngineCommand, EngineHandle, ShowEngine},
    infra::connectors::{artnet::ARTNET_PORT, sacn::SACN_PORT, OutputManager},
    infra::devices::{spawn_mdns_browser, DeviceManager},
    infra::session::SessionManager,
    infra::showfile,
    infra::sync::SyncManager,
    state::AppState,
};

#[derive(Parser)]
#[command(about = "pult-backend lighting console server")]
struct Args {
    #[arg(long, default_value_t = 7700)]
    port: u16,
    #[arg(long, default_value_t = 7701)]
    sync_port: u16,
    #[arg(long, default_value = "show.db")]
    showfile: String,
    /// Send Art-Net to this address, e.g. 10.0.0.5 or 255.255.255.255:6454.
    /// The port defaults to 6454. Repeat the flag to feed several nodes. Off unless
    /// given: a console should not put packets on someone's network because it
    /// happened to start up.
    #[arg(long, value_name = "ADDR", value_parser = parse_artnet_target)]
    artnet: Vec<std::net::SocketAddr>,
    /// Send sACN. E1.31 has a multicast group per universe, so no address is needed
    /// and only receivers that joined a universe see it. Give an address to unicast
    /// to a receiver that cannot be reached by multicast.
    #[arg(long, value_name = "ADDR", num_args = 0..=1, default_missing_value = "multicast", value_parser = parse_sacn_target)]
    sacn: Option<Option<std::net::SocketAddr>>,
    /// Port for the MQTT broker this node runs for its OpenHaunt devices. Started
    /// only when this node is the one driving them.
    #[arg(long, default_value_t = 1883)]
    openhaunt_broker_port: u16,
}

/// Accept either `host:port` or a bare address, defaulting to the Art-Net port.
fn parse_artnet_target(value: &str) -> Result<std::net::SocketAddr, String> {
    parse_target(value, ARTNET_PORT)
}

/// `--sacn` on its own means multicast; `--sacn <addr>` unicasts there.
fn parse_sacn_target(value: &str) -> Result<Option<std::net::SocketAddr>, String> {
    if value == "multicast" {
        return Ok(None);
    }
    parse_target(value, SACN_PORT).map(Some)
}

fn parse_target(value: &str, default_port: u16) -> Result<std::net::SocketAddr, String> {
    if let Ok(addr) = value.parse::<std::net::SocketAddr>() {
        return Ok(addr);
    }
    value
        .parse::<std::net::IpAddr>()
        .map(|ip| std::net::SocketAddr::new(ip, default_port))
        .map_err(|e| format!("not an address: {e}"))
}

/// Turn `--artnet` / `--sacn` into `outputs` entries, but only on a show that has
/// none. Once outputs are show data, a flag is a convenience for the first run and
/// a bug on every run after it.
async fn seed_outputs_from_flags(engine: &EngineHandle, node_id: NodeId, args: &Args) {
    use pult_schema::{
        lifecycle::Lifecycle,
        path::PathSegment,
        types::output::{OutputConfig, OutputKind},
    };

    if args.artnet.is_empty() && args.sacn.is_none() {
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
    for target in &args.artnet {
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
    if let Some(target) = args.sacn {
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("pult_backend=debug".parse()?))
        .init();

    let args = Args::parse();
    let config = Config {
        port: args.port,
        sync_port: args.sync_port,
        showfile: args.showfile.clone(),
        ..Config::default()
    };

    let pool = Arc::new(showfile::open(&config.showfile).await?);
    let node_id = NodeId::new();

    let (engine_tx, engine_rx) = mpsc::channel::<EngineCommand>(256);
    let engine_handle = EngineHandle(engine_tx);

    let (mut sync_mgr, sync_handle, sync_addr) =
        SyncManager::bind(node_id, config.sync_port, engine_handle.clone()).await?;
    info!("peer sync on {sync_addr}");

    let (mut engine, _broadcast) = ShowEngine::new_with_rx(
        node_id,
        engine_rx,
        pool.clone(),
        Some(sync_handle.clone()),
    );

    // Every node browses for OpenHaunt devices; only the one leading the session
    // adopts or commands any of them.
    let (device_mgr, device_handle, device_directory) =
        DeviceManager::new(node_id, engine_handle.clone(), args.openhaunt_broker_port);
    tokio::spawn(device_mgr.run());
    spawn_mdns_browser(device_handle.clone());

    // Which outputs exist is show data now. The manager reconciles against the
    // `outputs` collection, and the engine hands it that collection whenever it
    // changes — including once at load, so a saved show comes up sending.
    let (output_mgr, output) =
        OutputManager::new(node_id, engine_handle.clone(), Some((device_directory, device_handle.clone())));
    tokio::spawn(output_mgr.run());
    engine.set_output(output);

    engine_handle.0.send(EngineCommand::LoadFromShowfile).await?;
    tokio::spawn(engine.run());

    // The flags survive as a way to seed an empty showfile. Anything already
    // configured wins: a flag should not quietly add a second output every start.
    seed_outputs_from_flags(&engine_handle, node_id, &args).await;

    let (session_mgr, session_handle) =
        SessionManager::new(node_id, config.sync_port, engine_handle.clone(), sync_handle.clone());
    // If the leader disappears and this node wins the election, the session layer
    // has to start advertising so newcomers find the show here.
    sync_mgr.on_promotion(session_mgr.promotion_sender());
    tokio::spawn(sync_mgr.run());
    tokio::spawn(session_mgr.run());

    let state = AppState {
        engine: engine_handle,
        pool,
        sync: sync_handle,
        session: session_handle,
        devices: device_handle,
        node_id,
        ws_registry: SubscriptionRegistry::default(),
        broadcast: _broadcast,
    };

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    info!("pult-backend listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
