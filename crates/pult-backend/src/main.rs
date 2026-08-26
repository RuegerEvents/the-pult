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
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::{
    api::ws::{ws_handler, SubscriptionRegistry},
    config::Config,
    engine::{EngineCommand, EngineHandle, ShowEngine},
    infra::connectors::{artnet::{ArtNetOutput, ARTNET_PORT}, OutputManager},
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
    /// The port defaults to 6454. Off unless given: a console should not put
    /// packets on someone's network because it happened to start up.
    #[arg(long, value_name = "ADDR", value_parser = parse_artnet_target)]
    artnet: Option<std::net::SocketAddr>,
}

/// Accept either `host:port` or a bare address, defaulting to the Art-Net port.
fn parse_artnet_target(value: &str) -> Result<std::net::SocketAddr, String> {
    if let Ok(addr) = value.parse::<std::net::SocketAddr>() {
        return Ok(addr);
    }
    value
        .parse::<std::net::IpAddr>()
        .map(|ip| std::net::SocketAddr::new(ip, ARTNET_PORT))
        .map_err(|e| format!("not an address: {e}"))
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
        showfile: args.showfile,
        ..Config::default()
    };

    let pool = Arc::new(showfile::open(&config.showfile).await?);
    let node_id = NodeId::new();

    let (engine_tx, engine_rx) = mpsc::channel::<EngineCommand>(256);
    let engine_handle = EngineHandle(engine_tx);

    let (sync_mgr, sync_handle, sync_addr) =
        SyncManager::bind(node_id, config.sync_port, engine_handle.clone()).await?;
    info!("peer sync on {sync_addr}");
    tokio::spawn(sync_mgr.run());

    let (mut engine, _broadcast) = ShowEngine::new_with_rx(
        node_id,
        engine_rx,
        pool.clone(),
        Some(sync_handle.clone()),
    );

    if let Some(target) = args.artnet {
        match ArtNetOutput::bind(target).await {
            Ok(plugin) => {
                let (manager, output) = OutputManager::new(plugin);
                tokio::spawn(manager.run());
                engine.set_output(output);
                info!("Art-Net output to {target}");
            }
            Err(e) => warn!("Art-Net output disabled: {e}"),
        }
    }
    engine_handle.0.send(EngineCommand::LoadFromShowfile).await?;
    tokio::spawn(engine.run());

    let (session_mgr, session_handle) =
        SessionManager::new(node_id, config.sync_port, engine_handle.clone(), sync_handle.clone());
    tokio::spawn(session_mgr.run());

    let state = AppState {
        engine: engine_handle,
        pool,
        sync: sync_handle,
        session: session_handle,
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
