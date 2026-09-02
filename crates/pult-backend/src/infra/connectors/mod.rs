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
        output::{OutputConfig, OutputCoverage, OutputKind, OutputStatus, OutputStatuses},
        programmer::ProgrammerValue,
        station::FrameCost,
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

/// What one drawn frame cost the connector that drew it.
///
/// The manager times the whole call; only the connector can say how much of that was
/// working out what the patch is doing rather than putting it on a wire, so it says.
/// A connector that does not care reports nothing and the split reads as zero, which
/// is honest: it did no evaluating worth naming.
#[derive(Debug, Default, Clone, Copy)]
pub struct Frame {
    pub evaluating: std::time::Duration,
}

/// What one call to [`OutputPlugin::send`] returns.
///
/// Boxed rather than `impl Future`, because a trait with `async fn` cannot be used
/// behind `dyn`, and a rig can have several outputs at once: Art-Net to the house
/// and sACN to a guest console is an ordinary evening.
pub type SendFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<Frame>> + Send + 'a>>;

/// When a protocol wants a frame of its own, over and above the ones a change to the
/// show pushes at it.
///
/// Two rates because a protocol has two situations. While something is moving, a
/// connector that carries values has to draw them, and how often is its own business —
/// DMX at 40 Hz, a display when it changes and not otherwise. Once everything has
/// settled the values are not going anywhere, and what is left is whatever keep-alive
/// the protocol needs so a receiver does not decide the controller has gone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frames {
    pub while_moving: Option<std::time::Duration>,
    pub when_settled: Option<std::time::Duration>,
}

impl Frames {
    /// Nothing on its own account: this protocol speaks only when the show changes.
    pub const ON_CHANGE_ONLY: Frames = Frames { while_moving: None, when_settled: None };

    /// What the DMX family wants: a frame while anything moves, and a refresh often
    /// enough that a receiver does not time the controller out.
    pub const DMX: Frames = Frames {
        while_moving: Some(std::time::Duration::from_millis(25)),
        when_settled: Some(dmx::REFRESH_AFTER),
    };
}

/// Something that puts fixture state on a wire.
///
/// `send` is called on the output manager's own schedule, not the engine's, so a
/// protocol that has to refresh at a fixed rate can do so without the engine
/// knowing anything about it.
pub trait OutputPlugin: Send {
    fn name(&self) -> &'static str;

    /// Work out what the patch is doing at `now_ms` and emit it. `changed` is the
    /// fixtures the show moved since the last call, which a protocol that sends deltas
    /// can use and one that sends whole frames can ignore; on a frame the connector
    /// asked for itself it is empty, because the show did not change — the moment did.
    fn send<'a>(&'a mut self, patch: &'a Patch, changed: &'a [Uuid], now_ms: u64)
        -> SendFuture<'a>;

    /// How often this protocol wants a frame of its own.
    fn frames(&self) -> Frames {
        Frames::ON_CHANGE_ONLY
    }
}

// ── OutputManager ─────────────────────────────────────────────────────────────

#[allow(dead_code, reason = "Stop has no caller until the server shuts down gracefully")]
pub enum OutputCommand {
    /// The engine's view of what is driving the rig, pushed whenever the *show*
    /// changes — a cue taken, a fade started, a fixture patched. Not when a value
    /// moves: after this change the engine never separately learns that one has.
    Patch {
        fixtures: Vec<Fixture>,
        fixture_types: Vec<FixtureType>,
        programmer: Vec<ProgrammerValue>,
        changed: Vec<Uuid>,
    },
    /// The `outputs` collection changed. Reconcile against it.
    Configure(Vec<OutputConfig>),
    Stop,
}

#[derive(Clone)]
pub struct OutputHandle(pub mpsc::Sender<OutputCommand>);

impl OutputHandle {
    /// Hand the current patch to the output side. Never blocks the engine: if output
    /// is behind, the update is dropped, because a connector evaluating from the patch
    /// it already holds is not stalled by missing one.
    pub fn push(
        &self,
        fixtures: Vec<Fixture>,
        fixture_types: Vec<FixtureType>,
        programmer: Vec<ProgrammerValue>,
        changed: Vec<Uuid>,
    ) {
        let _ = self
            .0
            .try_send(OutputCommand::Patch { fixtures, fixture_types, programmer, changed });
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
    /// When this connector next wants a frame of its own, if it wants one at all.
    ///
    /// Its own, per connector, because their rates are their own: Art-Net drawing at
    /// 40 Hz beside an OpenHaunt node that was told about a fade once is the ordinary
    /// case, and one clock over both would make the second keep the first's time.
    next_frame: Option<std::time::Instant>,
    /// What this connector's frames have cost since the window was last closed.
    ///
    /// Plain integers on a struct only this task touches: measuring a frame must cost
    /// the same on a rig of five thousand as on a rig of five, and it must not put a
    /// lock or an allocation on the path it is measuring.
    frames: u32,
    total_us: u64,
    max_us: u64,
    evaluating_total_us: u64,
    evaluating_max_us: u64,
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
            next_frame: None,
            frames: 0,
            total_us: 0,
            max_us: 0,
            evaluating_total_us: 0,
            evaluating_max_us: 0,
        }
    }

    /// Record one drawn frame.
    fn measure(&mut self, whole: std::time::Duration, frame: Frame) {
        let whole_us = whole.as_micros() as u64;
        let evaluating_us = frame.evaluating.as_micros() as u64;
        self.frames += 1;
        self.total_us += whole_us;
        self.max_us = self.max_us.max(whole_us);
        self.evaluating_total_us += evaluating_us;
        self.evaluating_max_us = self.evaluating_max_us.max(evaluating_us);
    }

    /// Close the window and start a new one.
    ///
    /// `None` when it contained no frames at all — a connector whose protocol is idle
    /// and sending nothing — because zero would read as "instant", which is the
    /// opposite of the truth: nothing was measured at all.
    fn close_window(&mut self, elapsed: std::time::Duration) -> Option<FrameCost> {
        let frames = std::mem::take(&mut self.frames);
        let total_us = std::mem::take(&mut self.total_us);
        let max_us = std::mem::take(&mut self.max_us);
        let evaluating_total_us = std::mem::take(&mut self.evaluating_total_us);
        let evaluating_max_us = std::mem::take(&mut self.evaluating_max_us);
        if frames == 0 {
            return None;
        }
        let mean = |total: u64| total as f32 / frames as f32 / 1000.0;
        Some(FrameCost {
            output: self.config.name.clone(),
            kind: format!("{:?}", self.config.kind).to_lowercase(),
            mean_ms: mean(total_us),
            max_ms: max_us as f32 / 1000.0,
            evaluating_mean_ms: mean(evaluating_total_us),
            evaluating_max_ms: evaluating_max_us as f32 / 1000.0,
            frames,
            window_ms: elapsed.as_millis() as u32,
        })
    }

    /// Put the next frame where this connector's own rate says it goes.
    fn schedule(&mut self, from: std::time::Instant, moving: bool) {
        let frames = self.plugin.frames();
        let after = if moving { frames.while_moving } else { frames.when_settled };
        self.next_frame = after.map(|period| from + period);
    }
}

/// Owns the output plugins and feeds them.
pub struct OutputManager {
    node_id: NodeId,
    engine: EngineHandle,
    running: HashMap<Uuid, Running>,
    rx: mpsc::Receiver<OutputCommand>,
    devices: Devices,
    /// Every configured output, this station's or not: coverage is a property
    /// of the show, not of what happens to run here.
    configured: Vec<OutputConfig>,
    /// The patch as it bears on coverage — id, name and address of each fixture —
    /// so the answer is recomputed when a fixture moves, not forty times a second.
    addressed: Vec<(Uuid, String, pult_schema::types::fixture::FixtureAddress)>,
    coverage: Option<OutputCoverage>,
    /// What each connector's frames cost over the window just closed, for the station
    /// reporter to publish. A watch, so nothing on the frame path takes a lock and
    /// nothing is written to replicated state per frame.
    frame_costs: watch::Sender<Vec<FrameCost>>,
    /// The last thing the engine said was driving the rig.
    ///
    /// Held across frames, and that is the whole of what this change did here: a
    /// connector draws its own frames out of this rather than waiting to be handed
    /// values, so the engine says nothing at all between one cue and the next.
    patch: Option<Patch>,
}

impl OutputManager {
    pub fn new(
        node_id: NodeId,
        engine: EngineHandle,
        devices: Devices,
    ) -> (Self, OutputHandle, watch::Receiver<Vec<FrameCost>>) {
        let (tx, rx) = mpsc::channel(4);
        let (frame_costs, costs_rx) = watch::channel(Vec::new());
        (
            Self {
                node_id,
                engine,
                running: HashMap::new(),
                rx,
                devices,
                configured: Vec::new(),
                addressed: Vec::new(),
                coverage: None,
                patch: None,
                frame_costs,
            },
            OutputHandle(tx),
            costs_rx,
        )
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
            // A connector with no rate of its own, and a station with nothing patched,
            // both sleep here until something speaks to them. Far enough off that the
            // arithmetic never overflows, near enough that it is obviously a placeholder.
            let next_frame = self
                .next_frame_at()
                .unwrap_or_else(|| std::time::Instant::now() + std::time::Duration::from_secs(3600));

            tokio::select! {
                biased;
                cmd = self.rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        OutputCommand::Stop => break,
                        OutputCommand::Configure(outputs) => self.reconcile(outputs).await,
                        OutputCommand::Patch { fixtures, fixture_types, programmer, changed } => {
                            self.take_patch(fixtures, fixture_types, programmer, changed).await;
                        }
                    }
                }
                _ = report.tick() => {
                    self.measure_rates();
                    self.publish_status().await;
                }
                _ = tokio::time::sleep_until(next_frame.into()) => {
                    self.draw_due_frames().await;
                }
            }
        }
        info!("[output] stopped");
    }

    /// The soonest any connector wants a frame.
    fn next_frame_at(&self) -> Option<std::time::Instant> {
        self.running.values().filter_map(|output| output.next_frame).min()
    }

    /// Take a new picture of what is driving the rig, and put it on every wire at once.
    ///
    /// The show changed, so every connector hears about it immediately whatever its own
    /// rate is — a re-addressed fixture changes the wire without changing a single
    /// level, and a node that can run a fade itself wants telling the moment the fade
    /// exists rather than on the next frame.
    async fn take_patch(
        &mut self,
        fixtures: Vec<Fixture>,
        fixture_types: Vec<FixtureType>,
        programmer: Vec<ProgrammerValue>,
        changed: Vec<Uuid>,
    ) {
        let addressed: Vec<_> =
            fixtures.iter().map(|f| (f.id, f.name.clone(), f.address.clone())).collect();
        if addressed != self.addressed {
            self.addressed = addressed;
            self.publish_coverage(&fixtures).await;
        }
        self.patch = Some(Patch::new(fixtures, fixture_types, programmer));
        self.draw(changed).await;
    }

    /// Draw the connectors whose own frame has come due.
    async fn draw_due_frames(&mut self) {
        let now = std::time::Instant::now();
        let due: Vec<Uuid> = self
            .running
            .iter()
            .filter(|(_, output)| output.next_frame.is_some_and(|at| at <= now))
            .map(|(id, _)| *id)
            .collect();
        if due.is_empty() {
            return;
        }
        self.draw_these(&due, Vec::new()).await;
    }

    /// One frame on every running connector.
    async fn draw(&mut self, changed: Vec<Uuid>) {
        let all: Vec<Uuid> = self.running.keys().copied().collect();
        self.draw_these(&all, changed).await;
    }

    /// Work out what the patch is doing now and hand it to the named connectors.
    ///
    /// Sequentially, and one plugin's failure does not stop the rest: an unplugged
    /// Art-Net interface must not silence sACN.
    async fn draw_these(&mut self, which: &[Uuid], changed: Vec<Uuid>) {
        let Some(patch) = &self.patch else { return };
        // One reading of the clock for the whole frame. Two would put two fixtures of
        // the same cue a fraction of a cycle apart, which is exactly the disagreement
        // this change spent its effort removing.
        let now_ms = pult_schema::types::sequence::now_ms();
        let moving = patch.is_moving(now_ms);
        let at = std::time::Instant::now();

        for id in which {
            let Some(output) = self.running.get_mut(id) else { continue };
            let began = std::time::Instant::now();
            match output.plugin.send(patch, &changed, now_ms).await {
                Ok(frame) => {
                    output.measure(began.elapsed(), frame);
                    output.status.last_send = Some(Utc::now());
                    output.sends_since_report += 1;
                }
                Err(e) => {
                    warn!("[output] {}: {e}", output.config.name);
                    output.status.error_count += 1;
                    output.status.last_error = Some(e.to_string());
                }
            }
            output.schedule(at, moving);
        }
    }

    /// Bring the running plugins in line with the configured outputs.
    ///
    /// Rebuilt only where the configuration actually changed, so renaming one output
    /// does not drop and re-open every socket in the rig — which for Art-Net would
    /// reset the dedup cache and put a redundant frame on the wire for a label edit.
    async fn reconcile(&mut self, outputs: Vec<OutputConfig>) {
        if outputs != self.configured {
            self.configured = outputs.clone();
            let fixtures = self.fixtures_as_addressed();
            self.publish_coverage(&fixtures).await;
        }
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

    /// Close the measurement window and open a new one. Timer only: doing this
    /// whenever a status happens to be published would divide the count by whatever
    /// gap preceded it, and report a rate in the thousands after a reconfigure.
    ///
    /// One window closes both figures — the frame rate the Outputs panel shows and
    /// the frame cost the station row publishes — because they are two readings of
    /// the same frames and reporting them over different windows would let them
    /// disagree about how many there were.
    fn measure_rates(&mut self) {
        let mut costs = Vec::new();
        for output in self.running.values_mut() {
            let elapsed = output.reported_at.elapsed();
            if elapsed.as_secs_f32() > 0.0 {
                output.status.frames_per_second =
                    output.sends_since_report as f32 / elapsed.as_secs_f32();
            }
            output.sends_since_report = 0;
            output.reported_at = std::time::Instant::now();
            costs.extend(output.close_window(elapsed));
        }
        // Sorted, so two stations reading the same rig list their connectors the same
        // way and a panel does not reshuffle its rows every couple of seconds.
        costs.sort_by(|a, b| a.output.cmp(&b.output));
        let _ = self.frame_costs.send(costs);
    }

    /// The last patch, as far as coverage cares: enough of a fixture to place it.
    fn fixtures_as_addressed(&self) -> Vec<Fixture> {
        self.addressed
            .iter()
            .map(|(id, name, address)| Fixture {
                id: *id,
                name: name.clone(),
                fixture_type_id: Uuid::nil(),
                address: address.clone(),
                position: None,
                parent: None,
                layer: None,
                class: None,
                focus: None,
                fixture_number: None,
                unit_number: None,
                sensed_values: Default::default(),
                live_effects: Default::default(),
                live_fades: Default::default(),
                home_values: Default::default(),
            })
            .collect()
    }

    /// Say which fixtures no output reaches, when that changes.
    async fn publish_coverage(&mut self, fixtures: &[Fixture]) {
        let coverage = OutputCoverage::of(&self.configured, fixtures);
        if self.coverage.as_ref() == Some(&coverage) {
            return;
        }
        for gap in &coverage.gaps {
            match gap.universe {
                Some(universe) => warn!(
                    "[output] nothing carries universe {universe}: {} unreached",
                    gap.fixture_names.join(", ")
                ),
                None => warn!(
                    "[output] no OpenHaunt output: {} not driven",
                    gap.fixture_names.join(", ")
                ),
            }
        }
        if let Ok(json) = serde_json::to_value(&coverage) {
            let path = vec![PathSegment::Key("output_coverage".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
        self.coverage = Some(coverage);
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
