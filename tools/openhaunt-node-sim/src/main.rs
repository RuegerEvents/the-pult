//! Run a simulated OpenHaunt node.
//!
//!     openhaunt-node-sim --module input --serial 1a2b3c
//!     openhaunt-node-sim --config configs/fogger.json
//!
//! Then type at it: `in 3 1` closes contact 3, `in 3 0` opens it, `read 0 21.5`
//! reports a sensor value.
//!
//! `--module` picks one of the catalogue presets; `--config` runs whatever a file
//! says, which is how a node this crate has never heard of gets simulated. The
//! other flags override either.

use anyhow::Result;
use clap::Parser;
use openhaunt_node_sim::{start, Input, ModuleKind, NodeConfig, SimConfig, SACN_PORT};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(about = "a simulated OpenHaunt I/O node")]
struct Args {
    /// A node config file: identity, module descriptor and ports. Overrides
    /// `--module`, and the flags below override it in turn.
    #[arg(long, value_name = "PATH")]
    config: Option<std::path::PathBuf>,
    /// dmx, input, led, relay, oled, contact, or env.
    #[arg(long, default_value = "input")]
    module: String,
    #[arg(long)]
    serial: Option<String>,
    /// HTTP control port. 0 asks the OS for a free one.
    #[arg(long)]
    port: Option<u16>,
    /// Do not advertise over mDNS. The node is then only reachable if something
    /// already knows its address.
    #[arg(long)]
    quiet: bool,
    /// Toggle an input, or report a reading, every this many milliseconds.
    #[arg(long, value_name = "MS")]
    auto: Option<u64>,
    /// Write the config this run would use to a file and exit. A preset is the
    /// easiest thing to start editing from.
    #[arg(long, value_name = "PATH")]
    write_config: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("openhaunt_node_sim=info".parse()?))
        .init();

    let args = Args::parse();
    let config = resolve(&args)?;

    for problem in config.problems() {
        eprintln!("warning: {problem}");
    }

    if let Some(path) = &args.write_config {
        config.write(path)?;
        println!("wrote {}", path.display());
        return Ok(());
    }

    let name = config.module.name.clone();
    let handle = start(SimConfig { node: config, sacn_port: SACN_PORT }).await?;

    println!("{name} on {}", handle.http_addr);
    println!("type `in <port> <0|1>`, `read <port> <value>`, or ctrl-c");

    let inputs = handle.inputs.clone();
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(stdin));
        while let Ok(Some(line)) = lines.next_line().await {
            match parse_command(&line) {
                Some(input) => {
                    let _ = inputs.send(input).await;
                }
                None if line.trim().is_empty() => {}
                None => println!("? {line}"),
            }
        }
    });

    // Frames arriving on sACN are worth seeing when driving a gateway by hand.
    let mut frames = handle.sacn_frames;
    while let Some((universe, channels)) = frames.recv().await {
        let lit = channels.iter().filter(|c| **c > 0).count();
        println!("sACN universe {universe}: {lit} channels above zero");
    }

    // A node with no sACN socket has nothing else to wait for.
    std::future::pending::<()>().await;
    Ok(())
}

/// The config this run should use: a file or a preset, with the flags on top.
///
/// Flags win over a file so that one config can be started twice on one machine —
/// two serials, two ports, one description.
fn resolve(args: &Args) -> Result<NodeConfig> {
    let mut config = match &args.config {
        Some(path) => NodeConfig::read(path)?,
        None => {
            let module = ModuleKind::parse(&args.module)
                .ok_or_else(|| anyhow::anyhow!("unknown module: {}", args.module))?;
            let mut config = module.config("1a2b3c");
            // A preset is meant to be run, and running means being findable on
            // port 80 like a real node. Its defaults are the ones tests want.
            config.http_port = 80;
            config.advertise = true;
            config
        }
    };

    if let Some(serial) = &args.serial {
        // The name usually carries the serial, and a renamed node keeping the old
        // one in its friendly name is the sort of thing that wastes an afternoon.
        if config.name.ends_with(&config.serial) {
            let stem = config.name.trim_end_matches(&config.serial).trim_end();
            config.name = format!("{stem} {serial}");
        }
        config.serial = serial.clone();
    }
    if let Some(port) = args.port {
        config.http_port = port;
    }
    if args.quiet {
        config.advertise = false;
    }
    if let Some(auto) = args.auto {
        config.auto_ms = Some(auto);
    }
    Ok(config)
}

fn parse_command(line: &str) -> Option<Input> {
    let mut words = line.split_whitespace();
    match words.next()? {
        "in" => Some(Input::Contact {
            port: words.next()?.parse().ok()?,
            state: words.next()? != "0",
        }),
        "read" => Some(Input::Reading {
            port: words.next()?.parse().ok()?,
            value: words.next()?.parse().ok()?,
        }),
        _ => None,
    }
}
