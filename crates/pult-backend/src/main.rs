//! The console as a server: flags in, [`pult_backend::start`] out.
//!
//! Everything this used to do inline now lives in the library, so the desktop app
//! starts a station exactly the way this does.

use anyhow::Result;
use clap::Parser;
use pult_backend::{
    infra::connectors::{artnet::ARTNET_PORT, sacn::SACN_PORT},
    Config,
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(about = "pult-backend lighting console server", version)]
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
    /// Use this station id instead of the one recorded beside the showfile. For
    /// moving a station's identity to different hardware, and for tests.
    #[arg(long, value_name = "UUID")]
    node_id: Option<uuid::Uuid>,
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("pult_backend=debug".parse()?))
        .init();

    let args = Args::parse();
    let running = pult_backend::start(Config {
        port: args.port,
        sync_port: args.sync_port,
        showfile: args.showfile,
        artnet: args.artnet,
        sacn: args.sacn,
        openhaunt_broker_port: args.openhaunt_broker_port,
        node_id: args.node_id,
        ..Config::default()
    })
    .await?;

    running.serve.await?
}
