//! Turning fixture state into DMX channel values.
//!
//! Shared by every protocol in the DMX family, so Art-Net and sACN differ only in
//! how they put the bytes on the wire.

use std::collections::{HashMap, HashSet};

use pult_render::color::EmitterSpec;
use pult_schema::types::{
    dmx_mode::DmxChannelLayout,
    fixture::{
        driving, emitters_of, Emitter, Fixture, FixtureAddress, FixtureType, ParameterDirection,
        ParameterValue,
    },
    output::{UniverseFrame, UniverseSummary, UniverseTraffic},
    programmer::ProgrammerValue,
};
use uuid::Uuid;

use crate::model::playback::parameter_key;

pub mod encode;

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
    /// Every parameter this patch puts on a wire, and where each one's bytes go.
    ///
    /// Resolved once, when the patch arrives, for the same reason `settles_at` is:
    /// which mode a fixture is in, and where that mode puts each parameter, is a
    /// property of the patch rather than of the moment. A frame over two thousand
    /// fixtures therefore does no mode lookups and no string comparison against a mode
    /// name — it walks a flat list and writes bytes.
    ///
    /// Grouped by *parameter* rather than by channel, and that is the whole point of
    /// the shape: an RGBW head's colour is four bytes and one value, and a flat list
    /// of channels would evaluate the same colour four times a frame.
    placed: Vec<PlacedParameter>,
    /// Every universe this patch occupies, ascending.
    ///
    /// Not derivable from `placed`: a fixture whose parameters are all inputs, or all
    /// on ports, places no channel and still occupies its universe.
    universes: Vec<u16>,
}

/// One parameter of one fixture, and every byte it reaches.
struct PlacedParameter {
    /// Index into `Patch::fixtures` rather than a borrow, so the patch owns one copy
    /// of everything and a connector can keep it across frames.
    fixture: usize,
    /// The key this parameter is driven under, evaluated once per frame however many
    /// bytes it comes to.
    key: String,
    /// Where in `Patch::programmer` this parameter's held entry is, if anything is
    /// holding it. Resolved here so a frame does not build a key to look it up with.
    held: Option<usize>,
    /// What to send when nothing is driving it.
    default: ParameterValue,
    channels: Vec<PlacedChannel>,
}

/// One byte-span of one parameter, at the universe and address its mode puts it.
struct PlacedChannel {
    universe: u16,
    /// The break's 1-based start address in that universe.
    address: u16,
    layout: DmxChannelLayout,
    /// The one emitter this channel drives, for a colour. `None` on everything else —
    /// and on a colour channel whose mode named an emitter the type does not have,
    /// which is a channel there is nothing honest to write into.
    ///
    /// Resolved here rather than per frame: the mixer wants a spec, the schema holds
    /// an `Emitter`, and converting between them per channel per frame was measurably
    /// the whole cost of colour on a rig of five hundred.
    emitter: Option<EmitterSpec>,
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
            placed: Vec::new(),
            universes: Vec::new(),
        };
        patch.settles_at = patch.work_out_when_it_settles();
        (patch.placed, patch.universes) = patch.place_channels();
        patch
    }

    /// Work out where every parameter of every DMX fixture lands.
    ///
    /// A fixture whose mode its type does not have falls back to the type's first mode
    /// and says so once: going dark because a GDTF file was revised is worse than going
    /// to the wrong mode, and silently doing either is worse than both.
    fn place_channels(&self) -> (Vec<PlacedParameter>, Vec<u16>) {
        let mut placed: Vec<PlacedParameter> = Vec::new();
        let mut universes: Vec<u16> = Vec::new();
        let mut warned: HashSet<(Uuid, String)> = HashSet::new();

        for (index, fixture) in self.fixtures.iter().enumerate() {
            let FixtureAddress::Dmx { mode: mode_name, breaks } = &fixture.address else {
                continue;
            };
            let Some(fixture_type) = self.fixture_type(fixture) else {
                // Patched to a type that is not in the show. Nothing sensible to send.
                continue;
            };
            if !fixture_type.dmx_modes.is_empty()
                && !fixture_type.has_mode(mode_name)
                && warned.insert((fixture_type.id, mode_name.clone()))
            {
                tracing::warn!(
                    fixture_type = %fixture_type.name,
                    mode = %mode_name,
                    "no such mode on this type; falling back to its first"
                );
            }
            let mode = fixture_type.mode(mode_name);

            // Every break this unit occupies, so a universe exists for it even when
            // nothing in it is driven. A patched fixture's universe is on the wire
            // whether or not the show is doing anything to it, which is what keeps a
            // rig of sensors from silently dropping its universe off the network.
            for entry in breaks {
                if !universes.contains(&entry.universe) {
                    universes.push(entry.universe);
                }
            }

            // What the type says about each parameter, by key, so the loop below does
            // one lookup rather than a scan per channel.
            let by_key: HashMap<String, (&ParameterValue, Vec<Emitter>)> = fixture_type
                .parameters
                .iter()
                .filter(|p| p.direction == ParameterDirection::Output)
                .map(|p| (parameter_key(&p.kind), (&p.default_value, emitters_of(p))))
                .collect();

            for layout in &mode.channels {
                // A virtual channel occupies no slot, so there is nothing to write.
                if layout.offsets.is_empty() {
                    continue;
                }
                // A mode may place a channel the parameter list has nothing for — a
                // control channel the console has no concept of. Nothing drives it, so
                // nothing writes it.
                let Some((default, emitters)) = by_key.get(&layout.parameter_key) else {
                    continue;
                };
                let Some(entry) = breaks.get(layout.break_index as usize) else {
                    // The mode wants a break this unit was never given an address in.
                    // Dropping the channel is the only answer that is not a guess.
                    continue;
                };
                let channel = PlacedChannel {
                    universe: entry.universe,
                    address: entry.address,
                    layout: layout.clone(),
                    emitter: layout.emitter.as_deref().and_then(|name| {
                        emitters.iter().find(|each| each.name == name).map(encode::spec_of)
                    }),
                };

                // Beside the other bytes of the same parameter where there are any,
                // so the frame evaluates a colour once and writes four bytes from it.
                match placed.iter_mut().rev().find(|each| {
                    each.fixture == index && each.key == layout.parameter_key
                }) {
                    Some(existing) => existing.channels.push(channel),
                    None => placed.push(PlacedParameter {
                        fixture: index,
                        key: layout.parameter_key.clone(),
                        held: self
                            .held
                            .get(&(fixture.id, layout.parameter_key.clone()))
                            .copied(),
                        default: (*default).clone(),
                        channels: vec![channel],
                    }),
                }
            }
        }
        universes.sort_unstable();
        (placed, universes)
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
/// Only fixtures with a DMX address take part, and only through the mode they are
/// patched in: a parameter the mode does not place, one the device writes rather than
/// reads, and one on an OpenHaunt node all have no channel to occupy.
pub fn render(patch: &Patch, now_ms: u64) -> Vec<Universe> {
    render_carried(patch, now_ms, &[])
}

/// The same, restricted to the universes one output carries.
///
/// `carried` is [`pult_schema::types::output::OutputConfig::universes`] and empty
/// means all of them, which is what [`render`] passes.
///
/// **The filter is here rather than at the send**, and the entry that asked for this
/// predicted the opposite. Gating the socket would make the field honest and leave
/// every restricted output evaluating the whole rig — and task 51 measured evaluating
/// at 94% of an output frame at five thousand fixtures. The ordinary reason to write
/// this list down is to split a rig across two nodes; filtered at the send, both
/// halves of the split cost what the undivided rig cost, which is to say the split
/// buys nothing. Filtered here, an Art-Net node carrying four universes of
/// fifty-nine evaluates a fifteenth of the rig.
///
/// Nothing is lost by moving it: two connectors on one station render the patch
/// twice either way, since `send` is per plugin and always was. What the dedup cache
/// would have made unsafe is filtering *after* it — a universe the wire never
/// carried recorded as sent — and no path here does that. A universe the filter
/// drops therefore never enters the cache at all, which is also what makes the wire
/// viewer stop offering universes the connector is not carrying.
pub fn render_carried(patch: &Patch, now_ms: u64, carried: &[u16]) -> Vec<Universe> {
    let mut universes: HashMap<u16, Universe> = patch
        .universes
        .iter()
        .filter(|number| pult_schema::types::output::carries(carried, **number))
        .map(|number| (*number, Universe::new(*number)))
        .collect();

    for parameter in &patch.placed {
        // Nowhere for any of this parameter's bytes to go, so it is never evaluated.
        // The map is the filter: every placed channel's universe came from a break
        // that put that universe in `patch.universes`, so a miss here is the output
        // not carrying it and nothing else.
        if !parameter.channels.iter().any(|channel| universes.contains_key(&channel.universe)) {
            continue;
        }
        let fixture = &patch.fixtures[parameter.fixture];
        // Once, however many bytes it comes to: an RGBW head's colour is one
        // evaluation and four writes, not four of each.
        let driving = driving(
            fixture,
            patch.fixture_type(fixture),
            parameter.held.and_then(|at| patch.programmer.get(at)),
            &parameter.key,
        );
        let value = pult_render::value_at(&driving, now_ms);
        let value = value.as_ref().unwrap_or(&parameter.default);

        for channel in &parameter.channels {
            // A fixture with two breaks can have one of them on a universe this
            // output carries and the other not, and only the carried half is written.
            let Some(universe) = universes.get_mut(&channel.universe) else { continue };
            encode::write_channel(
                &mut universe.channels,
                channel.address,
                &channel.layout,
                value,
                channel.emitter.as_ref(),
            );
        }
    }

    let mut out: Vec<Universe> = universes.into_values().collect();
    out.sort_by_key(|u| u.number);
    out
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
    sent: Vec<Sent>,
}

/// One universe as it was last put on the wire.
struct Sent {
    number: u16,
    channels: [u8; UNIVERSE_SIZE],
    at: std::time::Instant,
    /// When these bytes last became *different* bytes, which is not when they were
    /// last sent: a settled universe goes out on the keep-alive every 800 ms and has
    /// not changed in an hour. The viewer shows both, because "is anything moving in
    /// universe 4" and "is universe 4 still being fed" are different questions and
    /// only one of them is answered by the send.
    changed_at: std::time::Instant,
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
        match self.sent.iter_mut().find(|sent| sent.number == universe.number) {
            Some(sent) => {
                let changed = sent.channels != universe.channels;
                if changed || now.duration_since(sent.at) >= refresh_after {
                    sent.channels = universe.channels;
                    sent.at = now;
                    if changed {
                        sent.changed_at = now;
                    }
                    true
                } else {
                    false
                }
            }
            None => {
                self.sent.push(Sent {
                    number: universe.number,
                    channels: universe.channels,
                    at: now,
                    changed_at: now,
                });
                true
            }
        }
    }

    /// What this connector last put on the wire, for somebody watching.
    ///
    /// Read off the dedup cache and nothing else, which is the point: the images are
    /// already here because skipping an unchanged universe needs them, so a viewer
    /// costs one pass over what the connector was keeping anyway and **nothing at all
    /// on the frame path**. Every DMX-family connector answers through here, so a
    /// sheet reads the same whichever protocol carried it.
    pub fn observe(&self, focus: Option<&str>, now: std::time::Instant) -> UniverseTraffic {
        let since = |then: std::time::Instant| now.saturating_duration_since(then).as_millis() as u32;
        let mut universes: Vec<&Sent> = self.sent.iter().collect();
        universes.sort_by_key(|sent| sent.number);

        // What was asked for, or the lowest-numbered universe when nothing was: a
        // sheet that opens blank until somebody picks a universe is a sheet that
        // looks broken on the rig where there is only one.
        let wanted: Option<u16> = focus.and_then(|f| f.parse().ok());
        let focused = wanted
            .and_then(|number| universes.iter().find(|sent| sent.number == number))
            .or_else(|| universes.first())
            .map(|sent| UniverseFrame { universe: sent.number, channels: sent.channels.to_vec() });

        UniverseTraffic {
            universes: universes
                .iter()
                .map(|sent| UniverseSummary {
                    universe: sent.number,
                    live_channels: sent.channels.iter().filter(|byte| **byte != 0).count() as u16,
                    changed_ms_ago: since(sent.changed_at),
                    sent_ms_ago: since(sent.at),
                })
                .collect(),
            focused,
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
