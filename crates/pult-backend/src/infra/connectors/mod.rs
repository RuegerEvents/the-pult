//! Output: getting fixture state onto a wire.
//!
//! The spec calls for output plugins that translate high-level data into whatever
//! protocol a fixture speaks, with network-based communication preferred over
//! DMX-centric workflows. So the shape here is a plugin trait and a manager, and
//! Art-Net is one implementation of that trait rather than the centre of it.

use std::collections::HashMap;

use pult_schema::types::fixture::{Fixture, FixtureType};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

pub mod artnet;
pub mod dmx;

use dmx::Patch;

// ── OutputPlugin ──────────────────────────────────────────────────────────────

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
    fn send(
        &mut self,
        patch: &Patch,
        changed: &[Uuid],
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

// ── OutputManager ─────────────────────────────────────────────────────────────

pub enum OutputCommand {
    /// The engine's view of the patch, pushed whenever fixture output changes.
    Patch { fixtures: Vec<Fixture>, fixture_types: Vec<FixtureType>, changed: Vec<Uuid> },
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
}

/// Owns the output plugins and feeds them.
pub struct OutputManager<P: OutputPlugin> {
    plugin: P,
    rx: mpsc::Receiver<OutputCommand>,
}

impl<P: OutputPlugin> OutputManager<P> {
    pub fn new(plugin: P) -> (Self, OutputHandle) {
        let (tx, rx) = mpsc::channel(4);
        (Self { plugin, rx }, OutputHandle(tx))
    }

    pub async fn run(mut self) {
        info!("[output] {} started", self.plugin.name());
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                OutputCommand::Stop => break,
                OutputCommand::Patch { fixtures, fixture_types, changed } => {
                    let patch = Patch {
                        fixtures,
                        fixture_types: fixture_types.into_iter().map(|t| (t.id, t)).collect(),
                    };
                    if let Err(e) = self.plugin.send(&patch, &changed).await {
                        warn!("[output] {}: {e}", self.plugin.name());
                    }
                }
            }
        }
        info!("[output] {} stopped", self.plugin.name());
    }
}

/// Index fixture types by id, for callers building a [`Patch`] by hand.
pub fn index_types(types: Vec<FixtureType>) -> HashMap<Uuid, FixtureType> {
    types.into_iter().map(|t| (t.id, t)).collect()
}
