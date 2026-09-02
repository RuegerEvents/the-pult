//! Turning fixture state into DMX channel values.
//!
//! Shared by every protocol in the DMX family, so Art-Net and sACN differ only in
//! how they put the bytes on the wire.

use std::collections::HashMap;

use pult_schema::types::{
    fixture::{
        driving, Fixture, FixtureType, ParameterDirection, ParameterValue,
    },
    programmer::ProgrammerValue,
};
use uuid::Uuid;

use crate::model::playback::parameter_key;

/// A DMX universe: 512 channels, indexed from 0 for channel 1.
pub const UNIVERSE_SIZE: usize = 512;

/// One universe of channel data.
#[derive(Clone)]
pub struct Universe {
    pub number: u16,
    pub channels: [u8; UNIVERSE_SIZE],
}

impl Universe {
    pub fn new(number: u16) -> Self {
        Self { number, channels: [0; UNIVERSE_SIZE] }
    }
}

/// The patch: what a plugin needs to work out a fixture's values and place them on a
/// wire.
///
/// Values, not *the* values. Nothing here is a number a station computed and stored;
/// it is what is *driving* each parameter — the fades and effects anchored in console
/// time, the programmer over the top, the home value underneath — and a connector
/// turns that into numbers for whatever moment its own frame is for. Which is why a
/// connector can run at its own rate over a patch nobody has pushed it since the cue
/// went.
pub struct Patch {
    pub fixtures: Vec<Fixture>,
    pub fixture_types: HashMap<Uuid, FixtureType>,
    /// What the programmer is holding, indexed by the parameter it holds.
    ///
    /// Owned rather than borrowed, because a connector keeps its patch across frames
    /// and the engine is long gone by the time the next one is drawn.
    pub programmer: Vec<ProgrammerValue>,
    /// Where in `programmer` each held parameter is, so a frame over a rig of
    /// thousands does not scan a look of thousands per parameter.
    held: HashMap<(Uuid, String), usize>,
    /// The console millisecond after which nothing in this patch changes on its own,
    /// or `None` while an effect is running.
    ///
    /// Worked out once, when the patch arrives, because it is a property of the
    /// descriptions rather than of the moment: it is what lets a connector drop from
    /// its frame rate to its protocol's keep-alive when a show has settled.
    settles_at: Option<u64>,
}

impl Patch {
    pub fn new(
        fixtures: Vec<Fixture>,
        fixture_types: Vec<FixtureType>,
        programmer: Vec<ProgrammerValue>,
    ) -> Self {
        let held = programmer
            .iter()
            .enumerate()
            .map(|(at, entry)| ((entry.fixture_id, parameter_key(&entry.parameter_kind)), at))
            .collect();
        let mut patch = Self {
            fixtures,
            fixture_types: fixture_types.into_iter().map(|t| (t.id, t)).collect(),
            programmer,
            held,
            settles_at: Some(0),
        };
        patch.settles_at = patch.work_out_when_it_settles();
        patch
    }

    pub fn fixture_type(&self, fixture: &Fixture) -> Option<&FixtureType> {
        self.fixture_types.get(&fixture.fixture_type_id)
    }

    /// What is acting on one parameter of one fixture, highest priority first.
    pub fn driving<'a>(&'a self, fixture: &'a Fixture, key: &str) -> pult_render::Driving<'a> {
        let held = self
            .held
            .get(&(fixture.id, key.to_string()))
            .and_then(|at| self.programmer.get(*at));
        driving(fixture, self.fixture_type(fixture), held, key)
    }

    /// What one parameter is putting out at `now_ms`.
    pub fn value_at(&self, fixture: &Fixture, key: &str, now_ms: u64) -> Option<ParameterValue> {
        pult_render::value_at(&self.driving(fixture, key), now_ms)
    }

    /// True while anything in the patch is still moving at `now_ms`.
    pub fn is_moving(&self, now_ms: u64) -> bool {
        match self.settles_at {
            None => true,
            Some(at) => now_ms < at,
        }
    }

    fn work_out_when_it_settles(&self) -> Option<u64> {
        let mut latest = 0;
        for fixture in &self.fixtures {
            for fade in fixture.live_fades.values() {
                latest = latest.max(fade.t0.saturating_add(fade.duration_ms as u64));
            }
            if !fixture.live_effects.is_empty() {
                return None;
            }
        }
        // A programmer shape runs for ever too, wherever the station resolved it to.
        if self.programmer.iter().any(|entry| entry.effect.is_some()) {
            return None;
        }
        Some(latest)
    }
}

/// Render the whole patch into universes, one per universe number in use, as of
/// `now_ms`.
///
/// Only fixtures with a DMX address take part. A fixture on an OpenHaunt node has
/// no slot in a universe, and neither does a parameter bound to a port or one the
/// device writes rather than reads — none of those have a channel to occupy.
pub fn render(patch: &Patch, now_ms: u64) -> Vec<Universe> {
    let mut universes: HashMap<u16, Universe> = HashMap::new();

    for fixture in &patch.fixtures {
        let Some((number, address)) = fixture.address.dmx() else { continue };
        let Some(fixture_type) = patch.fixture_type(fixture) else {
            // Patched to a type that is not in the show. Nothing sensible to send.
            continue;
        };
        let universe = universes.entry(number).or_insert_with(|| Universe::new(number));

        for parameter in &fixture_type.parameters {
            if parameter.direction != ParameterDirection::Output {
                continue;
            }
            let Some(channel) = parameter.binding.dmx_channel() else { continue };
            let key = parameter_key(&parameter.kind);
            let value = patch
                .value_at(fixture, &key, now_ms)
                .unwrap_or_else(|| parameter.default_value.clone());
            write_parameter(&mut universe.channels, address, channel, &value);
        }
    }

    let mut out: Vec<Universe> = universes.into_values().collect();
    out.sort_by_key(|u| u.number);
    out
}

/// Write one parameter into the universe at the fixture's address.
///
/// Addresses are 1-based on the outside and 0-based in the buffer. A parameter that
/// would run past channel 512 is dropped rather than wrapping into another fixture.
fn write_parameter(
    channels: &mut [u8; UNIVERSE_SIZE],
    dmx_address: u16,
    channel: u8,
    value: &ParameterValue,
) {
    let base = dmx_address as usize + channel as usize - 1;
    let Some(start) = base.checked_sub(1) else { return };

    match value {
        ParameterValue::Color { r, g, b } => {
            for (offset, component) in [r, g, b].into_iter().enumerate() {
                if let Some(slot) = channels.get_mut(start + offset) {
                    *slot = to_byte(*component);
                }
            }
        }
        other => {
            if let Some(slot) = channels.get_mut(start) {
                *slot = match other {
                    ParameterValue::Float(f) => to_byte(*f),
                    ParameterValue::Int(i) => (*i).clamp(0, 255) as u8,
                    ParameterValue::Bool(true) => 255,
                    ParameterValue::Bool(false) => 0,
                    // A display's text has no byte on a DMX line. Leaving the channel
                    // alone is the only honest thing to write.
                    ParameterValue::Text(_) => return,
                    ParameterValue::Color { .. } => unreachable!("handled above"),
                };
            }
        }
    }
}

// ── Not sending what has not changed ──────────────────────────────────────────

/// A DMX-family protocol expects a receiver to hear from a controller regularly.
/// Re-sending every universe about once a second keeps one from deciding the
/// controller is gone, without putting an idle rig's full output on the wire 40
/// times a second.
pub const REFRESH_AFTER: std::time::Duration = std::time::Duration::from_millis(800);

/// Remembers what was last sent per universe, so an unchanged one is skipped.
///
/// Shared by every protocol that carries whole universes — Art-Net, sACN, and the
/// unicast the OpenHaunt gateway wants — because the rule is the same for all of
/// them and getting it subtly different per protocol is how an idle rig starts
/// flooding one wire and not another.
#[derive(Default)]
pub struct UniverseCache {
    sent: Vec<(u16, [u8; UNIVERSE_SIZE], std::time::Instant)>,
}

impl UniverseCache {
    /// True if this universe has changed, or has gone long enough without a refresh.
    /// Records the universe as sent, so calling it twice for one frame is wrong.
    pub fn needs_send(
        &mut self,
        universe: &Universe,
        now: std::time::Instant,
        refresh_after: std::time::Duration,
    ) -> bool {
        match self.sent.iter_mut().find(|(n, _, _)| *n == universe.number) {
            Some((_, channels, last)) => {
                let changed = *channels != universe.channels;
                if changed || now.duration_since(*last) >= refresh_after {
                    *channels = universe.channels;
                    *last = now;
                    true
                } else {
                    false
                }
            }
            None => {
                self.sent.push((universe.number, universe.channels, now));
                true
            }
        }
    }
}

/// A sequence counter per universe, for the protocols that carry one.
///
/// Zero means "sequence not implemented" in both Art-Net and E1.31, so it wraps
/// through 1..=255 rather than through zero.
#[derive(Default)]
pub struct SequenceCounter {
    counters: Vec<(u16, u8)>,
}

impl SequenceCounter {
    pub fn next(&mut self, universe: u16) -> u8 {
        match self.counters.iter_mut().find(|(u, _)| *u == universe) {
            Some((_, seq)) => {
                *seq = if *seq >= 255 { 1 } else { *seq + 1 };
                *seq
            }
            None => {
                self.counters.push((universe, 1));
                1
            }
        }
    }
}

/// A 0.0 to 1.0 parameter as a DMX byte. Out-of-range values clamp rather than wrap,
/// so a bad value dims a light instead of flashing it to full.
fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Put a fixture where the console holds a parameter at a value.
///
/// A landed fade, from the value to itself, because that is the only way a parameter
/// holds a value now — nothing stores the number. Test-only, and shared by every
/// connector's tests so that they all describe the same rig the same way.
#[cfg(test)]
pub(crate) fn holding(fixture: &mut Fixture, key: &str, value: ParameterValue) {
    use pult_schema::types::effect::{Easing, RunningFade};
    fixture.live_fades.insert(
        key.into(),
        RunningFade {
            from: value.clone(),
            to: value,
            t0: 0,
            duration_ms: 0,
            easing: Easing::Step,
            cue_id: Uuid::nil(),
        },
    );
}

#[cfg(test)]
mod tests;
