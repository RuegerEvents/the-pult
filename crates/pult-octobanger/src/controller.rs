//! One controller: eight channels, one sequence, one trigger, and what to play.
//!
//! The field names are the app's own (`_delayEndSecs`, `_hiOut`), because they are what
//! a reader of the file sees; the meanings are the firmware's, because the firmware is
//! what the prop actually did with them. Where those two disagree with what the app's
//! screen says, the screen is noted beside the field.

use serde::Serialize;

use crate::nrbf::{self, Document, Object, Value};
use crate::Error;

/// How long one frame of a sequence is.
///
/// Fixed by the firmware ("I am going to start by fixing FPS at 20fps, or 50ms per
/// frame") and never stored in a file. The app keeps one byte per frame; the firmware
/// is sent the same thing run-length encoded, as (state, frames) pairs.
pub const FRAME_MS: u32 = 50;

/// The app's controller, as far as a prop is concerned.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Controller {
    pub name: String,
    pub comment: String,
    /// The eight outputs, channel 1 first.
    pub channels: Vec<Channel>,
    /// How long the sequence is. Set in the app, and in every file seen so far set from
    /// the length of the scare sound.
    pub end_ms: u32,
    /// One byte per frame, bit *n* being channel *n + 1* on — **only the frames that
    /// play**, which are those starting at or before [`Controller::end_ms`]. The app's
    /// buffer is six thousand bytes whatever the sequence, and what lies past the end is
    /// whatever an earlier, longer edit left there (Schaltschrank has five frames of it).
    /// Not serialized: [`Controller::spans`] is the form anybody reading it wants.
    #[serde(skip)]
    pub frames: Vec<u8>,
    pub pin_mapping: PinMapping,
    pub trigger: TriggerLevel,
    pub timing_offset: TimingOffset,
    /// `_delayEndSecs`: after a scare the prop goes cold for this long and a trigger
    /// does nothing — the firmware's `RESET_DELAY_SECS`, blinked away a second at a
    /// time.
    pub reset_delay_s: u32,
    /// `_bootDelaySecs`: how long after power-up before the prop takes a trigger.
    pub boot_delay_s: u32,
    pub media: Media,
}

/// One output.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// What the app calls it: "1 - Drop", "Solenoid 2".
    pub name: String,
    /// The notes column, which is often the wiring ("NC - 15 | NO - 16").
    pub comment: String,
    /// `_hiOut`, which is the app's *Active Low* column: every file seen so far has it
    /// checked exactly where this is true. The firmware flips that output at the pin
    /// (`bitRead(valIn, x) ^ TTL_TYPES[x]`), which is a fact about the relay board on
    /// the Arduino and not about the show: the sequence's bit is still "on".
    pub active_low: bool,
}

/// `PinMapping`: which Arduino pins the eight outputs were wired to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PinMapping {
    /// Outputs on D2–D9, trigger in on D11.
    DefaultTtl,
    /// Laid out for a four-relay shield: D7, D6, D5, D4, then D8–D11; trigger in on A0.
    Shield,
    /// Anything else the firmware was built with. The file does not say what.
    Other(i64),
}

/// `TriggerAmbientLevel`: what the trigger input rests at.
///
/// The names are the app's and they describe the *resting* level, so they read
/// backwards: the firmware reports `Low` as "Low (PIR or + trigger)" — it rests low and
/// a trigger takes it high — and `Hi` as "Hi (to gnd trigger)", resting on its pull-up
/// until a contact pulls it to ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TriggerLevel {
    Low,
    Hi,
    Other(i64),
}

/// `TimingOffsetType`: a correction for Arduino clones whose clock runs slow.
///
/// `Default` takes 0.405 ms off every frame (`TIMING_OFFSETS = {-.405, 0}`), because a
/// clone measured with a scope lost half a second a minute. Either way the *intended*
/// frame is fifty milliseconds, which is what a sequence means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TimingOffset {
    Default,
    None,
    Other(i64),
}

/// `SerialMediaType`: what the media player wired to the serial pin is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MediaKind {
    None,
    /// A Catalex MP3 board, playing the scare file on a trigger and nothing else.
    Mp3ScareOnly,
    /// The same board looping an ambient file, which a trigger interrupts with the
    /// scare and which comes back when the sequence ends.
    Mp3AmbientScare,
    /// A MedeaWiz Sprite video player, told to play on a trigger.
    Video,
    Other(i64),
}

/// What the media player plays, and how.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub kind: MediaKind,
    /// The ambient loop's file name, which the app keeps but does not embed.
    pub ambient: Option<String>,
    /// The scare's file name.
    pub scare: Option<String>,
    /// The Catalex board's volume, 1–30.
    pub volume: u8,
    /// `_mediaDelay`, in hundredths of a second: how long the firmware waits after
    /// starting the sound before the first frame, on top of a fixed 100 ms — to cover
    /// the MP3 board's slow start, which a console playing its own audio does not have.
    pub delay_cs: u8,
}

impl Controller {
    /// Read an `.OBC`.
    pub fn read(bytes: &[u8]) -> Result<Controller, Error> {
        if bytes.first() == Some(&b'<') || bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            return Err(Error::DocumentFormat);
        }
        let document = nrbf::read(bytes)?;
        let root = document.root().ok_or(Error::Missing("root object"))?;
        if !root.class.ends_with("OctoBangerController") {
            return Err(Error::NotAController(root.class.clone()));
        }
        Reading { document: &document }.controller(root)
    }

    /// Whether a channel is on in a frame. Channel 1 is the first.
    pub fn on(&self, channel: usize, frame: usize) -> bool {
        channel >= 1 && channel <= 8 && self.frames.get(frame).is_some_and(|f| f >> (channel - 1) & 1 == 1)
    }

    /// Whether a channel is ever on in the sequence.
    pub fn used(&self, channel: usize) -> bool {
        (0..self.frames.len()).any(|frame| self.on(channel, frame))
    }

    /// How long, in all, a channel is on.
    pub fn on_ms(&self, channel: usize) -> u32 {
        self.spans(channel).iter().map(|(from, to)| to - from).sum()
    }

    /// Where a channel is on, as `[from, to)` in milliseconds, cut at the end.
    ///
    /// The end is where the firmware sets every output low (`set_ambient_out`), so a
    /// channel still on in the last frame goes off at [`Controller::end_ms`], not at the
    /// end of a frame that may lie past it.
    pub fn spans(&self, channel: usize) -> Vec<(u32, u32)> {
        let mut spans = Vec::new();
        let mut from = None;
        for frame in 0..self.frames.len() {
            let at = frame as u32 * FRAME_MS;
            match (self.on(channel, frame), from) {
                (true, None) => from = Some(at),
                (false, Some(start)) => {
                    spans.push((start, at));
                    from = None;
                }
                _ => {}
            }
        }
        if let Some(start) = from {
            if start < self.end_ms {
                spans.push((start, self.end_ms));
            }
        }
        spans
    }

    /// A channel as change points: its state from the start, every change, and off at
    /// the end — the shape a recording is stored in.
    pub fn changes(&self, channel: usize) -> Vec<(u32, bool)> {
        let mut points = vec![(0, false)];
        for (from, to) in self.spans(channel) {
            match points.last_mut() {
                Some(last) if last.0 == from => last.1 = true,
                _ => points.push((from, true)),
            }
            points.push((to, false));
        }
        points
    }

    /// The Arduino pin each output was wired to, where the mapping says.
    pub fn pins(&self) -> Option<[&'static str; 8]> {
        match self.pin_mapping {
            PinMapping::DefaultTtl => Some(["D2", "D3", "D4", "D5", "D6", "D7", "D8", "D9"]),
            PinMapping::Shield => Some(["D7", "D6", "D5", "D4", "D8", "D9", "D10", "D11"]),
            PinMapping::Other(_) => None,
        }
    }

    /// The pin the trigger came in on.
    pub fn trigger_pin(&self) -> Option<&'static str> {
        match self.pin_mapping {
            PinMapping::DefaultTtl => Some("D11"),
            PinMapping::Shield => Some("A0"),
            PinMapping::Other(_) => None,
        }
    }
}

struct Reading<'a> {
    document: &'a Document,
}

impl<'a> Reading<'a> {
    fn object(&self, of: &'a Object, name: &'static str) -> Result<&'a Object, Error> {
        match self.document.get(of, name) {
            Some(Value::Object(object)) => Ok(object),
            _ => Err(Error::Missing(name)),
        }
    }

    fn text(&self, of: &Object, name: &'static str) -> String {
        match self.document.get(of, name) {
            Some(Value::String(text)) => text.clone(),
            _ => String::new(),
        }
    }

    fn integer(&self, of: &Object, name: &'static str) -> Result<i64, Error> {
        match self.document.get(of, name) {
            Some(Value::Int(n)) => Ok(*n),
            Some(Value::UInt(n)) => Ok(*n as i64),
            _ => Err(Error::Missing(name)),
        }
    }

    /// An enum is written as an object of its own with one member, `value__`.
    fn enumeration(&self, of: &'a Object, name: &'static str) -> Result<i64, Error> {
        self.integer(self.object(of, name)?, "value__")
    }

    fn controller(&self, root: &'a Object) -> Result<Controller, Error> {
        let sequence = self.object(root, "_sequence")?;
        let end_ms = self.integer(sequence, "_endMS")?.clamp(0, u32::MAX as i64) as u32;
        let data = match self.document.get(sequence, "_baseChannelData") {
            Some(Value::Bytes(bytes)) => bytes.as_slice(),
            _ => return Err(Error::Missing("_baseChannelData")),
        };
        let playing = (end_ms / FRAME_MS) as usize + 1;
        let frames = data[..playing.min(data.len())].to_vec();

        let list = self.object(root, "_baseChannels")?;
        let size = self.integer(list, "_size")?.max(0) as usize;
        let items = match self.document.get(list, "_items") {
            Some(Value::Array(items)) => items,
            _ => return Err(Error::Missing("_baseChannels")),
        };
        let mut channels = Vec::new();
        for item in items.iter().take(size) {
            if let Value::Object(channel) = self.document.resolve(item) {
                channels.push(Channel {
                    name: self.text(channel, "_displayname"),
                    comment: self.text(channel, "_comment"),
                    active_low: matches!(self.document.get(channel, "_hiOut"), Some(Value::Bool(true))),
                });
            }
        }

        let media = self.object(root, "_mediaProps")?;
        let named = |name: &'static str| Some(self.text(media, name)).filter(|t| !t.trim().is_empty());

        Ok(Controller {
            name: self.text(root, "_displayname"),
            comment: self.text(root, "_comment"),
            channels,
            end_ms,
            frames,
            pin_mapping: match self.enumeration(root, "_pinMapping")? {
                0 => PinMapping::DefaultTtl,
                1 => PinMapping::Shield,
                other => PinMapping::Other(other),
            },
            trigger: match self.enumeration(root, "_triggerAmbientLevel")? {
                0 => TriggerLevel::Low,
                1 => TriggerLevel::Hi,
                other => TriggerLevel::Other(other),
            },
            timing_offset: match self.enumeration(root, "_timingOffsetType")? {
                0 => TimingOffset::Default,
                1 => TimingOffset::None,
                other => TimingOffset::Other(other),
            },
            reset_delay_s: self.integer(root, "_delayEndSecs").unwrap_or(0).max(0) as u32,
            boot_delay_s: self.integer(root, "_bootDelaySecs").unwrap_or(0).max(0) as u32,
            media: Media {
                kind: match self.enumeration(media, "_serialMediaType")? {
                    0 => MediaKind::None,
                    1 => MediaKind::Mp3ScareOnly,
                    2 => MediaKind::Mp3AmbientScare,
                    3 => MediaKind::Video,
                    other => MediaKind::Other(other),
                },
                ambient: named("_ambientFileName"),
                scare: named("_audioFileName"),
                volume: self.integer(media, "_volume").unwrap_or(30).clamp(0, 255) as u8,
                delay_cs: self.integer(media, "_mediaDelay").unwrap_or(0).clamp(0, 255) as u8,
            },
        })
    }
}
