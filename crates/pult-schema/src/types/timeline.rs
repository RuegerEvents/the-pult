//! A timeline: a position, and what is written against it.
//!
//! What "timecode" comes to here. A timeline says *when* things happen; a sequence
//! stays the stack. An `events` entry is a position and a Go — into any sequence — so
//! one song can drive three of them and a cue goes on belonging to nobody's clock.
//! `FollowMode::Timecode` was removed when this arrived rather than being reimplemented
//! on top of it: a cue that says "at 00:04:11" is one place a position can be written
//! down, and this is the other, better one, because the position is a *list* an
//! operator can see and drag.
//!
//! **Running state is a SYNCED anchor, the shape `Sequence::went_at` has.** `running`,
//! `anchor_ms`, `position_at_anchor_ms` and `rate`, and the position on every station
//! and in every browser is [`Timeline::position_at`] of the moment. Nothing ticks
//! anywhere, which is the same rule the rest of the console lives by and the reason a
//! tablet's playhead agrees with the console's without either of them being told.
//!
//! **`play`, `stop`, `locate` and `record` carry `at`**, for the reason a Go does:
//! every station runs the same command from the same arguments, so all of them anchor
//! the same millisecond rather than whenever each actor got round to it.
//!
//! **Only the leader fires an event.** A crossed event becomes an ordinary Go,
//! replicated like any other, with `at` set to the wall millisecond the event fell at —
//! so a follower runs the cue it is told about rather than a cue it worked out
//! separately and anchored a few milliseconds off. See `model/timelines.rs`.
//!
//! **And where there is sound, the sound is the reference.** One station plays the
//! audio — [`Timeline::node_id`], or the leader — and measures the playhead off the
//! *audio callback's own sample clock* rather than off the wall. When the two differ by
//! more than twenty milliseconds it rewrites the anchor here, so every station and
//! every browser follows the loudspeakers instead of a clock nobody can hear. Under
//! twenty it does nothing at all: the anchor is the show's, and a station that
//! rewrote it every buffer would be jitter nothing downstream could smooth out. The
//! station's half is `infra/audio`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{events::operation::NodeId, pult_commands, PultSchema};

/// Where a timeline's position comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TimelineSource {
    /// The console's own clock, moved by the transport commands.
    Internal,
    /// Linear timecode arriving on an audio input.
    ///
    /// The frame rate is **declared, never sniffed**: 29.97 drop-frame and 30 differ
    /// by one frame in a thousand, and a console that guessed would be right all
    /// afternoon and wrong at the performance. What the wire carries is a frame
    /// *count*; whether thirty of them is a second is not in the signal at all.
    ///
    /// `offset_frames` is signed and is subtracted from the decoded position, because
    /// a show that starts at `01:00:00:00` is the normal case rather than the odd one:
    /// timecode from an hour is how a reel says "this is the programme and not the
    /// leader".
    Ltc { fps: LtcRate, offset_frames: i64 },
}

impl Default for TimelineSource {
    fn default() -> Self {
        TimelineSource::Internal
    }
}

/// The frame rates timecode is actually generated at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum LtcRate {
    F24,
    F25,
    F30,
    /// 29.97 non-drop: the frame *count* runs at 30 and the clock is slow.
    F2997,
    /// 29.97 drop-frame, which skips frame numbers so the clock stays honest.
    F2997Df,
}

/// What the beat detector found, before anybody agreed to it.
///
/// Kept beside `grid` rather than written into it, and that is the whole discipline:
/// **the detector proposes and the operator confirms.** A console that silently
/// rewrote the grid of a show somebody had already programmed against would be worse
/// than one that could not detect anything at all — so this is drawn over the waveform,
/// and one button turns it into segments.
///
/// Milliseconds rather than seconds, so it is the same unit as everything else written
/// against a position and nothing in a panel has to convert.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Detected {
    pub beats_ms: Vec<u32>,
    /// The "one" of each bar. A subset of `beats_ms`: the detector snaps each downbeat
    /// onto the nearest beat before reporting it, so a grid's bar lines always sit on
    /// its beat lines.
    pub downbeats_ms: Vec<u32>,
}

/// One stretch of the beat grid: a tempo, from a position, in a time signature.
///
/// Segments rather than one tempo, because a song has a rallentando in it and a show
/// has an interval. The detector proposes these and the operator drags them; nothing
/// here computes one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GridSegment {
    pub at_ms: u32,
    pub bpm: f32,
    pub beats_per_bar: u8,
}

/// A named position: "chorus 2", "blackout", "the bit with the confetti".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Marker {
    pub id: Uuid,
    pub at_ms: u32,
    pub name: String,
}

/// What a timeline does to a sequence, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TimelineAction {
    GoToCue { sequence_id: Uuid, cue_id: Uuid },
    GoNext { sequence_id: Uuid },
    Off { sequence_id: Uuid },
}

impl TimelineAction {
    /// Which sequence this acts on, which is what makes taking the tracked state
    /// possible: a locate applies the latest event *per sequence* and fires nothing
    /// else.
    pub fn sequence_id(&self) -> Uuid {
        match self {
            TimelineAction::GoToCue { sequence_id, .. }
            | TimelineAction::GoNext { sequence_id }
            | TimelineAction::Off { sequence_id } => *sequence_id,
        }
    }

    /// The registered command this becomes, and its arguments beside `at`.
    pub fn command(&self) -> (&'static str, serde_json::Value) {
        match self {
            TimelineAction::GoToCue { cue_id, .. } => {
                ("goToCue", serde_json::json!({ "cueId": cue_id }))
            }
            TimelineAction::GoNext { .. } => ("goNext", serde_json::json!({})),
            TimelineAction::Off { .. } => ("off", serde_json::json!({})),
        }
    }
}

/// A Go written against a position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineEvent {
    pub id: Uuid,
    pub at_ms: u32,
    pub action: TimelineAction,
}

/// One recording on a timeline.
///
/// `asset` is the sha of a `application/vnd.pult.track` in the content-addressed store,
/// which is the whole of what a track is: the codec is
/// [`pult_render::track`], so the station reading it for a wire and the browser reading
/// it for the screen are the same parser compiled twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineTrack {
    pub id: Uuid,
    pub name: String,
    /// The asset sha.
    pub asset: String,
    /// Which input it was recorded from, where it was recorded here. Kept so a take
    /// can be re-recorded from the same place without anybody remembering which cable
    /// it came in on.
    pub input_id: Option<Uuid>,
    /// Nudge, in milliseconds. Signed: a take is as often early as late.
    pub offset_ms: i32,
    pub enabled: bool,
}

// Commands live beside the type for the orphan rule's sake, the way `Sequence`'s do.
#[pult_commands]
impl Timeline {
    /// Start running from where the playhead is, or from `fromMs`.
    ///
    /// Args: `{ "at": <console unix ms>, "fromMs": <timeline ms> }`, both optional.
    /// `at` is carried for the reason a Go carries it: every station anchors the same
    /// millisecond rather than whenever its own actor got here, and the position is a
    /// function of that anchor everywhere.
    #[pult_command(args = "{ at?: number, fromMs?: number }")]
    pub fn play(
        &mut self,
        args: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let at = at_arg(&args);
        // Where it is *now* — which for a stopped timeline is where it was left and
        // for a running one is where it has got to, so pressing play twice does not
        // jump the playhead back to the last anchor.
        let from = args
            .get("fromMs")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| self.position_at(at));
        self.position_at_anchor_ms = from;
        self.anchor_ms = at;
        self.running = true;
        Ok(())
    }

    /// Stop, leaving the playhead where it had got to.
    ///
    /// Not a locate to zero: an operator who stops in the middle of a song and presses
    /// play again means from there.
    #[pult_command(args = "{ at?: number }")]
    pub fn stop(
        &mut self,
        args: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let at = at_arg(&args);
        self.position_at_anchor_ms = self.position_at(at);
        self.anchor_ms = at;
        self.running = false;
        Ok(())
    }

    /// Put the playhead somewhere, running or not.
    ///
    /// Args: `{ "positionMs": <timeline ms>, "at"?: <console unix ms> }`.
    #[pult_command(args = "{ positionMs: number, at?: number }")]
    pub fn locate(
        &mut self,
        args: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let position_ms = args
            .get("positionMs")
            .and_then(|v| v.as_u64())
            .ok_or("locate needs a positionMs")?;
        self.position_at_anchor_ms = position_ms;
        self.anchor_ms = at_arg(&args);
        Ok(())
    }

    /// Arm, or disarm, recording from an input.
    ///
    /// Args: `{ "inputId": "<uuid>" | null }`. Arming is a *state*, not an act: the
    /// station holding that input's socket records from the next play to the next
    /// stop, so an operator can arm at half past six and press play at the top of the
    /// show. `null` disarms, which is also what the recording station writes back when
    /// it has stored the take.
    #[pult_command(args = "{ inputId: string | null }")]
    pub fn record(
        &mut self,
        args: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.recording = match args.get("inputId") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(serde_json::from_value(value.clone())?),
        };
        Ok(())
    }
}

/// A position, and everything written against it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "timelines")]
pub struct Timeline {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// The audio asset this timeline runs against, where there is one.
    ///
    /// Played by the station [`Timeline::node_id`] names, off the audio callback's own
    /// sample clock — see `infra/audio` — so the sound is the reference and everything
    /// else follows the anchor it writes. A timeline with **no** audio is an ordinary
    /// stopwatch and runs exactly as it did before any of this existed, which is what
    /// "timecode without timecode" means and is the property that must not be lost.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub audio: Option<String>,
    /// The reduced waveform of [`Timeline::audio`], as its own asset sha.
    ///
    /// Computed once by whichever station first held the file and written here, so
    /// every browser in the building fetches one small asset rather than decoding
    /// fifty megabytes. A field rather than a lookup because the alternative — deriving
    /// the peaks asset's name from the audio's — would be a second content-addressing
    /// scheme, and a tablet would have to ask for a sha that might not exist yet.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub peaks: Option<String>,
    /// What the beat detector last found. See [`Detected`] on why this is not `grid`.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub detected: Option<Detected>,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub source: TimelineSource,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub grid: Vec<GridSegment>,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub events: Vec<TimelineEvent>,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub tracks: Vec<TimelineTrack>,
    /// A speed master this timeline drives from its grid.
    ///
    /// While the timeline runs, the **leader** writes that master's `bpm` and `t0` at
    /// each grid segment it crosses — a bounded step in phase, which is the discipline
    /// `types::speedmaster` already lives by. When the timeline stops the master keeps
    /// its last tempo, because an operator who has been chasing a song still wants the
    /// chases running at its tempo in the applause.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub speed_master: Option<Uuid>,
    /// Which station plays the audio, `None` for the leader — the rule outputs follow.
    ///
    /// And which station listens for LTC, where [`TimelineSource::Ltc`] is the source:
    /// one machine has both the speakers and the timecode input, and splitting them
    /// would mean chasing a clock this station cannot hear.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub node_id: Option<NodeId>,

    /// The transport, replicated so every station and every browser reads the same
    /// playhead out of the same four numbers.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub running: bool,
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub anchor_ms: u64,
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub position_at_anchor_ms: u64,
    #[serde(default = "one")]
    #[pult(lifecycle = SYNCED)]
    pub rate: f32,
    /// The input a take is being recorded from, if one is armed.
    ///
    /// SYNCED rather than LOCAL because the station that *arms* it is usually not the
    /// station holding the socket: an operator at the booth arms a recording from the
    /// stage rack's input, and the rack has to hear about it.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub recording: Option<Uuid>,
}

fn one() -> f32 {
    1.0
}

impl Timeline {
    /// Where the playhead is at a console millisecond.
    ///
    /// A stopped timeline sits at its anchor whatever the moment is, which is what
    /// makes "stop" and "pause" the same act: the position is remembered as a number
    /// rather than as a clock somebody has to keep.
    pub fn position_at(&self, now_ms: u64) -> u64 {
        if !self.running {
            return self.position_at_anchor_ms;
        }
        self.transport().position_at(now_ms)
    }

    /// The transport, in the form the evaluator wants.
    pub fn transport(&self) -> pult_render::Transport {
        pult_render::Transport {
            anchor_ms: self.anchor_ms,
            position_at_anchor_ms: self.position_at_anchor_ms,
            rate: self.rate,
        }
    }

    /// The events this timeline crosses going forwards from `after_ms` to `to_ms`,
    /// in order.
    ///
    /// Half open at the bottom and closed at the top, so an event fires exactly once
    /// however many passes straddle it — and so an event at position zero fires when
    /// the timeline is played from zero, which is where every show's first Go is.
    pub fn crossed(&self, after_ms: Option<u64>, to_ms: u64) -> Vec<&TimelineEvent> {
        let mut crossed: Vec<&TimelineEvent> = self
            .events
            .iter()
            .filter(|event| {
                let at = event.at_ms as u64;
                at <= to_ms && after_ms.is_none_or(|after| at > after)
            })
            .collect();
        crossed.sort_by_key(|event| event.at_ms);
        crossed
    }

    /// The latest event at or before a position, per sequence.
    ///
    /// What a locate takes rather than firing everything it jumped over: a jump into
    /// the second chorus should leave each sequence on the cue it would have been on,
    /// which is the last thing this timeline said about it and not a burst of forty
    /// Gos. Nothing crossed *backwards* fires at all.
    pub fn tracked_at(&self, position_ms: u64) -> Vec<&TimelineEvent> {
        let mut sorted: Vec<&TimelineEvent> =
            self.events.iter().filter(|e| e.at_ms as u64 <= position_ms).collect();
        sorted.sort_by_key(|event| event.at_ms);

        let mut latest: std::collections::BTreeMap<Uuid, &TimelineEvent> = Default::default();
        for event in sorted {
            latest.insert(event.action.sequence_id(), event);
        }
        let mut tracked: Vec<&TimelineEvent> = latest.into_values().collect();
        tracked.sort_by_key(|event| event.at_ms);
        tracked
    }

    /// The next event after a position, for the engine's `next_wake`.
    pub fn next_event_after(&self, position_ms: u64) -> Option<&TimelineEvent> {
        self.events
            .iter()
            .filter(|event| event.at_ms as u64 > position_ms)
            .min_by_key(|event| event.at_ms)
    }
}

/// The `at` a transport command was given, or now.
///
/// The same rule [`crate::types::sequence`]'s `went_at` follows, and it has to be the
/// same rule: a timeline's anchor and a cue's `went_at` are compared against each
/// other every time an event fires.
fn at_arg(args: &serde_json::Value) -> u64 {
    args.get("at").and_then(|v| v.as_u64()).unwrap_or_else(crate::clock::now_ms)
}

/// Whether timecode is arriving, as a panel says it.
///
/// Three states rather than a boolean, because "nothing has arrived yet" and "it was
/// arriving and stopped" are different things to be told at ten past seven: the first
/// is a cable that was never plugged in and the second is one that has fallen out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum LtcLock {
    Waiting,
    Locked,
    Lost,
}

/// What one timeline's audio is doing on this station: the LOCAL `audio_status` path.
///
/// LOCAL for the reason [`super::output::OutputStatus`] is: it describes a device on
/// *this* machine, and the station beside it running the same show has its own answer —
/// usually "nothing, somebody else is playing it". Published once a second beside the
/// output and input statuses.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioStatus {
    pub name: String,
    /// Sound is coming out of this station right now.
    pub playing: bool,
    /// The output device that was opened, by the name `audio.devices` lists it under.
    pub device: Option<String>,
    /// Why there is no sound. A device this machine has not got is named here rather
    /// than logged and forgotten — the rule `Network::bind` follows for a cable, and
    /// for the same reason: from the desk, a wrong device name and a broken file look
    /// identical.
    pub fault: Option<String>,
    /// How far the playhead measured off the sound card is from where the show's
    /// anchor says it should be. Signed, and normally a millisecond or two — the
    /// figure that says whether this station is keeping up.
    pub drift_ms: i32,
    /// Where timecode chasing has got to, where this timeline chases any.
    pub ltc: Option<LtcStatus>,
}

/// What the timecode input is doing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LtcStatus {
    pub lock: LtcLock,
    pub device: Option<String>,
    pub fault: Option<String>,
    /// The last position decoded, in milliseconds, offset applied.
    pub position_ms: Option<u64>,
    /// What the *signal* says its frame rate is doing, against what the show declared.
    /// `Some(true)` from a drop-frame generator feeding a show set to 25 is a fault
    /// worth naming, and nothing else on the panel would show it.
    pub drop_frame: Option<bool>,
}

/// Every timeline's audio status, keyed by timeline id: the LOCAL `audio_status` path.
pub type AudioStatuses = std::collections::BTreeMap<String, AudioStatus>;

#[cfg(test)]
mod tests {
    use super::*;

    fn a_timeline() -> Timeline {
        Timeline {
            id: Uuid::from_u128(1),
            name: "Act 1".into(),
            audio: None,
            peaks: None,
            detected: None,
            source: TimelineSource::Internal,
            grid: Vec::new(),
            markers: Vec::new(),
            events: Vec::new(),
            tracks: Vec::new(),
            speed_master: None,
            node_id: None,
            running: false,
            anchor_ms: 0,
            position_at_anchor_ms: 0,
            rate: 1.0,
            recording: None,
        }
    }

    fn an_event(at_ms: u32, sequence: u128) -> TimelineEvent {
        TimelineEvent {
            id: Uuid::from_u128(at_ms as u128 + 1000),
            at_ms,
            action: TimelineAction::GoNext { sequence_id: Uuid::from_u128(sequence) },
        }
    }

    #[test]
    fn a_stopped_timeline_sits_where_it_was_left() {
        let mut timeline = a_timeline();
        timeline.position_at_anchor_ms = 4_000;
        timeline.anchor_ms = 1_000;
        assert_eq!(timeline.position_at(9_999_999), 4_000);
    }

    #[test]
    fn playing_anchors_at_the_moment_it_was_given_and_the_position_follows_it() {
        let mut timeline = a_timeline();
        timeline.play(serde_json::json!({ "at": 1_000u64 })).unwrap();
        assert!(timeline.running);
        assert_eq!(timeline.position_at(1_000), 0);
        assert_eq!(timeline.position_at(3_500), 2_500);

        timeline.stop(serde_json::json!({ "at": 3_500u64 })).unwrap();
        assert!(!timeline.running);
        assert_eq!(timeline.position_at(9_000), 2_500, "stop is where it got to");

        timeline.play(serde_json::json!({ "at": 10_000u64 })).unwrap();
        assert_eq!(timeline.position_at(10_500), 3_000, "and play carries on from there");
    }

    #[test]
    fn playing_from_somewhere_starts_there() {
        let mut timeline = a_timeline();
        timeline.play(serde_json::json!({ "at": 1_000u64, "fromMs": 30_000u64 })).unwrap();
        assert_eq!(timeline.position_at(2_000), 31_000);
    }

    #[test]
    fn locating_moves_a_running_timeline_without_stopping_it() {
        let mut timeline = a_timeline();
        timeline.play(serde_json::json!({ "at": 1_000u64 })).unwrap();
        timeline.locate(serde_json::json!({ "positionMs": 60_000u64, "at": 2_000u64 })).unwrap();
        assert!(timeline.running);
        assert_eq!(timeline.position_at(2_000), 60_000);
        assert_eq!(timeline.position_at(2_500), 60_500);
    }

    #[test]
    fn arming_a_recording_names_the_input_and_null_disarms() {
        let mut timeline = a_timeline();
        let input = Uuid::from_u128(7);
        timeline.record(serde_json::json!({ "inputId": input })).unwrap();
        assert_eq!(timeline.recording, Some(input));
        timeline.record(serde_json::json!({ "inputId": serde_json::Value::Null })).unwrap();
        assert_eq!(timeline.recording, None);
    }

    /// Half open at the bottom: an event fires once however the passes fall, and an
    /// event at zero still fires when the timeline is played from zero.
    #[test]
    fn an_event_is_crossed_exactly_once() {
        let mut timeline = a_timeline();
        timeline.events = vec![an_event(0, 1), an_event(1_000, 1), an_event(2_000, 2)];

        let first: Vec<u32> = timeline.crossed(None, 0).iter().map(|e| e.at_ms).collect();
        assert_eq!(first, vec![0], "a play from zero takes the event at zero");

        let next: Vec<u32> = timeline.crossed(Some(0), 2_000).iter().map(|e| e.at_ms).collect();
        assert_eq!(next, vec![1_000, 2_000]);

        assert!(timeline.crossed(Some(2_000), 2_000).is_empty(), "and not again");
    }

    /// A locate is not forty Gos: each sequence gets the last thing this timeline said
    /// about it, and nothing that was jumped over backwards fires at all.
    #[test]
    fn locating_takes_the_latest_event_per_sequence_and_no_more() {
        let mut timeline = a_timeline();
        timeline.events = vec![
            an_event(0, 1),
            an_event(1_000, 1),
            an_event(1_500, 2),
            an_event(9_000, 1),
        ];

        let tracked: Vec<u32> = timeline.tracked_at(5_000).iter().map(|e| e.at_ms).collect();
        assert_eq!(tracked, vec![1_000, 1_500], "one per sequence, in position order");

        assert!(timeline.tracked_at(0).len() == 1);
    }

    #[test]
    fn the_next_event_is_a_deadline() {
        let mut timeline = a_timeline();
        timeline.events = vec![an_event(1_000, 1), an_event(5_000, 1)];
        assert_eq!(timeline.next_event_after(0).map(|e| e.at_ms), Some(1_000));
        assert_eq!(timeline.next_event_after(1_000).map(|e| e.at_ms), Some(5_000));
        assert_eq!(timeline.next_event_after(5_000).map(|e| e.at_ms), None);
    }
}
