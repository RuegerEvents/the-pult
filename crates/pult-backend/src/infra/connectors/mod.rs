//! Output: getting fixture state onto a wire.
//!
//! The spec calls for output plugins that translate high-level data into whatever
//! protocol a fixture speaks, with network-based communication preferred over
//! DMX-centric workflows. So the shape here is a plugin trait and a manager, and
//! Art-Net is one implementation of that trait rather than the centre of it.

use std::future::Future;
use std::pin::Pin;

use pult_schema::types::fixture::{Fixture, FixtureType};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

pub mod artnet;
pub mod dmx;
pub mod openhaunt;
pub mod sacn;

use dmx::Patch;

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
pub struct OutputManager {
    plugins: Vec<Box<dyn OutputPlugin>>,
    rx: mpsc::Receiver<OutputCommand>,
}

impl OutputManager {
    pub fn new(plugins: Vec<Box<dyn OutputPlugin>>) -> (Self, OutputHandle) {
        let (tx, rx) = mpsc::channel(4);
        (Self { plugins, rx }, OutputHandle(tx))
    }

    pub async fn run(mut self) {
        let names: Vec<&str> = self.plugins.iter().map(|p| p.name()).collect();
        info!("[output] started: {}", names.join(", "));

        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                OutputCommand::Stop => break,
                OutputCommand::Patch { fixtures, fixture_types, changed } => {
                    let patch = Patch {
                        fixtures,
                        fixture_types: fixture_types.into_iter().map(|t| (t.id, t)).collect(),
                    };
                    // Sequentially, and one plugin's failure does not stop the rest:
                    // an unplugged Art-Net interface must not silence sACN.
                    for plugin in &mut self.plugins {
                        if let Err(e) = plugin.send(&patch, &changed).await {
                            warn!("[output] {}: {e}", plugin.name());
                        }
                    }
                }
            }
        }
        info!("[output] stopped");
    }
}
