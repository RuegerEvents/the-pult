//! Four shows the console can make for itself.
//!
//! A demo used to be a Node script driving a running station over the WebSocket, and
//! for the *measurement* rigs it still is — `scripts/demo-seed.mjs` is deliberately
//! outside the console, because an instrument that goes through the public API
//! measures what an operator would feel. What that arrangement could not be was a
//! button on a welcome screen: an operator opening the console for the first time
//! has no Node, no repository and no terminal.
//!
//! So these are in Rust and in the binary. They are still *writes*, through
//! `EngineHandle` like anything else — not rows inserted underneath the engine —
//! which keeps validation, the oplog and the seeded operator exactly as they are for
//! anything a person does. What they skip is the network, not the model.
//!
//! # Which four
//!
//! Each one exists to show something the others cannot.
//!
//! - **Haunt** is the hand-made small demo, small enough to read: five fixtures, a
//!   speed master, three cues with a colour effect on the last, and two flows.
//! - **Theatre** is conventionals and a cue stack — no movers, split fade times,
//!   groups by system. What most of the world actually runs.
//! - **Club** is movers, washes and strobes with effects left running, so the rig
//!   view has something moving in it and the speed masters do something.
//! - **Festival** is scale: two hundred heads on trusses that are themselves scene
//!   objects, in layers, with six sequences each holding a slice of the rig.
//!
//! None of them needs an asset, so a demo opens with no download and no network.

use anyhow::Result;
use pult_schema::{lifecycle::Lifecycle, path::PathSegment, types::Fixture};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::engine::EngineHandle;

mod club;
mod festival;
mod haunt;
mod kit;
mod theatre;

/// Which demo. The names are what `--demo` takes and what the welcome screen's cards
/// carry, so the two cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Demo {
    Haunt,
    Theatre,
    Club,
    Festival,
}

/// Every demo, in the order the welcome screen offers them: smallest first, because
/// the first one somebody opens should be the one they can read.
pub const ALL: &[Demo] = &[Demo::Haunt, Demo::Theatre, Demo::Club, Demo::Festival];

impl Demo {
    /// The id `--demo` takes and `show.new` carries.
    pub fn id(self) -> &'static str {
        match self {
            Demo::Haunt => "haunt",
            Demo::Theatre => "theatre",
            Demo::Club => "club",
            Demo::Festival => "festival",
        }
    }

    pub fn parse(text: &str) -> Option<Demo> {
        ALL.iter().copied().find(|demo| demo.id() == text.trim().to_ascii_lowercase())
    }

    pub fn title(self) -> &'static str {
        match self {
            Demo::Haunt => "Haunt",
            Demo::Theatre => "Theatre",
            Demo::Club => "Club",
            Demo::Festival => "Festival",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Demo::Haunt => {
                "Five lights, three cues and two flows. Small enough to read the \
                 whole of, and the one to open first."
            }
            Demo::Theatre => {
                "Forty conventionals in front, back, side and cyc systems, with a \
                 twenty-cue stack and split fade times."
            }
            Demo::Club => {
                "Movers, LED washes and strobes on two trusses, with effects \
                 running against two speed masters."
            }
            Demo::Festival => {
                "Two hundred heads on trusses in layers, six sequences deep. The \
                 one to open if you want to see the console work."
            }
        }
    }
}

impl std::str::FromStr for Demo {
    type Err = String;
    fn from_str(text: &str) -> Result<Demo, String> {
        Demo::parse(text).ok_or_else(|| {
            format!("no such demo: {text}. Try one of: {}", ALL.iter().map(|d| d.id()).collect::<Vec<_>>().join(", "))
        })
    }
}

/// Put a demo into a show that has nothing in it.
///
/// Refuses a show that already has fixtures, which is what makes this safe to run at
/// every start: `--demo` on a console that has been programmed since must not add a
/// second rig on top of somebody's work.
pub async fn seed(engine: &EngineHandle, demo: Demo) -> Result<()> {
    let seeder = Seeder { engine: engine.clone() };
    if !seeder.rig_is_empty().await {
        info!("[demo] this show already has a rig; leaving it alone");
        return Ok(());
    }
    info!("[demo] seeding {}", demo.id());
    match demo {
        Demo::Haunt => haunt::seed(&seeder).await?,
        Demo::Theatre => theatre::seed(&seeder).await?,
        Demo::Club => club::seed(&seeder).await?,
        Demo::Festival => festival::seed(&seeder).await?,
    }
    let fixtures = seeder.fixtures().await.len();
    info!("[demo] {} seeded: {fixtures} fixtures", demo.id());
    Ok(())
}

/// The few writes a demo makes, said once.
///
/// Not a builder and not a transaction: every call here is an ordinary engine write,
/// so a demo that fails half way leaves a half-seeded show rather than nothing —
/// which is the right failure for something an operator can delete and make again.
pub struct Seeder {
    engine: EngineHandle,
}

impl Seeder {
    /// Create one row in a collection.
    pub async fn create<T: Serialize>(&self, table: &str, value: &T) -> Result<()> {
        let path = vec![
            PathSegment::Key(table.into()),
            PathSegment::Key("__create".into()),
        ];
        self.engine
            .set(path, Lifecycle::Persisted, serde_json::to_value(value)?)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Set one field of one row, which is how a demo starts a sequence running.
    pub async fn set(
        &self,
        path: Vec<PathSegment>,
        value: serde_json::Value,
        lifecycle: Lifecycle,
    ) -> Result<()> {
        self.engine
            .set(path, lifecycle, value)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Rename the show, which every demo does: the bundle's name is whatever folder
    /// somebody chose, and the show's own name is what the demo is called.
    ///
    /// A show with no row at all gets one. The engine seeds it at load from the
    /// bundle's manifest, so a console always has one by the time a demo runs — but a
    /// station with no bundle open has none, and a demo that failed on its first
    /// write there would be a demo nothing could test.
    pub async fn name_the_show(&self, name: &str) -> Result<()> {
        use pult_schema::types::Show;

        let existing = self.engine.get(vec![PathSegment::Key("show".into())]).await.ok();
        let has_one = existing.is_some_and(|value| !value.is_null());
        if has_one {
            return self
                .set(
                    vec![PathSegment::Key("show".into()), PathSegment::Key("name".into())],
                    serde_json::json!(name),
                    Lifecycle::Persisted,
                )
                .await;
        }

        let prefs = crate::infra::preferences::load();
        self.set(
            vec![PathSegment::Key("show".into())],
            serde_json::to_value(Show {
                id: id(),
                name: name.to_string(),
                created_at: chrono::Utc::now(),
                editing_cue: None,
                history_depth: prefs.history_depth,
                home_fade_ms: prefs.home_fade_ms,
                haze_density: prefs.haze_density,
                haze_turbulence: prefs.haze_turbulence,
            })?,
            Lifecycle::Persisted,
        )
        .await
    }

    pub async fn fixtures(&self) -> Vec<Fixture> {
        self.engine
            .get(vec![PathSegment::Key("fixtures".into())])
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    async fn rig_is_empty(&self) -> bool {
        self.fixtures().await.is_empty()
    }
}

/// The moment a demo is seeded, for a speed master's "one".
pub(crate) fn now_ms() -> u64 {
    pult_schema::types::sequence::now_ms()
}

/// A new id. Named, because a demo makes a great many and `Uuid::new_v4()` at every
/// call site reads as though the choice mattered.
pub(crate) fn id() -> Uuid {
    Uuid::new_v4()
}

#[cfg(test)]
mod tests;
