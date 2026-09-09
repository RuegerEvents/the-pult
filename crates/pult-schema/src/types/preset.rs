//! Presets: a look, by reference.
//!
//! Until now a cue always copied literals, and the substitute was the programmer's
//! `locked` parking — which `programmer.rs` says outright. That works for building
//! one look twice and not at all for the thing palettes exist to do: change *warm* in
//! one place and have the eighty cues that use it change with it.
//!
//! # Any mix, one pool
//!
//! Other desks separate intensity, position, colour and beam palettes. This one keeps
//! a single flat pool of presets whose values may be any mixture — a preset *is* a
//! look, and a look is usually a position and a colour together. The group tags an
//! operator filters by (I / P / C / B / O) are **derived from the keys** it holds and
//! never stored, so a preset that grows a colour is a colour preset from that moment
//! and nobody has to reclassify it.
//!
//! # Reference first, literal beside it
//!
//! A capture that names a preset also keeps `value`: the copy taken at the moment it
//! was stored. The reference wins wherever it resolves, and the literal is what plays
//! when the preset has been deleted or does not name that fixture. Which means
//! **deleting a preset cascades nothing** — every cue that used it goes on running
//! exactly as it last did, the UI says "preset missing", and Ctrl-Z of the delete
//! restores every link at once because no link was ever broken.
//!
//! The literal is *never rewritten by a preset edit*. It is a record of what the cue
//! was stored as, not a cache of what the preset currently says; rewriting it would
//! make a fallback that silently tracks the thing it is a fallback for.
//!
//! # Per fixture, not per type
//!
//! A `PresetValue` names a fixture. Type-wide presets — "every Mac Aura's warm" — are
//! a real and separate feature: they need a rule for a fixture the preset has never
//! seen, and inventing one here would be inventing it in the dark. See *What is not
//! done* in the roadmap.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::fixture::{parameter_key, ParameterKind, ParameterValue};
use crate::PultSchema;

/// One parameter of one fixture, as a preset holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PresetValue {
    pub fixture_id: Uuid,
    pub parameter_kind: ParameterKind,
    pub value: ParameterValue,
}

/// A named look that cues and the programmer can point at.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "presets")]
pub struct Preset {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// What it says, per fixture and parameter. A preset that names no fixture at all
    /// is legal and does nothing — which is what one being built looks like.
    #[pult(lifecycle = PERSISTED)]
    pub values: Vec<PresetValue>,
}

impl Preset {
    /// What this preset says about one parameter of one fixture, if anything.
    ///
    /// `None` is the ordinary answer for most of the rig: a preset knows the fixtures
    /// it was made from, and applying it to a selection is an intersection rather than
    /// a promise.
    pub fn value_for(&self, fixture: Uuid, kind: &ParameterKind) -> Option<&ParameterValue> {
        let key = parameter_key(kind);
        self.values
            .iter()
            .find(|v| v.fixture_id == fixture && parameter_key(&v.parameter_kind) == key)
            .map(|v| &v.value)
    }

    /// Every fixture this preset says anything about, in the order it says it.
    pub fn fixtures(&self) -> Vec<Uuid> {
        let mut out: Vec<Uuid> = Vec::new();
        for value in &self.values {
            if !out.contains(&value.fixture_id) {
                out.push(value.fixture_id);
            }
        }
        out
    }
}

/// The presets by id, for anything resolving a capture.
///
/// A borrowed map rather than a clone: playback builds one per pass over a collection
/// that changes when somebody edits a palette, which on a show night is never.
pub fn preset_index(presets: &[Preset]) -> HashMap<Uuid, &Preset> {
    presets.iter().map(|preset| (preset.id, preset)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset() -> Preset {
        Preset {
            id: Uuid::from_u128(1),
            name: "Warm".into(),
            values: vec![
                PresetValue {
                    fixture_id: Uuid::from_u128(10),
                    parameter_kind: ParameterKind::Intensity,
                    value: ParameterValue::Float(0.8),
                },
                PresetValue {
                    fixture_id: Uuid::from_u128(10),
                    parameter_kind: ParameterKind::Pan,
                    value: ParameterValue::Float(0.25),
                },
                PresetValue {
                    fixture_id: Uuid::from_u128(11),
                    parameter_kind: ParameterKind::Intensity,
                    value: ParameterValue::Float(0.4),
                },
            ],
        }
    }

    #[test]
    fn a_preset_answers_for_the_fixtures_it_knows_and_no_others() {
        let preset = preset();
        assert_eq!(
            preset.value_for(Uuid::from_u128(10), &ParameterKind::Intensity),
            Some(&ParameterValue::Float(0.8))
        );
        assert_eq!(preset.value_for(Uuid::from_u128(12), &ParameterKind::Intensity), None);
        assert_eq!(preset.value_for(Uuid::from_u128(11), &ParameterKind::Pan), None);
    }

    /// An indexed kind is matched by its *key*, the way everything else that reads
    /// `live_fades` or `home_values` matches one.
    #[test]
    fn an_indexed_kind_matches_by_key() {
        let preset = Preset {
            id: Uuid::from_u128(2),
            name: "Break-up".into(),
            values: vec![PresetValue {
                fixture_id: Uuid::from_u128(10),
                parameter_kind: ParameterKind::Gobo(1),
                value: ParameterValue::Int(4),
            }],
        };
        assert_eq!(
            preset.value_for(Uuid::from_u128(10), &ParameterKind::Gobo(1)),
            Some(&ParameterValue::Int(4))
        );
        assert_eq!(preset.value_for(Uuid::from_u128(10), &ParameterKind::Gobo(2)), None);
    }

    #[test]
    fn the_fixtures_are_listed_once_each_in_the_order_they_appear() {
        assert_eq!(preset().fixtures(), vec![Uuid::from_u128(10), Uuid::from_u128(11)]);
    }
}
