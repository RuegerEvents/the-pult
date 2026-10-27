//! Several sounds, one sound card.
//!
//! A timeline holds clips: a file, where it starts on the timeline, how much of the file
//! it plays, how loud, and which output channels it goes to. A station plays every clip
//! of every running timeline through **one** output stream, and this is the arithmetic
//! of that stream: where in a file a clip is at a moment, which output channel gets
//! which source channel, and the sum.
//!
//! # A deck per timeline, one clock for all of them
//!
//! A running timeline is a [`Deck`]: a cursor in timeline milliseconds and the voices
//! written against it. Every deck's cursor is advanced by the same callback from the
//! same sample count, so each timeline anchors itself off the sound card and two
//! timelines on one station cannot drift apart. CoreAudio would mix two streams on one
//! device just as well, but that gives two callbacks, and two callbacks are two clocks.
//!
//! # Real time
//!
//! [`Mixer::render`] runs inside an audio callback. It does not allocate, lock or free:
//! a deck arrives already built, and what [`Mixer::put`] and [`Mixer::remove`] hand
//! back is the caller's to drop *off* the audio thread. That is why they return the
//! deck rather than dropping it — a `Box<Deck>` holds `Arc`s to decoded files, and the
//! last reference to forty megabytes of samples must not be released inside a callback.
//!
//! # Where a destination goes
//!
//! Channels count from one, the way a patch bay and Dante Controller count them.
//! [`route`] has three rules, in order:
//!
//! - a mono file goes to every channel it names;
//! - a file with as many channels as it names, or a multiple of that, goes across them in
//!   order — stereo to `[5, 6]` is left on 5 and right on 6, and stereo to
//!   `[1, 2, 3, 4]` is two pairs;
//! - anything else is the file's downmix on each channel named, because stereo to one
//!   zone speaker should be the whole song and not just its left side.
//!
//! A channel the device has not got is a refusal ([`RouteError::Missing`]), never
//! folded onto one it has. A clip routed to channel 12 of an eight-channel interface
//! would otherwise come out of something else's speaker, and nobody at the desk could
//! tell that from silence.

use std::sync::Arc;

use thiserror::Error;

/// A decoded file: interleaved f32 at its own rate.
#[derive(Debug, Clone)]
pub struct Source {
    pub samples: Arc<Vec<f32>>,
    pub channels: usize,
    pub sample_rate: u32,
}

impl Source {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }

    pub fn duration_ms(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames() as f64 * 1_000.0 / self.sample_rate as f64
    }

    /// One sample of one channel at a fractional frame, interpolated linearly. Before
    /// the start and past the end is silence.
    fn read(&self, channel: usize, frame: f64) -> f32 {
        if frame < 0.0 {
            return 0.0;
        }
        let channels = self.channels.max(1);
        let base = frame.floor() as usize;
        let index = base * channels + channel;
        match (self.samples.get(index), self.samples.get(index + channels)) {
            (Some(a), Some(b)) => {
                let fraction = (frame - base as f64) as f32;
                a * (1.0 - fraction) + b * fraction
            }
            (Some(a), None) => *a,
            _ => 0.0,
        }
    }
}

/// Where a clip sits on its timeline, and what of the file it plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Where the clip starts, in timeline milliseconds.
    pub start_ms: f64,
    /// How far into the file it starts: the trim at the front.
    pub offset_ms: f64,
    /// How much of the file it plays from there. `None` is to the end of the file.
    pub length_ms: Option<f64>,
    /// How many times that region plays, back to back. Zero plays it once.
    pub repeats: u32,
    /// Linear, so 1 is the file as it was mixed.
    pub gain: f32,
    /// Into the first time round.
    pub fade_in_ms: f64,
    /// Out of the last time round.
    pub fade_out_ms: f64,
}

impl Placement {
    /// How long the region it plays is, for a file of this length.
    pub fn region_ms(&self, file_ms: f64) -> f64 {
        let rest = (file_ms - self.offset_ms).max(0.0);
        self.length_ms.map_or(rest, |length| length.clamp(0.0, rest))
    }

    /// How long the whole clip lasts on the timeline: the region, as many times as it
    /// plays.
    pub fn span_ms(&self, file_ms: f64) -> f64 {
        self.region_ms(file_ms) * self.repeats.max(1) as f64
    }

    /// Where in the file the clip is at a timeline position, and how loud. `None` is
    /// silence: before the clip starts, after its last time round, or a region with
    /// nothing in it.
    pub fn at(&self, timeline_ms: f64, file_ms: f64) -> Option<(f64, f32)> {
        let local = timeline_ms - self.start_ms;
        if local < 0.0 {
            return None;
        }
        let region = self.region_ms(file_ms);
        if region <= 0.0 {
            return None;
        }
        let span = region * self.repeats.max(1) as f64;
        if local >= span {
            return None;
        }
        let into = local % region;
        let mut envelope = self.gain;
        if self.fade_in_ms > 0.0 && local < self.fade_in_ms {
            envelope *= (local / self.fade_in_ms) as f32;
        }
        if self.fade_out_ms > 0.0 {
            let left = span - local;
            if left < self.fade_out_ms {
                envelope *= (left / self.fade_out_ms) as f32;
            }
        }
        Some((self.offset_ms + into, envelope))
    }
}

/// What one output channel takes from a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Channel(usize),
    /// Every source channel, averaged.
    Downmix,
}

/// One output channel of the device, by index from zero, and what it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub output: usize,
    pub feed: Feed,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RouteError {
    #[error("this clip is routed to no channel")]
    Nowhere,
    #[error("channels count from 1, and this clip names channel 0")]
    Zero,
    #[error("this clip is routed to channel {channel}, and the device has {device}")]
    Missing { channel: u16, device: usize },
}

/// Which output channel gets what from a source. See the module header for the three
/// rules. A channel named twice is taken once.
pub fn route(
    source_channels: usize,
    outputs: &[u16],
    device_channels: usize,
) -> Result<Vec<Route>, RouteError> {
    let mut named: Vec<u16> = Vec::with_capacity(outputs.len());
    for channel in outputs {
        if !named.contains(channel) {
            named.push(*channel);
        }
    }
    if named.is_empty() {
        return Err(RouteError::Nowhere);
    }
    for channel in &named {
        if *channel == 0 {
            return Err(RouteError::Zero);
        }
        if *channel as usize > device_channels {
            return Err(RouteError::Missing { channel: *channel, device: device_channels });
        }
    }
    let source_channels = source_channels.max(1);
    let spread = source_channels == 1 || named.len() % source_channels == 0;
    Ok(named
        .iter()
        .enumerate()
        .map(|(k, channel)| Route {
            output: *channel as usize - 1,
            feed: match (source_channels, spread) {
                (1, _) => Feed::Channel(0),
                (_, true) => Feed::Channel(k % source_channels),
                (_, false) => Feed::Downmix,
            },
        })
        .collect())
}

/// A clip, ready to play: its file, its place, and where it goes.
#[derive(Debug, Clone)]
pub struct Voice {
    pub source: Source,
    pub placement: Placement,
    pub routes: Vec<Route>,
}

/// Where a timeline ends, as the deck needs to know it: a loop wraps here and a stop
/// holds here, sample-accurately, rather than waiting for the leader to relocate the
/// transport a pass later.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct End {
    pub at_ms: f64,
    pub looped: bool,
}

/// One running timeline: a cursor, and the voices written against it.
#[derive(Debug, Clone)]
pub struct Deck {
    /// Whose deck this is. The station keys it by the timeline's id.
    pub id: u128,
    /// Free for the caller: the station uses it as the slot it reads the cursor back
    /// from.
    pub tag: usize,
    /// Where the timeline is, in timeline milliseconds, at the start of the next buffer.
    pub cursor_ms: f64,
    /// A multiple of real time. 1 is normal.
    pub rate: f64,
    /// Silence before the cursor starts moving, in real milliseconds: a timeline
    /// played with its anchor in the future, the way a count-in is. Rendered as
    /// frames like any other, so the first sample lands on the anchor to the frame
    /// rather than to whenever a manager pass happened to notice it.
    pub wait_ms: f64,
    pub end: Option<End>,
    pub voices: Vec<Voice>,
    /// Whether anything was audible in the last buffer rendered.
    pub sounding: bool,
}

impl Deck {
    pub fn new(id: u128, cursor_ms: f64, voices: Vec<Voice>) -> Deck {
        Deck { id, tag: 0, cursor_ms, rate: 1.0, wait_ms: 0.0, end: None, voices, sounding: false }
    }

    /// Whether any voice is audible at the cursor. What the station asks before it
    /// measures drift: with nothing sounding there is nothing anybody can hear to be
    /// in step with.
    pub fn audible_at(&self, timeline_ms: f64) -> bool {
        self.voices.iter().any(|v| v.placement.at(timeline_ms, v.source.duration_ms()).is_some())
    }

    fn advance(&mut self, by_ms: f64) {
        self.cursor_ms += by_ms;
        if let Some(end) = self.end.filter(|end| end.at_ms > 0.0) {
            if self.cursor_ms >= end.at_ms {
                self.cursor_ms = match end.looped {
                    true => self.cursor_ms % end.at_ms,
                    false => end.at_ms,
                };
            }
        }
    }

    /// Stopped at its end: holding there, silent, until the leader stops the timeline.
    fn held(&self) -> bool {
        self.end.is_some_and(|end| !end.looped && end.at_ms > 0.0 && self.cursor_ms >= end.at_ms)
    }
}

/// Every deck on one output stream. See the module header on real time.
#[derive(Debug)]
pub struct Mixer {
    channels: usize,
    sample_rate: u32,
    decks: Vec<Box<Deck>>,
}

impl Mixer {
    /// `capacity` is how many decks may play at once. Allocated here, so that putting a
    /// deck never grows the list inside a callback.
    pub fn new(channels: usize, sample_rate: u32, capacity: usize) -> Mixer {
        Mixer { channels: channels.max(1), sample_rate: sample_rate.max(1), decks: Vec::with_capacity(capacity) }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn decks(&self) -> &[Box<Deck>] {
        &self.decks
    }

    /// Add a deck, or replace the one with its id.
    ///
    /// `keep_cursor` is for an edit: a clip dragged while the timeline runs builds a new
    /// deck, and the new one carries on from where the old one had got to, at its rate,
    /// rather than jumping back to wherever the manager last thought it was.
    ///
    /// `Ok` carries the deck that was replaced, if one was; `Err` gives the deck back
    /// when the mixer is full. Either way, what comes back is the caller's to drop.
    pub fn put(&mut self, mut deck: Box<Deck>, keep_cursor: bool) -> Result<Option<Box<Deck>>, Box<Deck>> {
        if let Some(held) = self.decks.iter_mut().find(|d| d.id == deck.id) {
            if keep_cursor {
                deck.cursor_ms = held.cursor_ms;
                deck.rate = held.rate;
                deck.wait_ms = held.wait_ms;
            }
            return Ok(Some(std::mem::replace(held, deck)));
        }
        if self.decks.len() == self.decks.capacity() {
            return Err(deck);
        }
        self.decks.push(deck);
        Ok(None)
    }

    pub fn remove(&mut self, id: u128) -> Option<Box<Deck>> {
        let at = self.decks.iter().position(|d| d.id == id)?;
        Some(self.decks.swap_remove(at))
    }

    pub fn seek(&mut self, id: u128, timeline_ms: f64) {
        if let Some(deck) = self.decks.iter_mut().find(|d| d.id == id) {
            deck.cursor_ms = timeline_ms.max(0.0);
            deck.wait_ms = 0.0;
        }
    }

    pub fn set_rate(&mut self, id: u128, rate: f64) {
        if let Some(deck) = self.decks.iter_mut().find(|d| d.id == id) {
            deck.rate = rate.max(0.0);
        }
    }

    /// Fill an interleaved buffer of the device's channel count with every deck, summed,
    /// and move each deck's cursor on by the buffer's length.
    ///
    /// Clamped to ±1 at the end: a sum of several clips can go over, and a device
    /// wanting integers would otherwise wrap a loud moment into a crack.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let channels = self.channels;
        let frames = out.len() / channels;
        let frame_ms = 1_000.0 / self.sample_rate as f64;

        for deck in self.decks.iter_mut() {
            let step = frame_ms * deck.rate;
            let mut sounding = false;
            for frame in 0..frames {
                if deck.wait_ms > 0.0 {
                    deck.wait_ms -= frame_ms;
                    continue;
                }
                if deck.held() {
                    break;
                }
                let at = deck.cursor_ms;
                for voice in &deck.voices {
                    let source = &voice.source;
                    let Some((file_ms, envelope)) = voice.placement.at(at, source.duration_ms()) else {
                        continue;
                    };
                    if envelope == 0.0 {
                        continue;
                    }
                    sounding = true;
                    let source_frame = file_ms * source.sample_rate as f64 / 1_000.0;
                    for route in &voice.routes {
                        if route.output >= channels {
                            continue;
                        }
                        let sample = match route.feed {
                            Feed::Channel(c) => source.read(c, source_frame),
                            Feed::Downmix => {
                                let n = source.channels.max(1);
                                (0..n).map(|c| source.read(c, source_frame)).sum::<f32>() / n as f32
                            }
                        };
                        out[frame * channels + route.output] += sample * envelope;
                    }
                }
                deck.advance(step);
            }
            deck.sounding = sounding;
        }
        for sample in out.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}

/// One clip's region, rendered once into a buffer of its own: what a node is sent to
/// keep, rather than what a station mixes live.
///
/// The same arithmetic as a deck, which is the point — the trim, the fades and the
/// downmix a node plays have to be the ones the station would have played. Repeating is
/// left to whoever plays the result, so a placement that plays more than once is
/// rendered as one time round of its region with no fade on either end — a fade baked
/// into it would be heard on every time round.
pub fn bounce(source: &Source, placement: Placement, channels: usize, sample_rate: u32) -> Vec<f32> {
    let channels = channels.max(1);
    let region = placement.region_ms(source.duration_ms());
    let frames = (region * sample_rate as f64 / 1_000.0).round() as usize;
    let outputs: Vec<u16> = (1..=channels as u16).collect();
    let Ok(routes) = route(source.channels, &outputs, channels) else { return Vec::new() };
    let repeated = placement.repeats > 1;
    let placement = Placement {
        start_ms: 0.0,
        repeats: 1,
        fade_in_ms: if repeated { 0.0 } else { placement.fade_in_ms },
        fade_out_ms: if repeated { 0.0 } else { placement.fade_out_ms },
        ..placement
    };
    let mut mixer = Mixer::new(channels, sample_rate, 1);
    let voice = Voice { source: source.clone(), placement, routes };
    let _ = mixer.put(Box::new(Deck::new(0, 0.0, vec![voice])), false);
    let mut out = vec![0.0f32; frames * channels];
    mixer.render(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1_000;

    /// A file whose every sample says which frame it is, on each channel offset by the
    /// channel: frame 5 of channel 1 reads 0.0051. So a test can tell from the output
    /// exactly which frame of which channel arrived where.
    fn counting(channels: usize, frames: usize) -> Source {
        let mut samples = Vec::with_capacity(channels * frames);
        for frame in 0..frames {
            for channel in 0..channels {
                samples.push(frame as f32 / 1_000.0 + channel as f32 / 10_000.0);
            }
        }
        Source { samples: Arc::new(samples), channels, sample_rate: RATE }
    }

    fn constant(channels: usize, frames: usize, value: f32) -> Source {
        Source { samples: Arc::new(vec![value; channels * frames]), channels, sample_rate: RATE }
    }

    fn placed(start_ms: f64) -> Placement {
        Placement {
            start_ms,
            offset_ms: 0.0,
            length_ms: None,
            repeats: 1,
            gain: 1.0,
            fade_in_ms: 0.0,
            fade_out_ms: 0.0,
        }
    }

    fn voice(source: Source, placement: Placement, outputs: &[u16], device: usize) -> Voice {
        let routes = route(source.channels, outputs, device).expect("a route");
        Voice { source, placement, routes }
    }

    /// One buffer of `frames` frames out of an N-channel sink.
    fn render(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * mixer.channels()];
        mixer.render(&mut out);
        out
    }

    fn channel(out: &[f32], channels: usize, which: usize) -> Vec<f32> {
        out.iter().skip(which).step_by(channels).copied().collect()
    }

    #[test]
    fn a_clip_is_silent_before_it_starts_and_after_it_has_played() {
        let placement = placed(1_000.0);
        assert_eq!(placement.at(999.0, 500.0), None);
        assert_eq!(placement.at(1_000.0, 500.0), Some((0.0, 1.0)));
        assert_eq!(placement.at(1_250.0, 500.0), Some((250.0, 1.0)));
        assert_eq!(placement.at(1_500.0, 500.0), None, "a 500 ms file has played by then");
    }

    #[test]
    fn a_deck_that_waits_is_silent_and_then_starts_from_its_cursor() {
        // At 1 kHz a frame is a millisecond: three frames of wait, then frame 0 on.
        let mut mixer = Mixer::new(1, RATE, 4);
        let mut deck = Deck::new(0, 0.0, vec![voice(counting(1, 100), placed(0.0), &[1], 1)]);
        deck.wait_ms = 3.0;
        let _ = mixer.put(Box::new(deck), false);
        let out = render(&mut mixer, 6);
        assert_eq!(out, vec![0.0, 0.0, 0.0, 0.0, 0.001, 0.002]);
        assert_eq!(mixer.decks()[0].cursor_ms, 3.0, "the cursor did not move while it waited");
    }

    #[test]
    fn a_trim_is_an_offset_into_the_file_and_a_length() {
        let placement = Placement { offset_ms: 200.0, length_ms: Some(100.0), ..placed(0.0) };
        assert_eq!(placement.at(0.0, 1_000.0), Some((200.0, 1.0)));
        assert_eq!(placement.at(99.0, 1_000.0), Some((299.0, 1.0)));
        assert_eq!(placement.at(100.0, 1_000.0), None);
        // A length longer than what is left of the file stops at the file.
        let long = Placement { offset_ms: 900.0, length_ms: Some(500.0), ..placed(0.0) };
        assert_eq!(long.region_ms(1_000.0), 100.0);
    }

    #[test]
    fn a_repeated_clip_goes_round_its_region_as_many_times_as_it_says() {
        let placement = Placement { offset_ms: 100.0, length_ms: Some(200.0), repeats: 3, ..placed(0.0) };
        assert_eq!(placement.at(250.0, 1_000.0), Some((150.0, 1.0)));
        assert_eq!(placement.at(450.0, 1_000.0), Some((150.0, 1.0)), "the third time round");
        assert_eq!(placement.at(600.0, 1_000.0), None, "and then it is over");
        assert_eq!(placement.span_ms(1_000.0), 600.0);
    }

    #[test]
    fn fades_scale_the_ends_of_the_whole_clip() {
        let placement = Placement { fade_in_ms: 100.0, fade_out_ms: 100.0, gain: 0.5, ..placed(0.0) };
        assert_eq!(placement.at(50.0, 1_000.0), Some((50.0, 0.25)));
        assert_eq!(placement.at(500.0, 1_000.0), Some((500.0, 0.5)));
        assert_eq!(placement.at(950.0, 1_000.0), Some((950.0, 0.25)));
        let twice = Placement { repeats: 2, ..placement };
        assert_eq!(twice.at(950.0, 1_000.0), Some((950.0, 0.5)), "no fade out of the first time round");
        assert_eq!(twice.at(1_050.0, 1_000.0), Some((50.0, 0.5)), "nor into the second");
        assert_eq!(twice.at(1_950.0, 1_000.0), Some((950.0, 0.25)), "out of the last");
    }

    #[test]
    fn mono_goes_to_every_channel_it_names_and_nowhere_else() {
        let routes = route(1, &[3, 7], 8).unwrap();
        assert_eq!(
            routes,
            vec![
                Route { output: 2, feed: Feed::Channel(0) },
                Route { output: 6, feed: Feed::Channel(0) },
            ]
        );
    }

    #[test]
    fn stereo_goes_across_its_channels_in_pairs_and_downmixes_onto_one() {
        assert_eq!(
            route(2, &[5, 6, 1, 2], 8).unwrap().iter().map(|r| (r.output, r.feed)).collect::<Vec<_>>(),
            vec![
                (4, Feed::Channel(0)),
                (5, Feed::Channel(1)),
                (0, Feed::Channel(0)),
                (1, Feed::Channel(1)),
            ]
        );
        assert_eq!(route(2, &[4], 8).unwrap(), vec![Route { output: 3, feed: Feed::Downmix }]);
        assert_eq!(route(2, &[1, 2, 3], 8).unwrap()[2].feed, Feed::Downmix);
    }

    #[test]
    fn a_channel_the_device_has_not_got_is_refused_by_name() {
        assert_eq!(route(1, &[12], 8), Err(RouteError::Missing { channel: 12, device: 8 }));
        assert_eq!(route(1, &[], 8), Err(RouteError::Nowhere));
        assert_eq!(route(1, &[0], 8), Err(RouteError::Zero));
        assert_eq!(route(1, &[2, 2], 8).unwrap().len(), 1, "a channel named twice is taken once");
    }

    /// The routing, end to end through a fake eight-channel sink: a mono scream on
    /// zone 3 and a stereo bed on zones 5 and 6 come out exactly there.
    #[test]
    fn two_timelines_on_one_device_land_on_their_own_channels() {
        let mut mixer = Mixer::new(8, RATE, 4);
        let scream = Deck::new(1, 0.0, vec![voice(constant(1, 1_000, 0.5), placed(0.0), &[3], 8)]);
        let bed = Deck::new(2, 0.0, vec![voice(counting(2, 1_000), placed(0.0), &[5, 6], 8)]);
        assert!(mixer.put(Box::new(scream), false).unwrap().is_none());
        assert!(mixer.put(Box::new(bed), false).unwrap().is_none());

        let out = render(&mut mixer, 10);
        for silent in [0, 1, 3, 6, 7] {
            assert!(channel(&out, 8, silent).iter().all(|s| *s == 0.0), "channel {} is silent", silent + 1);
        }
        assert!(channel(&out, 8, 2).iter().all(|s| *s == 0.5));
        assert!((channel(&out, 8, 4)[5] - 0.005).abs() < 1e-6, "left, frame 5");
        assert!((channel(&out, 8, 5)[5] - 0.0051).abs() < 1e-6, "right, frame 5");
    }

    #[test]
    fn clips_sum_and_the_sum_is_clamped() {
        let mut mixer = Mixer::new(2, RATE, 1);
        let loud = || voice(constant(1, 100, 0.75), placed(0.0), &[1], 2);
        mixer.put(Box::new(Deck::new(1, 0.0, vec![loud(), loud()])), false).unwrap();
        assert!(channel(&render(&mut mixer, 4), 2, 0).iter().all(|s| *s == 1.0));
    }

    #[test]
    fn a_clip_starts_where_the_timeline_says_and_the_cursor_moves_with_the_buffer() {
        let mut mixer = Mixer::new(1, RATE, 1);
        mixer.put(Box::new(Deck::new(1, 0.0, vec![voice(counting(1, 1_000), placed(5.0), &[1], 1)])), false).unwrap();
        let out = render(&mut mixer, 10);
        assert_eq!(&out[..5], &[0.0; 5]);
        assert!((out[7] - 0.002).abs() < 1e-6, "two frames into the clip");
        assert_eq!(mixer.decks()[0].cursor_ms, 10.0);
        assert!(mixer.decks()[0].sounding);
    }

    #[test]
    fn a_deck_loops_at_the_timelines_end_by_itself() {
        let mut deck = Deck::new(1, 95.0, vec![voice(counting(1, 1_000), placed(0.0), &[1], 1)]);
        deck.end = Some(End { at_ms: 100.0, looped: true });
        let mut mixer = Mixer::new(1, RATE, 1);
        mixer.put(Box::new(deck), false).unwrap();
        let out = render(&mut mixer, 10);
        assert!((out[4] - 0.099).abs() < 1e-6);
        assert!(out[5].abs() < 1e-6, "back at the top on the next frame");
        assert!((out[9] - 0.004).abs() < 1e-6);
        assert_eq!(mixer.decks()[0].cursor_ms, 5.0);
    }

    #[test]
    fn a_deck_stopping_at_its_end_holds_there_silent() {
        let mut deck = Deck::new(1, 95.0, vec![voice(constant(1, 1_000, 0.5), placed(0.0), &[1], 1)]);
        deck.end = Some(End { at_ms: 100.0, looped: false });
        let mut mixer = Mixer::new(1, RATE, 1);
        mixer.put(Box::new(deck), false).unwrap();
        let out = render(&mut mixer, 10);
        assert_eq!(&out[..5], &[0.5; 5]);
        assert_eq!(&out[5..], &[0.0; 5]);
        assert_eq!(mixer.decks()[0].cursor_ms, 100.0);
        assert!(!mixer.render_sounding(), "and the next buffer is silent");
    }

    #[test]
    fn replacing_a_deck_keeps_its_cursor_and_hands_the_old_one_back() {
        let mut mixer = Mixer::new(1, RATE, 1);
        mixer.put(Box::new(Deck::new(1, 0.0, Vec::new())), false).unwrap();
        render(&mut mixer, 40);
        let old = mixer.put(Box::new(Deck::new(1, 0.0, Vec::new())), true).unwrap();
        assert!(old.is_some(), "the replaced deck is the caller's to drop");
        assert_eq!(mixer.decks()[0].cursor_ms, 40.0);
        assert!(mixer.put(Box::new(Deck::new(2, 0.0, Vec::new())), false).is_err(), "full");
        assert!(mixer.remove(1).is_some());
        assert!(mixer.decks().is_empty());
    }

    #[test]
    fn rate_and_seek_move_one_deck_only() {
        let mut mixer = Mixer::new(1, RATE, 2);
        mixer.put(Box::new(Deck::new(1, 0.0, Vec::new())), false).unwrap();
        mixer.put(Box::new(Deck::new(2, 0.0, Vec::new())), false).unwrap();
        mixer.set_rate(1, 0.5);
        mixer.seek(2, 1_000.0);
        render(&mut mixer, 10);
        assert_eq!(mixer.decks().iter().find(|d| d.id == 1).unwrap().cursor_ms, 5.0);
        assert_eq!(mixer.decks().iter().find(|d| d.id == 2).unwrap().cursor_ms, 1_010.0);
    }

    /// What a node keeps is the region the timeline plays: trimmed, at the rate asked
    /// for, and down to the node's channels.
    #[test]
    fn a_bounce_is_the_trimmed_region_at_the_asked_rate() {
        let source = counting(2, 1_000);
        let placement = Placement { offset_ms: 100.0, length_ms: Some(200.0), ..placed(50.0) };
        let out = bounce(&source, placement, 1, RATE);
        assert_eq!(out.len(), 200, "200 ms at 1 kHz, wherever the clip starts on the timeline");
        // Mono from stereo is the downmix: the two channels' frame 100, averaged.
        assert!((out[0] - (0.100 + 0.1001) / 2.0).abs() < 1e-6, "{}", out[0]);

        let doubled = bounce(&source, placement, 2, RATE * 2);
        assert_eq!(doubled.len(), 2 * 400, "twice the rate, twice the frames");
    }

    #[test]
    fn a_repeated_bounce_is_one_time_round_with_no_fades() {
        let source = constant(1, 1_000, 0.5);
        let placement = Placement { repeats: 4, fade_in_ms: 500.0, ..placed(0.0) };
        let out = bounce(&source, placement, 1, RATE);
        assert_eq!(out.len(), 1_000);
        assert_eq!(out[0], 0.5, "a fade-in baked into a loop would be heard on every lap");
    }

    impl Mixer {
        fn render_sounding(&mut self) -> bool {
            let mut out = vec![0.0; self.channels * 4];
            self.render(&mut out);
            self.decks.iter().any(|d| d.sounding)
        }
    }
}
