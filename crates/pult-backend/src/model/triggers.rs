//! Trigger evaluation: pure, like playback, and for the same reason.
//!
//! [`Triggers::tick`] takes the rules, the inputs that arrived since the last tick,
//! and a timestamp, and returns what to do. It reads no clock and touches no
//! engine, so a test can run a ten-second delay in microseconds.
//!
//! Inputs are handed in as a list rather than read from the current state, because
//! a button pressed and released between two ticks would otherwise look like
//! nothing happening at all.

use std::time::Instant;

use pult_schema::types::{
    fixture::ParameterValue,
    trigger::{Trigger, TriggerAction, TriggerCondition, TriggerSource},
};
use uuid::Uuid;

/// One parameter changing, as the engine saw it happen.
#[derive(Debug, Clone, PartialEq)]
pub struct InputEvent {
    pub fixture_id: Uuid,
    /// The `live_values` key, as [`crate::model::playback::parameter_key`] writes it.
    pub key: String,
    /// What was there before, if anything ever was.
    pub previous: Option<ParameterValue>,
    pub current: ParameterValue,
}

/// What a trigger tick asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerEffect {
    /// Carry out a trigger's action, and mark it as having fired.
    Fire { trigger_id: Uuid, action: TriggerAction },
    /// The delay has started, or has finished.
    SetPending { trigger_id: Uuid, pending: bool },
}

#[derive(Default)]
pub struct Triggers {
    /// Triggers whose condition has been met, and when they are due.
    pending: Vec<(Uuid, Instant)>,
}

impl Triggers {
    /// Is there anything to do even with no input? A delay still has to expire.
    pub fn has_work(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn tick(
        &mut self,
        now: Instant,
        triggers: &[Trigger],
        inputs: &[InputEvent],
    ) -> Vec<TriggerEffect> {
        let mut effects = Vec::new();

        // A trigger deleted or switched off while its delay was running does not go
        // off afterwards. Dropped silently: there is nothing left to mark as done.
        self.pending.retain(|(id, _)| {
            triggers.iter().any(|t| t.id == *id && t.enabled)
        });

        for input in inputs {
            for trigger in triggers.iter().filter(|t| t.enabled) {
                if !watches(trigger, input) || !fires(trigger.condition, input) {
                    continue;
                }
                if trigger.delay_ms == 0 {
                    effects.push(TriggerEffect::Fire {
                        trigger_id: trigger.id,
                        action: trigger.action.clone(),
                    });
                    continue;
                }
                let due = now + std::time::Duration::from_millis(trigger.delay_ms as u64);
                // Re-arming a trigger that is already waiting restarts its delay,
                // rather than queueing a second one behind the first.
                self.pending.retain(|(id, _)| *id != trigger.id);
                self.pending.push((trigger.id, due));
                effects.push(TriggerEffect::SetPending { trigger_id: trigger.id, pending: true });
            }
        }

        let (due, waiting): (Vec<_>, Vec<_>) =
            self.pending.iter().partition(|(_, at)| *at <= now);
        self.pending = waiting;
        for (trigger_id, _) in due {
            let Some(trigger) = triggers.iter().find(|t| t.id == trigger_id) else { continue };
            effects.push(TriggerEffect::SetPending { trigger_id, pending: false });
            effects.push(TriggerEffect::Fire { trigger_id, action: trigger.action.clone() });
        }

        effects
    }
}

/// Is this input the thing the trigger is watching?
fn watches(trigger: &Trigger, input: &InputEvent) -> bool {
    match &trigger.source {
        TriggerSource::Parameter { fixture_id, parameter } => {
            *fixture_id == input.fixture_id
                && crate::model::playback::parameter_key(parameter) == input.key
        }
    }
}

fn fires(condition: TriggerCondition, input: &InputEvent) -> bool {
    match condition {
        TriggerCondition::RisingEdge => {
            !as_bool(input.previous.as_ref()).unwrap_or(false) && as_bool(Some(&input.current)) == Some(true)
        }
        TriggerCondition::FallingEdge => {
            as_bool(input.previous.as_ref()) == Some(true)
                && as_bool(Some(&input.current)) == Some(false)
        }
        TriggerCondition::AnyChange => input.previous.as_ref() != Some(&input.current),
        // On the crossing, not on the level: a room that is already warm must not
        // fire the cue again on every reading.
        TriggerCondition::Above(threshold) => {
            let current = as_number(Some(&input.current));
            let previous = as_number(input.previous.as_ref());
            current.is_some_and(|c| c > threshold)
                && previous.map(|p| p <= threshold).unwrap_or(true)
        }
        TriggerCondition::Below(threshold) => {
            let current = as_number(Some(&input.current));
            let previous = as_number(input.previous.as_ref());
            current.is_some_and(|c| c < threshold)
                && previous.map(|p| p >= threshold).unwrap_or(true)
        }
    }
}

/// A parameter as a truth value, for the kinds that have one.
fn as_bool(value: Option<&ParameterValue>) -> Option<bool> {
    match value? {
        ParameterValue::Bool(b) => Some(*b),
        // A level counts as closed once it is off zero, so an edge condition works
        // on a dimmer as well as on a contact.
        ParameterValue::Float(f) => Some(*f > 0.0),
        ParameterValue::Int(i) => Some(*i != 0),
        ParameterValue::Text(t) => Some(!t.is_empty()),
        ParameterValue::Color { .. } => None,
    }
}

fn as_number(value: Option<&ParameterValue>) -> Option<f32> {
    match value? {
        ParameterValue::Float(f) => Some(*f),
        ParameterValue::Int(i) => Some(*i as f32),
        ParameterValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        ParameterValue::Text(_) | ParameterValue::Color { .. } => None,
    }
}

#[cfg(test)]
mod tests;
