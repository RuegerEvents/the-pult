//! The programmer, laid over playback.
//!
//! This is the first explicit priority rule in the system, and it is the standard
//! one: **for every parameter the programmer holds, the programmer wins.** A cue
//! taken while an operator has hold of a fader does not move that fader.
//!
//! What it does *not* do is stop the cue. Fades keep running underneath, and
//! [`Overlay::beneath`] is kept current as they do — so a value released or stored
//! lands where playback has got to by then, rather than snapping back to wherever
//! the cue was when the operator first grabbed it. That is the difference between an
//! override and a freeze, and it is the reason a look can be built during a fade.
//!
//! Anything else that writes a live value — a flow action, an input off a device —
//! is not fought with. It writes, the overlay notices on the next tick, remembers
//! what it wrote as the new value underneath, and covers it again. So the programmer
//! stays on top without needing every other writer to know it exists.

use std::collections::HashMap;

use pult_schema::types::{
    effect::{EffectSource, RunningEffect},
    fixture::ParameterValue,
    programmer::ProgrammerValue,
};
use uuid::Uuid;

use super::{effects, parameter_key, zero_like, Changes, ShowView};

/// A fixture and one of its parameter keys: what the programmer holds one of.
pub type Key = (Uuid, String);

/// What the programmer puts on a key it has taken.
///
/// An entry asserts one or the other, never both: grabbing a fader and putting a sine
/// on it are the same act of taking hold of one parameter, and the difference is only
/// in whether the answer changes on its own from one tick to the next.
#[derive(Debug, Clone)]
pub enum Held {
    Value(ParameterValue),
    Effect(RunningEffect),
}

impl Held {
    fn value_at(&self, wall_ms: u64) -> ParameterValue {
        match self {
            Held::Value(value) => value.clone(),
            Held::Effect(effect) => effects::value_at(effect, wall_ms),
        }
    }
}

/// What the programmer is asserting, and what it is asserting it over.
#[derive(Debug, Default)]
pub struct Overlay {
    /// What the programmer currently puts on each key it has taken.
    held: HashMap<Key, Held>,
    /// What each held key would be showing if the programmer let go right now.
    /// Fades and cue effects write here while they run, so this follows the cue rather
    /// than freezing.
    beneath: HashMap<Key, ParameterValue>,
}

impl Overlay {
    /// True while the programmer holds anything.
    ///
    /// The engine only ticks playback when the show has changed or there is work
    /// outstanding, and holding a value *is* outstanding work: a flow action or a
    /// device input writing the same key does not bump the show's version, so
    /// without this the programmer would quietly lose the key it is holding.
    pub fn has_work(&self) -> bool {
        !self.held.is_empty()
    }

    /// The value the programmer puts on this key right now, if it holds it.
    ///
    /// Rendered rather than looked up, because a held effect is a different value on
    /// every tick.
    pub fn covering(&self, fixture_id: Uuid, key: &str, wall_ms: u64) -> Option<ParameterValue> {
        self.held.get(&(fixture_id, key.to_string())).map(|held| held.value_at(wall_ms))
    }

    /// Whether the programmer has this key at all, whatever it is putting on it.
    pub fn holds(&self, fixture_id: Uuid, key: &str) -> bool {
        self.held.contains_key(&(fixture_id, key.to_string()))
    }

    /// The effects the programmer is holding, for the plugins that can offload one.
    ///
    /// A key held as a plain value is deliberately absent rather than listed as
    /// nothing: absence is what tells a node to stop tracing a shape and take values
    /// again.
    pub fn held_effects(&self) -> impl Iterator<Item = (&Key, &RunningEffect)> {
        self.held.iter().filter_map(|(key, held)| match held {
            Held::Effect(effect) => Some((key, effect)),
            Held::Value(_) => None,
        })
    }

    /// Record where a fade has got to under a key the programmer is holding.
    pub fn note_beneath(&mut self, fixture_id: Uuid, key: &str, value: ParameterValue) {
        self.beneath.insert((fixture_id, key.to_string()), value);
    }

    /// Take the keys the programmer wants, give back the ones it has let go of.
    pub fn assert(&mut self, view: &ShowView<'_>, wall_ms: u64, changes: &mut Changes) {
        let wanted = wanted(view.programmer, view.speed_masters);
        if wanted.is_empty() && self.held.is_empty() {
            return;
        }

        self.release(&wanted, changes);

        for (key, want) in wanted {
            let value = want.value_at(wall_ms);
            match self.held.get(&key) {
                // Newly taken: remember what was showing, so releasing can put it back.
                None => {
                    let under = view
                        .live_value(key.0, &key.1)
                        .unwrap_or_else(|| zero_like(&value));
                    self.beneath.insert(key.clone(), under);
                }
                // Already held. If the live value is not what this overlay put there,
                // somebody else wrote it — a flow action, an input off a device. Take
                // the key back, and treat what they wrote as the new value underneath.
                Some(asserted) => {
                    if let Some(live) = view.live_value(key.0, &key.1) {
                        if live != asserted.value_at(wall_ms) {
                            self.beneath.insert(key.clone(), live);
                        }
                    }
                }
            }
            self.held.insert(key.clone(), want);
            changes.entry(key.0).or_default().insert(key.1, value);
        }
    }

    /// Put back what was under every key the programmer no longer wants.
    fn release(&mut self, wanted: &HashMap<Key, Held>, changes: &mut Changes) {
        let released: Vec<Key> =
            self.held.keys().filter(|key| !wanted.contains_key(*key)).cloned().collect();
        for key in released {
            self.held.remove(&key);
            let Some(under) = self.beneath.remove(&key) else { continue };
            changes.entry(key.0).or_default().insert(key.1, under);
        }
    }
}

/// What the programmer entries add up to, one hold per key.
///
/// Two entries for the same parameter should not exist — the frontend derives an
/// entry's id from the fixture and the key precisely so that they cannot — but a
/// peer that disagrees must not make the output flicker between them, so the last
/// one read wins and stays won.
///
/// An effect is resolved here rather than stored resolved, so that editing the speed
/// master an entry follows changes what it does on the very next tick without anything
/// having to go looking for the entries that named it.
fn wanted(
    entries: &[ProgrammerValue],
    masters: &[pult_schema::types::speedmaster::SpeedMaster],
) -> HashMap<Key, Held> {
    entries
        .iter()
        .map(|entry| {
            let key = (entry.fixture_id, parameter_key(&entry.parameter_kind));
            let held = match &entry.effect {
                // A programmer effect carries its own anchor, set when the operator
                // applied it. An entry that arrived without one falls back to the
                // epoch rather than to now: any fixed instant will do, but it has to
                // be fixed. Anchoring on the current tick would move the anchor
                // forward exactly as fast as time passes, and the effect would sit
                // still for ever.
                Some(spec) => Held::Effect(effects::resolve(
                    spec,
                    masters,
                    0,
                    EffectSource::Programmer,
                )),
                None => Held::Value(entry.value.clone()),
            };
            (key, held)
        })
        .collect()
}
