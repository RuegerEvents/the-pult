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

#[derive(Parser)]
#[command(about = "pult-backend lighting console server", version)]
struct Args {
    #[arg(long, default_value_t = 7700)]
    port: u16,
    #[arg(long, default_value_t = 7701)]
    sync_port: u16,
    /// The show to open: a `Name.pult` bundle directory, made if it is not there.
    /// Left out, the console starts with no show open and serves the welcome screen.
    #[arg(long, value_name = "BUNDLE")]
    show: Option<std::path::PathBuf>,
    /// Where this station's own id is kept. Defaults to `PULT_IDENTITY` and then to
    /// the configuration directory — it belongs to the machine now, not to the show,
    /// so a copied bundle no longer clones the station that made it.
    #[arg(long, value_name = "FILE")]
    identity: Option<std::path::PathBuf>,
    /// Where the shows this console makes for itself go, and what the welcome screen
    /// lists. Defaults to the station preference and then to the data directory.
    #[arg(long, value_name = "DIR")]
    shows_dir: Option<std::path::PathBuf>,
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
    /// Use this station id instead of the one this machine has recorded. For
    /// moving a station's identity to different hardware, and for tests.
    #[arg(long, value_name = "UUID")]
    node_id: Option<uuid::Uuid>,
    /// Load WASM plugins from this directory — either one plugin's directory or
    /// a directory of them. Repeat the flag for several. Changed files reload
    /// the plugin while the console runs.
    #[arg(long = "plugins", value_name = "DIR")]
    plugin_dirs: Vec<std::path::PathBuf>,
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
    // Builds the subscriber this binary used to build by hand, with a capture layer
    // beside the `fmt` one so the console can show its own log. Levels come from
    // preferences a moment later, once the station has read them.
    let log = pult_backend::logging::install(pult_backend::logging::LogOptions::default())?;

    let args = Args::parse();
    // A `Console` rather than a bare station: a station is built around one showfile
    // and cannot change it, so opening a show is this one stopping and another
    // starting in its place. The console is the thing that outlives both.
    let console = pult_backend::Console::start(Config {
        port: args.port,
        sync_port: args.sync_port,
        show: args.show,
        identity: args.identity,
        shows_dir: args.shows_dir,
        artnet: args.artnet,
        sacn: args.sacn,
        openhaunt_broker_port: args.openhaunt_broker_port,
        node_id: args.node_id,
        plugin_dirs: args.plugin_dirs,
        log: Some(log),
        ..Config::default()
    })
    .await?;

    console.serve().await
}
