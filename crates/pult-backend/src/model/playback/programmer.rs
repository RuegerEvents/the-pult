//! The programmer, laid over playback.
//!
//! This is the first explicit priority rule in the system, and it is the standard
//! one: **for every parameter the programmer holds, the programmer wins.** A cue
//! taken while an operator has hold of a fader does not move that fader.
//!
//! What it does *not* do is stop the cue. Fades and effects keep running underneath,
//! and they are still published, so a value released lands on wherever playback has
//! got to by then rather than snapping back to where the cue was when the operator
//! first grabbed it. That is the difference between an override and a freeze, and it
//! is the reason a look can be built during a fade.
//!
//! There is no state here at all, and that is the change this module records. The
//! overlay used to remember which keys it had taken and what was showing underneath
//! each of them, because releasing had to put a value back into a map. Nothing stores
//! a value now: releasing is simply the entry going away, and what shows through is
//! whatever the layers beneath already say. So what is left is a function of the
//! `programmer_values` collection, which every station has replicated anyway.

use std::collections::HashMap;

use pult_schema::types::{
    effect::{EffectSource, RunningEffect},
    fixture::ParameterValue,
    programmer::ProgrammerValue,
};
use uuid::Uuid;

use super::{effects, parameter_key, ShowView};

/// A fixture and one of its parameter keys: what the programmer holds one of.
pub type Key = (Uuid, String);

/// What the programmer puts on a key it has taken.
///
/// An entry asserts one or the other, never both: grabbing a fader and putting a sine
/// on it are the same act of taking hold of one parameter, and the difference is only
/// in whether the answer changes on its own from one moment to the next.
#[derive(Debug, Clone)]
pub enum Held {
    Value(ParameterValue),
    Effect(RunningEffect),
}

/// What the programmer puts on one parameter, if it holds it.
///
/// Two entries for the same parameter should not exist — the frontend derives an
/// entry's id from the fixture and the key precisely so that they cannot — but a peer
/// that disagrees must not make the output flicker between them, so the last one read
/// wins and stays won.
pub fn held_by_the_programmer(view: &ShowView<'_>, at: &Key) -> Option<Held> {
    let entry = view
        .programmer
        .iter()
        .filter(|entry| entry.fixture_id == at.0 && parameter_key(&entry.parameter_kind) == at.1)
        .next_back()?;
    Some(hold(entry, view.speed_masters))
}

/// Every parameter the programmer has hold of, whatever it is putting on it.
pub fn held_keys(view: &ShowView<'_>) -> std::collections::HashSet<Key> {
    view.programmer
        .iter()
        .map(|entry| (entry.fixture_id, parameter_key(&entry.parameter_kind)))
        .collect()
}

/// Every shape the programmer is holding, resolved.
///
/// A key held as a plain value is deliberately absent rather than listed as nothing:
/// absence is what tells a node to stop tracing a shape and take values again — and
/// the plain value itself needs no publishing, since it is already replicated as the
/// programmer entry that carries it.
pub fn held_effects(view: &ShowView<'_>) -> HashMap<Key, RunningEffect> {
    view.programmer
        .iter()
        .filter_map(|entry| {
            let spec = entry.effect.as_ref()?;
            Some((
                (entry.fixture_id, parameter_key(&entry.parameter_kind)),
                // A programmer effect carries its own anchor, set when the operator
                // applied it. An entry that arrived without one falls back to the
                // epoch rather than to now: any fixed instant will do, but it has to
                // be fixed. Anchoring on the current moment would move the anchor
                // forward exactly as fast as time passes, and the effect would sit
                // still for ever.
                //
                // Resolved here rather than stored resolved, so that editing the speed
                // master an entry follows changes what it does on the very next pass
                // without anything having to go looking for the entries that named it.
                effects::resolve(spec, view.speed_masters, 0, EffectSource::Programmer),
            ))
        })
        .collect()
}

fn hold(entry: &ProgrammerValue, masters: &[pult_schema::types::speedmaster::SpeedMaster]) -> Held {
    match &entry.effect {
        Some(spec) => {
            Held::Effect(effects::resolve(spec, masters, 0, EffectSource::Programmer))
        }
        None => Held::Value(entry.value.clone()),
    }
}
