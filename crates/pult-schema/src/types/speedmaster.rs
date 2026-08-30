//! Tempo, shared.
//!
//! An effect can carry its own rate in Hz, and for one chase that is the simplest
//! thing that works. What it cannot do is let an operator take hold of a whole show's
//! tempo at once, which is what a speed master is for: effects name the master
//! instead of a number, and tapping it moves all of them together.
//!
//! # Why the anchor is a stored field and not each station's guess
//!
//! Beat phase has to be the same on every console, so it cannot be inferred from when
//! each of them happened to hear about a tap. [`SpeedMaster::t0`] is the console time
//! of the last "one", written by whoever tapped, replicated like any other SYNCED
//! field. Every tap and every bpm edit rewrites it, which is what makes a tempo change
//! a bounded step in phase rather than a drift: the new rate and the anchor it is
//! measured from arrive together.
//!
//! Mixed lifecycle, like [`crate::types::cue::Cue`]. Name, tempo and multiplier are
//! part of the show and persist; whether it is running, and where its beat is, are
//! not. A `t0` of 0 after a reload is a defined anchor rather than a missing one, so
//! a stored cue naming a master renders deterministically from the first tick.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// One tempo several effects can follow.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "speed_masters")]
pub struct SpeedMaster {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub bpm: f32,
    /// Halve or double without losing the tapped tempo underneath.
    #[pult(lifecycle = PERSISTED)]
    pub multiplier: f32,
    #[pult(lifecycle = SYNCED)]
    pub running: bool,
    /// Console unix ms of the last "one". Rewritten by every tap and every bpm edit.
    #[pult(lifecycle = SYNCED)]
    pub t0: u64,
}

/// What an effect gets when it names a master that is not there.
///
/// A cue can outlive the master it was stored against — deleted, or loaded from a
/// showfile written elsewhere. Rendering nothing would make the fixture stick at
/// whatever it last held, which looks like a fault; rendering at a defined default
/// keeps output deterministic and visibly wrong in a way an operator can fix.
pub const FALLBACK_BPM: f32 = 120.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_master_round_trips_with_its_anchor() {
        let master = SpeedMaster {
            id: Uuid::nil(),
            name: "Chases".into(),
            bpm: 128.0,
            multiplier: 0.5,
            running: true,
            t0: 1_756_550_400_123,
        };

        let back: SpeedMaster =
            serde_json::from_value(serde_json::to_value(&master).unwrap()).unwrap();
        assert_eq!(back.bpm, 128.0);
        assert_eq!(back.multiplier, 0.5);
        assert!(back.running);
        assert_eq!(back.t0, 1_756_550_400_123);
    }

    /// `running` and `t0` are SYNCED, so creating a master asks only for what the
    /// show keeps. A reload brings the other two back defaulted, and a `t0` of zero is
    /// a defined anchor rather than a missing one, which is what keeps a cue naming
    /// this master deterministic from the first tick after a load.
    #[test]
    fn creating_a_master_asks_only_for_what_the_show_keeps() {
        let create = SpeedMasterCreate { name: "Chases".into(), bpm: 128.0, multiplier: 1.0 };
        let fields = serde_json::to_value(&create).unwrap();
        let fields = fields.as_object().unwrap();

        assert_eq!(fields.len(), 3);
        assert!(fields.contains_key("bpm"));
        assert!(!fields.contains_key("running"), "SYNCED, so not asked for");
        assert!(!fields.contains_key("t0"), "SYNCED, so not asked for");
    }
}
