//! Output: getting fixture state onto a wire.
//!
//! The spec calls for output plugins that translate high-level data into whatever
//! protocol a fixture speaks, with network-based communication preferred over
//! DMX-centric workflows. So the shape here is a plugin trait and a manager, and
//! Art-Net is one implementation of that trait rather than the centre of it.
//!
//! Which plugins exist is show data. [`OutputManager`] is handed the `outputs`
//! collection and reconciles what it is running against it, so an output can be
//! added, re-addressed, or switched off while the show is up.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use chrono::Utc;
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        fixture::{Fixture, FixtureType},
        output::{OutputConfig, OutputKind, OutputStatus, OutputStatuses},
    },
};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use uuid::Uuid;

pub mod artnet;
pub mod dmx;
pub mod openhaunt;
pub mod sacn;

use crate::{
    engine::EngineHandle,
    infra::devices::{DeviceDirectory, DeviceHandle},
};
use dmx::Patch;

/// What an OpenHaunt output needs to reach adopted nodes.
type Devices = Option<(watch::Receiver<DeviceDirectory>, DeviceHandle)>;

// ── OutputPlugin ──────────────────────────────────────────────────────────────

/// What one call to [`OutputPlugin::send`] returns.
///
/// Boxed rather than `impl Future`, because a trait with `async fn` cannot be used
/// behind `dyn`, and a rig can have several outputs at once: Art-Net to the house
/// and sACN to a guest console is an ordinary evening.
pub type SendFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// Something that puts fixture state on a wire.
///
/// `send` is called on the output manager's own schedule, not the engine's, so a
/// protocol that has to refresh at a fixed rate can do so without the engine
/// knowing anything about it.
pub trait OutputPlugin: Send {
    fn name(&self) -> &'static str;

    /// Emit the current state of the patch. `changed` is the fixtures that moved
    /// since the last call, which a protocol that sends deltas can use and one that
    /// sends whole frames can ignore.
    fn send<'a>(&'a mut self, patch: &'a Patch, changed: &'a [Uuid]) -> SendFuture<'a>;
}

// ── OutputManager ─────────────────────────────────────────────────────────────

#[allow(dead_code, reason = "Stop has no caller until the server shuts down gracefully")]
pub enum OutputCommand {
    /// The engine's view of the patch, pushed whenever fixture output changes.
    Patch { fixtures: Vec<Fixture>, fixture_types: Vec<FixtureType>, changed: Vec<Uuid> },
    /// The `outputs` collection changed. Reconcile against it.
    Configure(Vec<OutputConfig>),
    Stop,
}

#[derive(Clone)]
pub struct OutputHandle(pub mpsc::Sender<OutputCommand>);

impl OutputHandle {
    /// Hand the current patch to the output side. Never blocks the engine: if output
    /// is behind, the update is dropped, because the next tick carries the same state.
    pub fn push(&self, fixtures: Vec<Fixture>, fixture_types: Vec<FixtureType>, changed: Vec<Uuid>) {
        let _ = self.0.try_send(OutputCommand::Patch { fixtures, fixture_types, changed });
    }

    /// Hand over the configured outputs. Same rule: never block the engine.
    pub fn configure(&self, outputs: Vec<OutputConfig>) {
        let _ = self.0.try_send(OutputCommand::Configure(outputs));
    }
}

/// One running output: the plugin, and what it was built from.
struct Running {
    config: OutputConfig,
    plugin: Box<dyn OutputPlugin>,
    status: OutputStatus,
    /// Sends since the last status report, for the frame rate.
    sends_since_report: u32,
    reported_at: std::time::Instant,
}

impl Running {
    fn new(config: OutputConfig, plugin: Box<dyn OutputPlugin>) -> Self {
        Running {
            status: OutputStatus {
                name: config.name.clone(),
                kind: format!("{:?}", config.kind).to_lowercase(),
                running: true,
                ..Default::default()
            },
            config,
            plugin,
            sends_since_report: 0,
            reported_at: std::time::Instant::now(),
        }
    }
}

/// Owns the output plugins and feeds them.
pub struct OutputManager {
    node_id: NodeId,
    engine: EngineHandle,
    running: HashMap<Uuid, Running>,
    rx: mpsc::Receiver<OutputCommand>,
    devices: Devices,
}

impl OutputManager {
    pub fn new(node_id: NodeId, engine: EngineHandle, devices: Devices) -> (Self, OutputHandle) {
        let (tx, rx) = mpsc::channel(4);
        (Self { node_id, engine, running: HashMap::new(), rx, devices }, OutputHandle(tx))
    }

    /// Seed a plugin without going through a configuration. Test-only: it is how a
    /// stand-in for a protocol gets in, so the manager's own behaviour can be tested
    /// without a socket on the other end.
    #[cfg(test)]
    pub fn preload(&mut self, config: OutputConfig, plugin: Box<dyn OutputPlugin>) {
        self.running.insert(config.id, Running::new(config, plugin));
    }

    pub async fn run(mut self) {
        info!("[output] started");
        // Status is reported on a timer rather than per send: at 40 Hz a frame count
        // only means anything over a window, and nothing is served by rewriting LOCAL
        // state forty times a second per output.
        //
        // Starting a period late, not immediately: `interval` fires straight away,
        // which would divide the first second's sends by a window microseconds wide
        // and report a rate in the thousands.
        let period = std::time::Duration::from_secs(1);
        let mut report = tokio::time::interval_at(tokio::time::Instant::now() + period, period);

        loop {
            tokio::select! {
                cmd = self.rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        OutputCommand::Stop => break,
                        OutputCommand::Configure(outputs) => self.reconcile(outputs).await,
                        OutputCommand::Patch { fixtures, fixture_types, changed } => {
                            self.send_patch(fixtures, fixture_types, changed).await;
                        }
                    }
                }
                _ = report.tick() => {
                    self.measure_rates();
                    self.publish_status().await;
                }
            }
        }
        info!("[output] stopped");
    }

    async fn send_patch(
        &mut self,
        fixtures: Vec<Fixture>,
        fixture_types: Vec<FixtureType>,
        changed: Vec<Uuid>,
    ) {
        let patch = Patch {
            fixtures,
            fixture_types: fixture_types.into_iter().map(|t| (t.id, t)).collect(),
        };
        // Sequentially, and one plugin's failure does not stop the rest: an unplugged
        // Art-Net interface must not silence sACN.
        for output in self.running.values_mut() {
            match output.plugin.send(&patch, &changed).await {
                Ok(()) => {
                    output.status.last_send = Some(Utc::now());
                    output.sends_since_report += 1;
                }
                Err(e) => {
                    warn!("[output] {}: {e}", output.config.name);
                    output.status.error_count += 1;
                    output.status.last_error = Some(e.to_string());
                }
            }
        }
    }

    /// Bring the running plugins in line with the configured outputs.
    ///
    /// Rebuilt only where the configuration actually changed, so renaming one output
    /// does not drop and re-open every socket in the rig — which for Art-Net would
    /// reset the dedup cache and put a redundant frame on the wire for a label edit.
    async fn reconcile(&mut self, outputs: Vec<OutputConfig>) {
        let wanted: HashMap<Uuid, OutputConfig> = outputs
            .into_iter()
            .filter(|o| o.runs_on(self.node_id))
            .map(|o| (o.id, o))
            .collect();

        let gone: Vec<Uuid> =
            self.running.keys().copied().filter(|id| !wanted.contains_key(id)).collect();
        for id in gone {
            if let Some(output) = self.running.remove(&id) {
                info!("[output] stopped {}", output.config.name);
            }
        }

        for (id, config) in wanted {
            match self.running.get_mut(&id) {
                // Same wire, different label: keep the socket, take the new name.
                Some(existing) if same_wire(&existing.config, &config) => {
                    existing.status.name = config.name.clone();
                    existing.config = config;
                }
                _ => match build(&self.devices, &config).await {
                    Some(plugin) => {
                        info!("[output] {} → {} ({})", config.name, describe(&config), plugin.name());
                        self.running.insert(id, Running::new(config, plugin));
                    }
                    None => {
                        self.running.remove(&id);
                    }
                },
            }
        }
        self.publish_status().await;
    }

    /// Close the frame-rate window and open a new one. Timer only: doing this
    /// whenever a status happens to be published would divide the count by whatever
    /// gap preceded it, and report a rate in the thousands after a reconfigure.
    fn measure_rates(&mut self) {
        for output in self.running.values_mut() {
            let elapsed = output.reported_at.elapsed().as_secs_f32();
            if elapsed > 0.0 {
                output.status.frames_per_second = output.sends_since_report as f32 / elapsed;
            }
            output.sends_since_report = 0;
            output.reported_at = std::time::Instant::now();
        }
    }

    /// Publish what every output has been doing, as LOCAL state.
    async fn publish_status(&mut self) {
        let statuses: OutputStatuses = self
            .running
            .iter()
            .map(|(id, output)| (id.to_string(), output.status.clone()))
            .collect();

        if let Ok(json) = serde_json::to_value(&statuses) {
            let path = vec![PathSegment::Key("output_status".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
    }
}

/// Open the socket an output needs.
///
/// A free function rather than a method: the manager holds boxed plugins across this
/// await, and borrowing `&self` here would require the whole manager to be `Sync`,
/// which a plugin is not and does not need to be.
async fn build(devices: &Devices, config: &OutputConfig) -> Option<Box<dyn OutputPlugin>> {
    match config.kind {
        OutputKind::Artnet => {
            // No address is not a default to guess at — Art-Net has nowhere to go.
            let target = parse_target(config.target.as_deref()?, artnet::ARTNET_PORT)?;
            bound(config, artnet::ArtNetOutput::bind(target).await)
        }
        OutputKind::Sacn => {
            let target = match config.target.as_deref() {
                Some(addr) => Some(parse_target(addr, sacn::SACN_PORT)?),
                None => None, // the per-universe multicast groups
            };
            bound(config, sacn::SacnOutput::bind(target).await)
        }
        OutputKind::OpenHaunt => {
            let (directory, handle) = devices.clone()?;
            bound(
                config,
                openhaunt::OpenHauntOutput::new(directory, handle, sacn::SACN_PORT).await,
            )
        }
    }
}

/// Box a plugin that opened its socket, or say why it did not.
fn bound<P: OutputPlugin + 'static>(
    config: &OutputConfig,
    result: anyhow::Result<P>,
) -> Option<Box<dyn OutputPlugin>> {
    match result {
        Ok(plugin) => Some(Box::new(plugin)),
        Err(e) => {
            warn!("[output] {} could not start: {e}", config.name);
            None
        }
    }
}

/// Would these two configurations open the same socket to the same place?
fn same_wire(a: &OutputConfig, b: &OutputConfig) -> bool {
    a.kind == b.kind && a.target == b.target && a.universes == b.universes
}

fn describe(config: &OutputConfig) -> String {
    match (&config.kind, &config.target) {
        (OutputKind::Artnet, Some(target)) => format!("Art-Net {target}"),
        (OutputKind::Sacn, Some(target)) => format!("sACN {target}"),
        (OutputKind::Sacn, None) => "sACN multicast".to_string(),
        (OutputKind::OpenHaunt, _) => "adopted OpenHaunt nodes".to_string(),
        (kind, None) => format!("{kind:?} with no target"),
    }
}

/// Accept either `host:port` or a bare address, defaulting to the protocol's port.
pub fn parse_target(value: &str, default_port: u16) -> Option<std::net::SocketAddr> {
    if let Ok(addr) = value.parse::<std::net::SocketAddr>() {
        return Some(addr);
    }
    value
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| std::net::SocketAddr::new(ip, default_port))
}

#[cfg(test)]
mod tests;
