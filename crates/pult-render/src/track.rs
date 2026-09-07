//! A recording: what a parameter was doing, as a function of a position on a
//! timeline.
//!
//! A track is the fourth thing that can drive a parameter, and it is the only one
//! that is *data* rather than a description — a fade is six numbers and a track is
//! however many change points somebody's console sent while the tape was rolling. So
//! it is content-addressed, kept in the asset store, and read by this crate: the
//! station decodes it to put it on a wire and the browser hands the same bytes to the
//! wasm build, which is the rule the rest of the evaluator already lives by. Two
//! parsers of one format would drift exactly where a recording is longest.
//!
//! # The format
//!
//! Hand-written little-endian, because the alternative was a serde format with a
//! version of its own to keep track of. `PLTK`, a `u16` version, then a count of
//! keys; each key is a fixture uuid, its parameter key, and its change points; each
//! point is a position in milliseconds and one tagged value.
//!
//! **Change points, not samples.** A source is sampled at whatever rate it sends at,
//! and a value that did not move writes nothing — which is what makes a forty-minute
//! recording of a rig where six heads move the size of six heads moving. It is also
//! why replay is **stepwise**: the recorder wrote a point exactly when the value
//! changed, so holding the latest point until the next one reproduces what came down
//! the wire, and interpolating between them would invent motion nobody sent.
//!
//! **Nothing before the first point.** A track that starts thirty seconds in says
//! nothing at all about the first thirty seconds, and [`sample`] answers `None` there
//! so the layer below — an effect, a fade, the home value — goes on showing through.
//! Answering the first point early would be the recording asserting a value it never
//! captured.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::value::ParameterValue;

/// One change: a position on the timeline, and what the parameter became there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackPoint {
    /// Milliseconds from the start of the timeline. `u32`, which is 49 days —
    /// comfortably longer than any recording anybody makes of a show.
    pub ms: u32,
    pub value: ParameterValue,
}

/// Everything one recording holds about one parameter of one fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackKey {
    pub fixture_id: Uuid,
    /// The parameter key, spelled the way `live_fades` and `home_values` spell it.
    pub key: String,
    /// Ascending in `ms`. The encoder writes them in order and the decoder does not
    /// sort: a file whose points are out of order is a file somebody else wrote
    /// wrongly, and quietly reordering it would hide that.
    pub points: Vec<TrackPoint>,
}

/// A recording.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub keys: Vec<TrackKey>,
}

/// Why a track could not be read.
///
/// Every arm names the byte offset it failed at, because the one thing anybody wants
/// to know about a corrupt recording is how much of it was good.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackError {
    /// The first four bytes are not `PLTK`, so this is not a track at all.
    NotATrack,
    /// A version this build does not know how to read.
    Version(u16),
    /// The file ends in the middle of something.
    Truncated { at: usize },
    /// A value tag this build has no meaning for.
    UnknownValue { at: usize, tag: u8 },
    /// A string that is not UTF-8.
    NotText { at: usize },
}

impl std::fmt::Display for TrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrackError::NotATrack => write!(f, "not a track: the magic is wrong"),
            TrackError::Version(v) => write!(f, "a track of version {v}, which this build cannot read"),
            TrackError::Truncated { at } => write!(f, "the track ends part way through, at byte {at}"),
            TrackError::UnknownValue { at, tag } => {
                write!(f, "a value of kind {tag} at byte {at}, which this build has no meaning for")
            }
            TrackError::NotText { at } => write!(f, "a name at byte {at} that is not UTF-8"),
        }
    }
}

/// The four bytes at the front, so a file that is not one says so rather than being
/// read as an empty recording.
pub const MAGIC: [u8; 4] = *b"PLTK";

/// The only version this build writes.
pub const VERSION: u16 = 1;

// The value tags. Numbers rather than a derived discriminant, so adding a variant to
// `ParameterValue` cannot silently renumber what is already written to disk.
const TAG_FLOAT: u8 = 0;
const TAG_INT: u8 = 1;
const TAG_BOOL: u8 = 2;
const TAG_COLOR: u8 = 3;
const TAG_TEXT: u8 = 4;

/// Write a recording out.
pub fn encode(track: &Track) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(track.keys.len() as u32).to_le_bytes());
    for key in &track.keys {
        out.extend_from_slice(key.fixture_id.as_bytes());
        write_short_string(&mut out, &key.key);
        out.extend_from_slice(&(key.points.len() as u32).to_le_bytes());
        for point in &key.points {
            out.extend_from_slice(&point.ms.to_le_bytes());
            write_value(&mut out, &point.value);
        }
    }
    out
}

fn write_short_string(out: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    out.extend_from_slice(&(bytes.len().min(u16::MAX as usize) as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..bytes.len().min(u16::MAX as usize)]);
}

fn write_value(out: &mut Vec<u8>, value: &ParameterValue) {
    match value {
        ParameterValue::Float(v) => {
            out.push(TAG_FLOAT);
            out.extend_from_slice(&v.to_le_bytes());
        }
        ParameterValue::Int(v) => {
            out.push(TAG_INT);
            out.extend_from_slice(&v.to_le_bytes());
        }
        ParameterValue::Bool(on) => {
            out.push(TAG_BOOL);
            out.push(u8::from(*on));
        }
        ParameterValue::Color { r, g, b, overrides } => {
            out.push(TAG_COLOR);
            out.extend_from_slice(&r.to_le_bytes());
            out.extend_from_slice(&g.to_le_bytes());
            out.extend_from_slice(&b.to_le_bytes());
            out.extend_from_slice(&(overrides.len().min(u16::MAX as usize) as u16).to_le_bytes());
            for (name, level) in overrides.iter().take(u16::MAX as usize) {
                write_short_string(out, name);
                out.extend_from_slice(&level.to_le_bytes());
            }
        }
        ParameterValue::Text(text) => {
            out.push(TAG_TEXT);
            let bytes = text.as_bytes();
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        }
    }
}

/// Read a recording back.
pub fn decode(bytes: &[u8]) -> Result<Track, TrackError> {
    let mut cursor = Cursor { bytes, at: 0 };
    if cursor.take(4)? != MAGIC {
        return Err(TrackError::NotATrack);
    }
    let version = cursor.u16()?;
    if version != VERSION {
        return Err(TrackError::Version(version));
    }
    let key_count = cursor.u32()? as usize;
    let mut keys = Vec::with_capacity(key_count.min(4096));
    for _ in 0..key_count {
        let mut id = [0u8; 16];
        id.copy_from_slice(cursor.take(16)?);
        let fixture_id = Uuid::from_bytes(id);
        let key = cursor.short_string()?;
        let point_count = cursor.u32()? as usize;
        let mut points = Vec::with_capacity(point_count.min(65_536));
        for _ in 0..point_count {
            let ms = cursor.u32()?;
            let value = cursor.value()?;
            points.push(TrackPoint { ms, value });
        }
        keys.push(TrackKey { fixture_id, key, points });
    }
    Ok(Track { keys })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], TrackError> {
        let end = self.at.checked_add(n).ok_or(TrackError::Truncated { at: self.at })?;
        let slice = self.bytes.get(self.at..end).ok_or(TrackError::Truncated { at: self.at })?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, TrackError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, TrackError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, TrackError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i32(&mut self) -> Result<i32, TrackError> {
        Ok(self.u32()? as i32)
    }

    fn f32(&mut self) -> Result<f32, TrackError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn short_string(&mut self) -> Result<String, TrackError> {
        let at = self.at;
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| TrackError::NotText { at })
    }

    fn value(&mut self) -> Result<ParameterValue, TrackError> {
        let at = self.at;
        let tag = self.u8()?;
        Ok(match tag {
            TAG_FLOAT => ParameterValue::Float(self.f32()?),
            TAG_INT => ParameterValue::Int(self.i32()?),
            TAG_BOOL => ParameterValue::Bool(self.u8()? != 0),
            TAG_COLOR => {
                let r = self.f32()?;
                let g = self.f32()?;
                let b = self.f32()?;
                let count = self.u16()? as usize;
                let mut overrides = BTreeMap::new();
                for _ in 0..count {
                    let name = self.short_string()?;
                    overrides.insert(name, self.f32()?);
                }
                ParameterValue::Color { r, g, b, overrides }
            }
            TAG_TEXT => {
                let at = self.at;
                let len = self.u32()? as usize;
                let bytes = self.take(len)?;
                ParameterValue::Text(
                    String::from_utf8(bytes.to_vec()).map_err(|_| TrackError::NotText { at })?,
                )
            }
            tag => return Err(TrackError::UnknownValue { at, tag }),
        })
    }
}

// ── Playing one back ──────────────────────────────────────────────────────────

/// Where a timeline's playhead is, as a function of a moment.
///
/// The shape `Sequence::went_at` has, with a rate on it: an anchor, the position the
/// playhead was at when it was anchored, and how fast it is moving. Nothing ticks, so
/// a station, a browser and a connector all arrive at the same position for the same
/// millisecond without exchanging anything, and a paused timeline is one with an
/// anchor nobody is reading.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transport {
    /// The console millisecond the playhead was last placed.
    pub anchor_ms: u64,
    /// Where it was placed, in timeline milliseconds.
    pub position_at_anchor_ms: u64,
    /// How fast it runs. 1.0 is real time; the audio half will vary it slightly to
    /// chase LTC.
    pub rate: f32,
}

impl Default for Transport {
    fn default() -> Self {
        Transport { anchor_ms: 0, position_at_anchor_ms: 0, rate: 1.0 }
    }
}

impl Transport {
    /// The playhead at a console millisecond.
    ///
    /// Before the anchor it is *at* the anchor rather than behind it: a station whose
    /// clock has not caught up with the leader's would otherwise run the playhead
    /// backwards through the start of the recording, and a track sampled at a
    /// negative position is a track that says nothing when it should be holding its
    /// first value.
    pub fn position_at(&self, now_ms: u64) -> u64 {
        if now_ms <= self.anchor_ms {
            return self.position_at_anchor_ms;
        }
        let elapsed = (now_ms - self.anchor_ms) as f64 * self.rate.max(0.0) as f64;
        self.position_at_anchor_ms.saturating_add(elapsed as u64)
    }
}

/// One playing recording, as it bears on one parameter.
#[derive(Debug, Clone, Copy)]
pub struct TrackAt<'a> {
    pub points: &'a [TrackPoint],
    pub transport: Transport,
}

impl<'a> TrackAt<'a> {
    /// What this recording is asserting at a console millisecond, if anything.
    pub fn value_at(&self, now_ms: u64) -> Option<&'a ParameterValue> {
        sample(self.points, self.transport.position_at(now_ms))
    }
}

/// The latest change point at or before `position_ms`.
///
/// Stepwise, and `None` before the first point — see the module header for both. A
/// linear scan from the back rather than a binary search, because the caller is a
/// frame walking a parameter's own points and the overwhelmingly common case is a
/// handful of them; a recording long enough for the difference to be measurable will
/// want an index kept beside it rather than a cleverer scan here.
pub fn sample(points: &[TrackPoint], position_ms: u64) -> Option<&ParameterValue> {
    points
        .iter()
        .rev()
        .find(|point| point.ms as u64 <= position_ms)
        .map(|point| &point.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(pairs: &[(u32, f32)]) -> Vec<TrackPoint> {
        pairs
            .iter()
            .map(|(ms, v)| TrackPoint { ms: *ms, value: ParameterValue::Float(*v) })
            .collect()
    }

    #[test]
    fn a_track_survives_being_written_and_read() {
        let mut overrides = BTreeMap::new();
        overrides.insert("White".to_string(), 0.25);
        let track = Track {
            keys: vec![
                TrackKey {
                    fixture_id: Uuid::from_u128(7),
                    key: "Intensity".into(),
                    points: points(&[(0, 0.0), (1_000, 0.5), (2_500, 1.0)]),
                },
                TrackKey {
                    fixture_id: Uuid::from_u128(9),
                    key: "ColorRgb".into(),
                    points: vec![
                        TrackPoint {
                            ms: 40,
                            value: ParameterValue::Color { r: 1.0, g: 0.5, b: 0.0, overrides },
                        },
                        TrackPoint { ms: 80, value: ParameterValue::Int(3) },
                        TrackPoint { ms: 120, value: ParameterValue::Bool(true) },
                        TrackPoint { ms: 160, value: ParameterValue::Text("CUE 4".into()) },
                    ],
                },
            ],
        };

        assert_eq!(decode(&encode(&track)), Ok(track));
    }

    #[test]
    fn an_empty_track_is_still_a_track() {
        assert_eq!(decode(&encode(&Track::default())), Ok(Track::default()));
    }

    /// The magic earns its place here: a `.glb` or a half-downloaded file has to say
    /// what it is rather than decoding as a recording of nothing.
    #[test]
    fn something_that_is_not_a_track_says_so_rather_than_reading_as_empty() {
        assert_eq!(decode(b"glTF\0\0\0\0"), Err(TrackError::NotATrack));
        assert_eq!(decode(&[]), Err(TrackError::Truncated { at: 0 }));

        let mut wrong_version = encode(&Track::default());
        wrong_version[4] = 9;
        assert_eq!(decode(&wrong_version), Err(TrackError::Version(9)));
    }

    #[test]
    fn a_track_that_stops_part_way_through_names_where_it_stopped() {
        let track = Track {
            keys: vec![TrackKey {
                fixture_id: Uuid::nil(),
                key: "Intensity".into(),
                points: points(&[(0, 0.0), (10, 1.0)]),
            }],
        };
        let whole = encode(&track);
        let cut = &whole[..whole.len() - 3];
        assert!(matches!(decode(cut), Err(TrackError::Truncated { .. })));
    }

    #[test]
    fn sampling_holds_the_latest_point_and_says_nothing_before_the_first() {
        let points = points(&[(1_000, 0.2), (2_000, 0.8)]);
        assert_eq!(sample(&points, 0), None, "the recording had not started");
        assert_eq!(sample(&points, 999), None);
        assert_eq!(sample(&points, 1_000), Some(&ParameterValue::Float(0.2)));
        assert_eq!(sample(&points, 1_999), Some(&ParameterValue::Float(0.2)), "stepwise");
        assert_eq!(sample(&points, 2_000), Some(&ParameterValue::Float(0.8)));
        assert_eq!(sample(&points, 9_999_999), Some(&ParameterValue::Float(0.8)));
        assert_eq!(sample(&[], 5), None);
    }

    #[test]
    fn the_playhead_is_a_function_of_the_moment_and_never_runs_backwards() {
        let transport = Transport { anchor_ms: 1_000, position_at_anchor_ms: 500, rate: 1.0 };
        assert_eq!(transport.position_at(0), 500, "before the anchor it sits at the anchor");
        assert_eq!(transport.position_at(1_000), 500);
        assert_eq!(transport.position_at(1_250), 750);

        let double = Transport { rate: 2.0, ..transport };
        assert_eq!(double.position_at(1_250), 1_000);

        let stopped = Transport { rate: 0.0, ..transport };
        assert_eq!(stopped.position_at(9_000), 500);
    }
}
