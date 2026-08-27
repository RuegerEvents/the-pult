//! A window onto a simulated OpenHaunt node.
//!
//! The node runs in this process, so the panel talks to it over Tauri's own IPC
//! rather than over anything on the wire: `openhaunt-node-sim` implements the node
//! side of the OpenHaunt protocol and nothing else, and a debug UI is not part of
//! that protocol. What the panel adds is the thing the command line cannot do
//! under `scripts/demo.sh` — its stdin is not connected there, which is why the
//! input node has to be started with `--auto`. Here a contact is a button.
//!
//! It also edits the node. Since a node is nothing but what it says about itself,
//! being able to change that — a port's unit, a whole new module nobody has built
//! — is the difference between a simulator of seven modules and a simulator of
//! whatever a console will one day meet.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::PathBuf, time::Duration};

use clap::Parser;
use openhaunt_node_sim::{
    start, Input, ModuleKind, NodeConfig, SimConfig, SimHandle, Snapshot, SACN_PORT,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// The same flags the `openhaunt-node-sim` binary takes, so the two are started
/// the same way and a node can be moved from one to the other mid-debug.
#[derive(Parser, Clone)]
#[command(about = "a simulated OpenHaunt I/O node, with a panel", version)]
struct Args {
    /// A node config file to open. Overrides `--module`.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
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

/// The configs shipped with the simulator, embedded rather than read off disk so
/// that a bundled app has them wherever it was dropped.
const DEMOS: &[(&str, &str)] = &[
    ("Fog machine", include_str!("../../openhaunt-node-sim/configs/fog-machine.json")),
    ("Weather station", include_str!("../../openhaunt-node-sim/configs/weather-station.json")),
    ("Haunted mirror", include_str!("../../openhaunt-node-sim/configs/mirror.json")),
    ("Old firmware", include_str!("../../openhaunt-node-sim/configs/old-firmware.json")),
];

// ── The running node ──────────────────────────────────────────────────────────

/// What the panel is allowed to do to the node.
///
/// A node is its config, so changing the config means putting the old node away
/// and starting a new one — the sockets and the mDNS record are not editable in
/// place. The window survives that, which is the only reason any of this is
/// behind a lock rather than being three fields.
struct Sim {
    session: Mutex<Session>,
    /// Fixed for the life of the process: a rebound node takes the same one back.
    sacn_port: u16,
    app: AppHandle,
}

struct Session {
    inputs: mpsc::Sender<Input>,
    snapshot: tokio::sync::watch::Receiver<Snapshot>,
    stop: openhaunt_node_sim::Stopper,
    /// The two tasks forwarding this node to the window. Replaced with it.
    forwarders: Vec<tauri::async_runtime::JoinHandle<()>>,
}

impl Session {
    /// Start a node and wire it to the window.
    fn open(app: &AppHandle, handle: SimHandle) -> Session {
        let forwarders = vec![
            tauri::async_runtime::spawn(publish_state(app.clone(), handle.snapshot.clone())),
            tauri::async_runtime::spawn(publish_sacn(app.clone(), handle.sacn_frames)),
        ];
        Session {
            inputs: handle.inputs,
            snapshot: handle.snapshot,
            stop: handle.stop,
            forwarders,
        }
    }

    /// Put the node away and stop talking about it.
    async fn close(&mut self) {
        for forwarder in self.forwarders.drain(..) {
            forwarder.abort();
        }
        self.stop.stop().await;
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// The panel asks for this once on mount rather than waiting for something to
/// change, since a node that nobody has touched yet is still worth drawing.
#[tauri::command]
async fn snapshot(sim: State<'_, Sim>) -> Result<Snapshot, String> {
    Ok(sim.session.lock().await.snapshot.borrow().clone())
}

#[tauri::command]
async fn contact(port: u8, state: bool, sim: State<'_, Sim>) -> Result<(), String> {
    let inputs = sim.session.lock().await.inputs.clone();
    inputs.send(Input::Contact { port, state }).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn reading(port: u8, value: f32, sim: State<'_, Sim>) -> Result<(), String> {
    let inputs = sim.session.lock().await.inputs.clone();
    inputs.send(Input::Reading { port, value }).await.map_err(|e| e.to_string())
}

/// Run this config instead of the one running now.
///
/// The old node is stopped first, because it is holding the ports the new one
/// probably wants. If the new one will not start — a port something else has, a
/// serial that is empty — the old one comes back, so a typo costs an error
/// message rather than the node the window was watching.
#[tauri::command]
async fn apply(config: NodeConfig, sim: State<'_, Sim>) -> Result<Snapshot, String> {
    let problems = config.problems();
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }

    let mut session = sim.session.lock().await;
    let previous = session.snapshot.borrow().config.clone();
    session.close().await;

    match start(SimConfig { node: config, sacn_port: sim.sacn_port }).await {
        Ok(handle) => {
            let snapshot = handle.snapshot.borrow().clone();
            *session = Session::open(&sim.app, handle);
            Ok(snapshot)
        }
        Err(e) => {
            let restored = start(SimConfig { node: previous, sacn_port: sim.sacn_port })
                .await
                .map_err(|back| format!("{e} — and the node it replaced would not restart: {back}"))?;
            *session = Session::open(&sim.app, restored);
            Err(e.to_string())
        }
    }
}

/// The catalogue presets, as configs to start editing from.
#[tauri::command]
fn presets() -> Vec<NodeConfig> {
    ModuleKind::ALL.iter().map(|module| module.config("1a2b3c")).collect()
}

/// One of the configs shipped with the simulator.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Demo {
    name: String,
    config: NodeConfig,
}

#[tauri::command]
fn demos() -> Vec<Demo> {
    DEMOS
        .iter()
        .filter_map(|(name, json)| {
            // A demo that will not parse is this crate's bug, not the operator's,
            // so it is dropped from the list rather than shown as an error.
            let config = serde_json::from_str(json).ok()?;
            Some(Demo { name: name.to_string(), config })
        })
        .collect()
}

#[tauri::command]
fn load_config(path: PathBuf) -> Result<NodeConfig, String> {
    NodeConfig::read(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_config(path: PathBuf, config: NodeConfig) -> Result<(), String> {
    config.write(path).map_err(|e| e.to_string())
}

// ── Starting up ───────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("openhaunt_node_sim=info".parse()?))
        .init();

    let args = Args::parse();
    let config = opening_config(&args)?;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            snapshot,
            contact,
            reading,
            apply,
            presets,
            demos,
            load_config,
            save_config,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let sim = tauri::async_runtime::block_on(async {
                start(SimConfig { node: config, sacn_port: SACN_PORT }).await
            })?;

            let session = Session::open(&handle, sim);
            app.manage(Sim {
                session: Mutex::new(session),
                sacn_port: SACN_PORT,
                app: handle,
            });
            Ok(())
        })
        .run(tauri::generate_context!())?;
    Ok(())
}

/// The node this window opens on: a file if one was named, a preset otherwise.
fn opening_config(args: &Args) -> anyhow::Result<NodeConfig> {
    let mut config = match &args.config {
        Some(path) => NodeConfig::read(path)?,
        None => {
            let module = ModuleKind::parse(&args.module)
                .ok_or_else(|| anyhow::anyhow!("unknown module: {}", args.module))?;
            let mut config = module.config(args.serial.clone());
            config.http_port = args.port;
            config.advertise = !args.quiet;
            config
        }
    };
    if args.auto.is_some() {
        config.auto_ms = args.auto;
    }
    Ok(config)
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

/// One E1.31 frame, as the panel draws it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    universe: u16,
    channels: Vec<u8>,
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
