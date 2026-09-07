//! Linear timecode: somebody else's clock, arriving as sound.
//!
//! LTC is an audio signal on a cable — a whole frame of timecode in one frame's worth
//! of audio, biphase-mark encoded so it survives being recorded, played back at the
//! wrong level, wired the wrong way round, and shuttled. That is the whole reason it is
//! still what a venue actually has: it goes down the same cable a microphone does.
//!
//! # The frame
//!
//! Eighty bits, sent least-significant-bit first within each field, ending in a sync
//! word that is the only pattern in the whole format that cannot occur in the data:
//! `0011 1111 1111 1101`. Twelve consecutive ones appear nowhere else, because every
//! run of data bits is broken by a flag or a BCD field's high zero bits. So a decoder
//! never has to be told where a frame starts — it shifts bits in and waits for the
//! word.
//!
//! The fields are BCD, and this console reads exactly those it acts on: frame,
//! second, minute, hour, and the drop-frame flag. **User bits are ignored**, and that
//! is a decision rather than an omission: they carry a date on some machines, a reel
//! number on others, and nothing at all on most, and a console that read them would
//! have to have an opinion about which.
//!
//! # Biphase mark, and why the decoder is only intervals
//!
//! Every bit boundary is a transition. A `1` has one more, in the middle of the bit.
//! So a `0` is one long interval between transitions and a `1` is two short ones —
//! which means the decoder never looks at a level, only at the distance between
//! crossings. Three consequences, all of them the point:
//!
//! **Polarity does not matter.** Swap the two conductors and every transition is still
//! a transition. A phase-inverted LTC feed is a fault nobody can hear and this cannot
//! see either, which is correct.
//!
//! **Level does not matter, within reason.** The crossings are found against a slow
//! running mean with hysteresis, so a signal at −20 dBu and one at +4 decode alike, and
//! a DC offset from a cheap interface does not stop it.
//!
//! **Sample rate does not matter.** The bit period is *measured*, not assumed: an
//! interval is short or long relative to the current estimate, which is re-estimated
//! from what has been arriving. 44.1 kHz through 96 kHz is nothing special, and neither
//! is a machine shuttling at 0.9×.
//!
//! # The frame rate is declared, never sniffed
//!
//! What the wire carries is a frame *count*. Whether 30 counts is a second, or
//! 1001/1000 of one, is not in the signal at all — 29.97 non-drop and 30 differ by one
//! frame in a thousand, which is thirty-six frames an hour and looks exactly like
//! nothing for the first ten minutes. So [`LtcRate`] comes off the timeline, the
//! operator sets it, and this module never guesses. The drop-frame *flag* is read and
//! reported, because a source that says it is dropping frames while the show says 25
//! is a mistake somebody can be told about.
//!
//! # The encoder
//!
//! It exists so the decoder can be tested. Generating a stream at every rate, at
//! awkward sample rates, across the minute boundaries where 29.97df drops frame
//! numbers and the tens of minutes where it does not, is the only way to know the
//! arithmetic is right without a machine in the room — and it is also what feeds the
//! integration test that anchors a timeline from timecode with no audio device
//! anywhere near it.

use serde::{Deserialize, Serialize};

/// The frame rates timecode is actually generated at.
///
/// The same five [`pult_schema::types::timeline::LtcRate`] carries. Repeated here
/// rather than imported because this crate depends on no pult crate — the wall
/// `pult-render` was split out over — and the backend converts between them in one
/// place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LtcRate {
    F24,
    F25,
    F30,
    /// 29.97 non-drop: the frame *count* runs at 30 and the clock is slow.
    F2997,
    /// 29.97 drop-frame, which skips frame numbers so the clock stays honest.
    F2997Df,
}

impl LtcRate {
    /// How many frame numbers a second of counting holds. Not the frame *rate*: at
    /// 29.97 this is 30, and the second it counts is 1001/1000 of a real one.
    pub fn counts_per_second(self) -> u32 {
        match self {
            LtcRate::F24 => 24,
            LtcRate::F25 => 25,
            LtcRate::F30 | LtcRate::F2997 | LtcRate::F2997Df => 30,
        }
    }

    /// Real seconds per counted frame, as a ratio, so the arithmetic below can stay
    /// in integers: 1001/30000 at 29.97 and 1/25 at 25.
    fn seconds_per_frame(self) -> (u64, u64) {
        match self {
            LtcRate::F24 => (1, 24),
            LtcRate::F25 => (1, 25),
            LtcRate::F30 => (1, 30),
            LtcRate::F2997 | LtcRate::F2997Df => (1001, 30_000),
        }
    }

    /// Whether numbers are dropped, which is a property of the rate and not of the
    /// flag on the wire.
    pub fn drops_frames(self) -> bool {
        matches!(self, LtcRate::F2997Df)
    }
}

/// One decoded frame of timecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LtcFrame {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    /// What the *signal* claims. Compared against the declared rate rather than used
    /// in place of it: a generator set to drop-frame feeding a 25 fps show is a fault
    /// worth naming.
    pub drop_frame: bool,
    /// How many samples into the pushed buffer this frame's sync word ended, so a
    /// caller can say when it arrived rather than only what it said.
    pub ended_at_sample: u64,
}

impl LtcFrame {
    /// Where this frame sits on the clock, in milliseconds, at a declared rate.
    ///
    /// The drop-frame case is the whole reason this is a function. 29.97df skips the
    /// numbers `:00` and `:01` at the top of every minute **except** every tenth
    /// minute, which brings the counted clock back to within a frame of the real one
    /// over an hour. So the frame *number* is not the count of frames since midnight,
    /// and reading it as one drifts by 108 frames an hour — three and a half seconds,
    /// which on a musical cue is a Go in the wrong bar.
    pub fn to_ms(self, rate: LtcRate) -> u64 {
        to_ms(self.hours, self.minutes, self.seconds, self.frames, rate)
    }
}

/// The clock position of an hour, minute, second and frame at a declared rate.
///
/// Free-standing as well as on [`LtcFrame`] so the corpus can be a table of numbers.
pub fn to_ms(hours: u8, minutes: u8, seconds: u8, frames: u8, rate: LtcRate) -> u64 {
    let total_minutes = hours as u64 * 60 + minutes as u64;
    let counts = rate.counts_per_second() as u64;
    let mut frame_number =
        ((total_minutes * 60) + seconds as u64) * counts + frames as u64;
    if rate.drops_frames() {
        // Two numbers per minute, less two per tenth minute, are never sent — so the
        // frame *number* is that many ahead of the count of frames actually elapsed.
        frame_number -= 2 * (total_minutes - total_minutes / 10);
    }
    let (num, den) = rate.seconds_per_frame();
    // Rounded rather than truncated: at 29.97 a frame is 33.3667 ms and truncating
    // loses a third of a millisecond per frame, which is a frame an hour.
    (frame_number * num * 1000 + den / 2) / den
}

// ── Encoding ──────────────────────────────────────────────────────────────────

/// The sync word as it sits in a shift register that takes each new bit at the top.
///
/// `0011 1111 1111 1101` in transmission order; bit-reversed, because the register
/// below shifts right and the first bit sent ends up lowest.
const SYNC: u16 = 0xBFFC;

/// The eighty bits of one frame, in transmission order, ready to be biphase-marked.
///
/// Only the fields this console acts on are written. Everything else — the user bits,
/// the colour-frame flag, the binary group flags — goes out as zero, which is what a
/// generator that has nothing to say in them does.
pub fn frame_bits(hours: u8, minutes: u8, seconds: u8, frames: u8, drop_frame: bool) -> u128 {
    fn put(value: u8, at: u32, width: u32) -> u128 {
        let mut out = 0u128;
        for i in 0..width {
            if value >> i & 1 == 1 {
                out |= 1u128 << (at + i);
            }
        }
        out
    }
    let mut bits: u128 = 0;
    bits |= put(frames % 10, 0, 4);
    bits |= put(frames / 10, 8, 2);
    if drop_frame {
        bits |= 1u128 << 10;
    }
    bits |= put(seconds % 10, 16, 4);
    bits |= put(seconds / 10, 24, 3);
    bits |= put(minutes % 10, 32, 4);
    bits |= put(minutes / 10, 40, 3);
    bits |= put(hours % 10, 48, 4);
    bits |= put(hours / 10, 56, 2);
    bits |= (SYNC as u128) << 64;
    bits
}

/// Generate one frame of LTC audio.
///
/// `level` is the peak amplitude. `phase` carries the encoder's polarity between
/// frames, because a biphase stream is continuous — the first transition of a frame is
/// the last bit boundary of the one before it, and a generator that reset to a fixed
/// level every frame would put a glitch on every frame boundary.
pub fn encode_frame(
    out: &mut Vec<f32>,
    bits: u128,
    samples_per_frame: f64,
    level: f32,
    phase: &mut bool,
) {
    for bit in 0..80u32 {
        let one = bits >> bit & 1 == 1;
        // Where this bit starts and ends, in samples, computed from the frame's own
        // boundaries rather than accumulated — the same rule the connectors' frame
        // deadline follows, and for the same reason: a per-bit rounding error summed
        // over eighty bits is a frame that is a sample and a half long.
        let start = (bit as f64 * samples_per_frame / 80.0).round() as usize;
        let end = ((bit + 1) as f64 * samples_per_frame / 80.0).round() as usize;
        let middle = (start + end) / 2;
        // A transition at the start of every bit; one more in the middle for a `1`.
        *phase = !*phase;
        for sample in start..end {
            if one && sample == middle {
                *phase = !*phase;
            }
            out.push(if *phase { level } else { -level });
        }
    }
}

/// A whole stream: `count` frames starting at a position, at a sample rate.
///
/// Used by the corpus and by the integration test, and nowhere in the console — a
/// station that generated timecode would be the thing everything else chases, which is
/// a feature nobody asked for.
pub fn encode(
    hours: u8,
    minutes: u8,
    seconds: u8,
    frames: u8,
    rate: LtcRate,
    count: u32,
    sample_rate: u32,
) -> Vec<f32> {
    let (num, den) = rate.seconds_per_frame();
    let samples_per_frame = sample_rate as f64 * num as f64 / den as f64;
    let mut out = Vec::with_capacity((samples_per_frame * count as f64) as usize + 16);
    let mut phase = false;
    let (mut h, mut m, mut s, mut f) = (hours, minutes, seconds, frames);
    for _ in 0..count {
        encode_frame(&mut out, frame_bits(h, m, s, f, rate.drops_frames()), samples_per_frame, 0.8, &mut phase);
        (h, m, s, f) = advance(h, m, s, f, rate);
    }
    out
}

/// The next frame number at a rate, dropping the two numbers 29.97df skips.
pub fn advance(hours: u8, minutes: u8, seconds: u8, frames: u8, rate: LtcRate) -> (u8, u8, u8, u8) {
    let (mut h, mut m, mut s, mut f) = (hours, minutes, seconds, frames);
    f += 1;
    if f as u32 >= rate.counts_per_second() {
        f = 0;
        s += 1;
        if s >= 60 {
            s = 0;
            m += 1;
            if m >= 60 {
                m = 0;
                h = (h + 1) % 24;
            }
            // The drop itself: at the top of every minute but every tenth, the
            // numbers `:00` and `:01` are never sent.
            if rate.drops_frames() && m % 10 != 0 {
                f = 2;
            }
        }
    }
    (h, m, s, f)
}

// ── Decoding ──────────────────────────────────────────────────────────────────

/// What the decoder thinks of the signal it is being fed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    /// Nothing that looks like timecode has arrived yet.
    Waiting,
    /// Frames are arriving and the sync word is landing where it should.
    Locked,
    /// Frames were arriving and have stopped, or stopped making sense.
    Lost,
}

/// A streaming biphase-mark decoder.
///
/// Fed whatever the sound card hands over, in whatever sizes, and answers with the
/// frames that completed inside that buffer. It holds no clock: the caller decides
/// what "now" was, which is what lets a test push a generated stream through it in one
/// call and get the same answers a sound card would produce over ten seconds.
pub struct Decoder {
    sample_rate: u32,
    /// The last eighty bits, newest at bit 79. See [`SYNC`].
    register: u128,
    bits_since_sync: u32,
    /// Running mean of the signal, which is what the crossings are found against —
    /// see the module header on level and DC.
    centre: f32,
    /// Which side of the centre the signal is on, with hysteresis, so a noisy zero
    /// crossing is one transition rather than nine.
    high: bool,
    /// How far above the centre the signal has been going, which is what the
    /// hysteresis band is a fraction of. Decays, so a stream that gets quieter is
    /// followed rather than latched at its loudest.
    excursion: f32,
    /// Samples since the last transition.
    run: u32,
    /// The current estimate of a full bit in samples, tracked from what arrives.
    bit_samples: f32,
    /// A half-bit interval is only half a `1`: the next one completes it.
    half_pending: bool,
    /// Samples pushed since this decoder was made, for [`LtcFrame::ended_at_sample`].
    position: u64,
    /// How many samples ago the last frame completed, for [`Decoder::lock`].
    since_frame: u64,
    seen_a_frame: bool,
}

/// How long a bit may be, as a fraction of the running estimate, and still count as a
/// full bit rather than a half or as noise.
const LONG: (f32, f32) = (0.75, 1.5);
const SHORT: (f32, f32) = (0.35, 0.75);

impl Decoder {
    pub fn new(sample_rate: u32) -> Decoder {
        Decoder {
            sample_rate,
            register: 0,
            bits_since_sync: 0,
            centre: 0.0,
            high: false,
            excursion: 0.0,
            run: 0,
            // Seeded at 30 fps, which is inside the tolerance of every rate this
            // decoder handles — 24 fps bits are 25% longer and the estimate walks on
            // to them inside a frame. Seeded rather than left at zero because a first
            // interval has nothing to be measured against.
            bit_samples: sample_rate as f32 / (30.0 * 80.0),
            half_pending: false,
            position: 0,
            since_frame: 0,
            seen_a_frame: false,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Whether timecode is arriving, given how long a caller is prepared to wait.
    ///
    /// `patience_ms` rather than a constant: a console chasing at 30 fps should call a
    /// gap of four frames a loss, and one at 24 fps has to wait longer for the same
    /// number of frames. The manager passes what the timeline's rate implies.
    pub fn lock(&self, patience_ms: u32) -> Lock {
        if !self.seen_a_frame {
            return Lock::Waiting;
        }
        let patience = self.sample_rate as u64 * patience_ms as u64 / 1000;
        if self.since_frame > patience {
            Lock::Lost
        } else {
            Lock::Locked
        }
    }

    /// Feed samples in; take whatever frames completed.
    ///
    /// Mono. A stereo interface hands over one channel — timecode on both channels of
    /// a pair is the same signal twice, and summing them would cancel it outright if
    /// one of them happened to be wired backwards.
    pub fn push(&mut self, samples: &[f32]) -> Vec<LtcFrame> {
        let mut out = Vec::new();
        for &sample in samples {
            self.position += 1;
            self.since_frame += 1;
            self.run += 1;

            // A running mean over about twenty milliseconds, which is where the two
            // pressures balance. The slowest thing LTC puts on a wire is half the bit
            // rate — 960 Hz at 24 fps — so a 50 Hz tracker follows a DC offset and
            // cannot follow the signal. Slower than this and a feed that arrives with
            // an offset already on it takes a second to decode, which is thirty
            // frames of a chase that has not started yet; that was the first version,
            // and a quiet signal riding a large offset never decoded at all inside a
            // short buffer.
            self.centre += (sample - self.centre) / (self.sample_rate as f32 / 50.0).max(1.0);
            let above = sample - self.centre;
            self.excursion = self.excursion.max(above.abs()) * 0.9999;
            // A tenth of the signal's own swing. Wide enough that dither and quiet
            // hiss do not cross it, narrow enough that a −20 dBu feed still does.
            let band = (self.excursion * 0.1).max(1e-5);

            let crossed = if self.high { above < -band } else { above > band };
            if !crossed {
                continue;
            }
            self.high = !self.high;
            let run = self.run;
            self.run = 0;
            self.take_interval(run, &mut out);
        }
        out
    }

    /// One transition-to-transition interval, classified against the running estimate.
    fn take_interval(&mut self, run: u32, out: &mut Vec<LtcFrame>) {
        let run = run as f32;
        let long = (self.bit_samples * LONG.0, self.bit_samples * LONG.1);
        let short = (self.bit_samples * SHORT.0, self.bit_samples * SHORT.1);

        if run >= long.0 && run <= long.1 {
            // A full bit period with no transition inside it: a zero. And the best
            // measurement of the bit period there is, so the estimate follows it.
            self.bit_samples += (run - self.bit_samples) * 0.05;
            self.half_pending = false;
            self.take_bit(false, out);
        } else if run >= short.0 && run < short.1 {
            if self.half_pending {
                self.half_pending = false;
                self.bit_samples += (run * 2.0 - self.bit_samples) * 0.05;
                self.take_bit(true, out);
            } else {
                self.half_pending = true;
            }
        } else {
            // Neither: noise, a dropout, or the tape stopping. The register is left
            // alone rather than cleared — the sync word is what re-synchronises, and
            // clearing would only delay it — but a half that never got its other half
            // must not pair with the next frame's first transition.
            self.half_pending = false;
        }
    }

    fn take_bit(&mut self, one: bool, out: &mut Vec<LtcFrame>) {
        self.register = (self.register >> 1) | ((one as u128) << 79);
        self.bits_since_sync = self.bits_since_sync.saturating_add(1);
        if (self.register >> 64) as u16 != SYNC {
            return;
        }
        // The sync word has landed. Eighty bits since the last one is a whole frame;
        // anything else is the first lock or a dropout, and the *bits* are still
        // good — the register holds one frame's worth whatever happened before it.
        let whole = self.bits_since_sync >= 80;
        self.bits_since_sync = 0;
        if !whole && self.seen_a_frame {
            // A short frame after a good one means bits were lost inside it, and the
            // fields would be a mixture of two frames. Dropped rather than reported.
            return;
        }
        if !whole && !self.seen_a_frame {
            // The very first sync of a stream, with a partial frame in front of it.
            // Nothing can be read out of it and the next one will be whole.
            self.seen_a_frame = true;
            self.since_frame = 0;
            return;
        }
        let bits = self.register;
        let frame = LtcFrame {
            hours: bcd(bits, 48, 4) + bcd(bits, 56, 2) * 10,
            minutes: bcd(bits, 32, 4) + bcd(bits, 40, 3) * 10,
            seconds: bcd(bits, 16, 4) + bcd(bits, 24, 3) * 10,
            frames: bcd(bits, 0, 4) + bcd(bits, 8, 2) * 10,
            drop_frame: bits >> 10 & 1 == 1,
            ended_at_sample: self.position,
        };
        // A BCD field can hold a nibble that is not a digit, and a hour of 47 is a
        // dropout that happened to end in a sync word rather than a position.
        if frame.hours > 23 || frame.minutes > 59 || frame.seconds > 59 || frame.frames > 39 {
            return;
        }
        self.seen_a_frame = true;
        self.since_frame = 0;
        out.push(frame);
    }
}

fn bcd(bits: u128, at: u32, width: u32) -> u8 {
    ((bits >> at) & ((1u128 << width) - 1)) as u8
}

#[cfg(test)]
mod tests;
