//! A window onto a simulated OpenHaunt node.
//!
//! The node runs in this process, so the panel talks to it over Tauri's own IPC
//! rather than over anything on the wire: `openhaunt-sim` implements the node side
//! of the OpenHaunt protocol and nothing else, and a debug UI is not part of that
//! protocol. What the panel adds is the thing the command line cannot do under
//! `scripts/demo.sh` — its stdin is not connected there, which is why the input
//! node has to be started with `--auto`. Here a contact is a button.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use clap::Parser;
use openhaunt_sim::{start, Input, ModuleKind, SimConfig, Snapshot, SACN_PORT};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// The same flags the `openhaunt-sim` binary takes, so the two are started the
/// same way and a node can be moved from one to the other mid-debug.
#[derive(Parser, Clone)]
#[command(about = "a simulated OpenHaunt I/O node, with a panel", version)]
struct Args {
    /// dmx, input, led, relay, oled, contact, or env.
    #[arg(long, default_value = "input")]
    module: String,
    #[arg(long, default_value = "1a2b3c")]
    serial: String,
    /// HTTP control port. 0 asks the OS for a free one.
    #[arg(long, default_value_t = 8801)]
    port: u16,
    /// Do not advertise over mDNS. The node is then only reachable if something
    /// already knows its address.
    #[arg(long)]
    quiet: bool,
    /// Toggle an input, or report a reading, every this many milliseconds.
    #[arg(long, value_name = "MS")]
    auto: Option<u64>,
}

/// What the panel is allowed to do to the node.
struct Sim {
    inputs: mpsc::Sender<Input>,
    snapshot: tokio::sync::watch::Receiver<Snapshot>,
}

/// One E1.31 frame, as the panel draws it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    universe: u16,
    channels: Vec<u8>,
}

/// The panel asks for this once on mount rather than waiting for something to
/// change, since a node that nobody has touched yet is still worth drawing.
#[tauri::command]
fn snapshot(sim: State<'_, Sim>) -> Snapshot {
    sim.snapshot.borrow().clone()
}

#[tauri::command]
async fn contact(port: u8, state: bool, sim: State<'_, Sim>) -> Result<(), String> {
    sim.inputs
        .send(Input::Contact { port, state })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn reading(port: u8, value: f32, sim: State<'_, Sim>) -> Result<(), String> {
    sim.inputs
        .send(Input::Reading { port, value })
        .await
        .map_err(|e| e.to_string())
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("openhaunt_sim=info".parse()?))
        .init();

    let args = Args::parse();
    let module = ModuleKind::parse(&args.module)
        .ok_or_else(|| anyhow::anyhow!("unknown module: {}", args.module))?;

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![snapshot, contact, reading])
        .setup(move |app| {
            let handle = app.handle().clone();
            let sim = tauri::async_runtime::block_on(async move {
                let handle = start(SimConfig {
                    http_port: args.port,
                    sacn_port: SACN_PORT,
                    advertise: !args.quiet,
                    auto: args.auto.map(Duration::from_millis),
                    ..SimConfig::new(module, args.serial.clone())
                })
                .await?;
                anyhow::Ok(handle)
            })?;

            tauri::async_runtime::spawn(publish_state(handle.clone(), sim.snapshot.clone()));
            tauri::async_runtime::spawn(publish_sacn(handle, sim.sacn_frames));

            app.manage(Sim {
                inputs: sim.inputs,
                snapshot: sim.snapshot,
            });
            Ok(())
        })
        .run(tauri::generate_context!())?;
    Ok(())
}

/// Forward the node's own account of itself whenever it changes.
async fn publish_state(app: AppHandle, mut snapshot: tokio::sync::watch::Receiver<Snapshot>) {
    loop {
        let current = snapshot.borrow_and_update().clone();
        let _ = app.emit("sim://state", current);
        if snapshot.changed().await.is_err() {
            return;
        }
    }
}

/// Forward received sACN, at a rate a window can be drawn at.
///
/// A gateway is sent a frame forty times a second and no eye reads a 512-channel
/// bar chart that fast, so only the newest frame per universe survives each tick.
/// It is dropped frames on purpose: the last one is the only one that is true.
async fn publish_sacn(app: AppHandle, mut frames: mpsc::Receiver<(u16, Vec<u8>)>) {
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    let mut latest: std::collections::BTreeMap<u16, Vec<u8>> = Default::default();

    loop {
        tokio::select! {
            frame = frames.recv() => match frame {
                Some((universe, channels)) => { latest.insert(universe, channels); }
                None => return,
            },
            _ = ticker.tick() => {
                for (universe, channels) in std::mem::take(&mut latest) {
                    let _ = app.emit("sim://sacn", Frame { universe, channels });
                }
            }
        }
    }
}
