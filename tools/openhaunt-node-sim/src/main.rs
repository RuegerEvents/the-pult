//! Run a simulated OpenHaunt node.
//!
//!     openhaunt-node-sim --module input --serial 1a2b3c
//!
//! Then type at it: `in 3 1` closes contact 3, `in 3 0` opens it, `read 0 21.5`
//! reports a sensor value.

use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use openhaunt_node_sim::{start, Input, ModuleKind, SimConfig, SACN_PORT};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(about = "a simulated OpenHaunt I/O node")]
struct Args {
    /// dmx, input, led, relay, oled, contact, or env.
    #[arg(long, default_value = "input")]
    module: String,
    #[arg(long, default_value = "1a2b3c")]
    serial: String,
    /// HTTP control port. 0 asks the OS for a free one.
    #[arg(long, default_value_t = 80)]
    port: u16,
    /// Do not advertise over mDNS. The node is then only reachable if something
    /// already knows its address.
    #[arg(long)]
    quiet: bool,
    /// Toggle an input, or report a reading, every this many milliseconds.
    #[arg(long, value_name = "MS")]
    auto: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("openhaunt_node_sim=info".parse()?))
        .init();

    let args = Args::parse();
    let module = ModuleKind::parse(&args.module)
        .ok_or_else(|| anyhow::anyhow!("unknown module: {}", args.module))?;

    let handle = start(SimConfig {
        http_port: args.port,
        sacn_port: SACN_PORT,
        advertise: !args.quiet,
        auto: args.auto.map(Duration::from_millis),
        ..SimConfig::new(module, args.serial)
    })
    .await?;

    println!("{} on {}", module.name(), handle.http_addr);
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
