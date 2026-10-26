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
    /// timecode from an hour is how a reel says "this is the program and not the
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

/// What a timeline does, and when: to a sequence, to another timeline, or to a variable.
///
/// The timeline and variable acts are the flow's own (`flow::TriggerAction`), so a
/// timeline can hand over to the next one — a loop that a PIR stops in favour of a
/// scare, and a scare whose last event plays the loop again — without a flow having
/// to know how long the scare is. Added at Heinz Horror Garten, 2026-10-25.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TimelineAction {
    GoToCue { sequence_id: Uuid, cue_id: Uuid },
    GoNext { sequence_id: Uuid },
    Off { sequence_id: Uuid },
    /// Play a timeline from its start — this one too, which is a restart.
    PlayTimeline {
        timeline_id: Uuid,
        #[serde(default)]
        if_running: crate::types::flow::IfRunning,
    },
    StopTimeline { timeline_id: Uuid },
    SetVariable { variable_id: Uuid, op: crate::types::variable::VariableOp },
}

impl TimelineAction {
    /// Which sequence this acts on, which is what makes taking the tracked state
    /// possible: a locate applies the latest event *per sequence* and fires nothing
    /// else. `None` for the acts on timelines and variables, which a locate never
    /// replays: jumping into the middle of a scare must not start the loop it ends with.
    pub fn sequence_id(&self) -> Option<Uuid> {
        match self {
            TimelineAction::GoToCue { sequence_id, .. }
            | TimelineAction::GoNext { sequence_id }
            | TimelineAction::Off { sequence_id } => Some(*sequence_id),
            _ => None,
        }
    }

    /// The registered sequence command this becomes, and its arguments beside `at`.
    /// `None` for an act that is not on a sequence; see [`Self::as_flow_action`].
    pub fn command(&self) -> Option<(&'static str, serde_json::Value)> {
        match self {
            TimelineAction::GoToCue { cue_id, .. } => {
                Some(("goToCue", serde_json::json!({ "cueId": cue_id })))
            }
            TimelineAction::GoNext { .. } => Some(("goNext", serde_json::json!({}))),
            TimelineAction::Off { .. } => Some(("off", serde_json::json!({}))),
            _ => None,
        }
    }

    /// The same act as a flow would do it, for the ones a flow can do: one executor,
    /// so a timeline playing a timeline and a flow playing one cannot disagree about
    /// what "if running" means.
    pub fn as_flow_action(&self) -> Option<crate::types::flow::TriggerAction> {
        use crate::types::flow::TriggerAction;
        match self {
            TimelineAction::PlayTimeline { timeline_id, if_running } => Some(TriggerAction::PlayTimeline {
                timeline_id: *timeline_id,
                if_running: *if_running,
            }),
            TimelineAction::StopTimeline { timeline_id } => {
                Some(TriggerAction::StopTimeline { timeline_id: *timeline_id })
            }
            TimelineAction::SetVariable { variable_id, op } => {
                Some(TriggerAction::SetVariable { variable_id: *variable_id, op: *op })
            }
            _ => None,
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
    /// it came in on. `None` is a take recorded from the programmer.
    pub input_id: Option<Uuid>,
    /// Nudge, in milliseconds. Signed: a take is as often early as late.
    pub offset_ms: i32,
    pub enabled: bool,
    /// The parameter of the recording this row plays: a fixture (or, for a recorded
    /// sequence level, the sequence) and a parameter key. A row per parameter, so each
    /// is cropped, sliced and replaced on its own; `None` is a take stored before that,
    /// playing every parameter in its recording until the station splits it.
    #[serde(default)]
    pub fixture_id: Option<Uuid>,
    #[serde(default)]
    pub key: Option<String>,
    /// The stretch of the timeline this row plays, in timeline milliseconds, `None`
    /// open on that side. Inside it the row is completely in charge of its parameter —
    /// see `pult_render::compose`.
    #[serde(default)]
    pub from_ms: Option<u32>,
    #[serde(default)]
    pub to_ms: Option<u32>,
}

impl TimelineTrack {
    /// This row over its decoded recording, for composing.
    pub fn part<'a>(&'a self, track: &'a pult_render::Track) -> pult_render::Part<'a> {
        pult_render::Part {
            track,
            fixture_id: self.fixture_id,
            key: self.key.as_deref(),
            from_ms: self.from_ms,
            to_ms: self.to_ms,
            offset_ms: self.offset_ms,
        }
    }

    /// Whether this row plays a parameter. A row from before rows were per parameter
    /// plays all of them, which only its recording can say.
    pub fn plays(&self, fixture_id: Uuid, key: &str) -> bool {
        self.fixture_id == Some(fixture_id) && self.key.as_deref() == Some(key)
    }

    /// Whether this row's window reaches anywhere into `[lo, hi)`.
    fn overlaps(&self, lo: u32, hi: u32) -> bool {
        self.from_ms.unwrap_or(0) < hi && self.to_ms.is_none_or(|to| to > lo)
    }
}

/// Take `[lo, hi)` out of every row playing a parameter: a row inside it goes, a row
/// across one edge is cropped back to it, and a row across both becomes two.
///
/// What re-recording a parameter does to the rows already playing it, and a slice
/// deleted from the middle of one. `rows` have to be per parameter already; a row
/// playing every parameter of its recording is left alone, since only the recording
/// says whether it plays this one.
pub fn cut_rows(rows: &mut Vec<TimelineTrack>, fixture_id: Uuid, key: &str, lo: u32, hi: u32) {
    let mut out = Vec::with_capacity(rows.len() + 1);
    for row in rows.drain(..) {
        if !row.plays(fixture_id, key) || !row.overlaps(lo, hi) {
            out.push(row);
            continue;
        }
        let from = row.from_ms.unwrap_or(0);
        if from < lo {
            out.push(TimelineTrack { to_ms: Some(lo), ..row.clone() });
        }
        if row.to_ms.is_none_or(|to| to > hi) {
            // The second half is a row of its own, so it needs a name of its own.
            let id = if from < lo { Uuid::new_v4() } else { row.id };
            out.push(TimelineTrack { id, from_ms: Some(hi), ..row });
        }
    }
    *rows = out;
}

/// A row that plays every parameter of its recording, as a row per parameter: the same
/// asset, offset and window, the first keeping the row's id.
pub fn split_row(row: &TimelineTrack, keys: &[(Uuid, String)]) -> Vec<TimelineTrack> {
    keys.iter()
        .enumerate()
        .map(|(at, (fixture_id, key))| TimelineTrack {
            id: if at == 0 { row.id } else { Uuid::new_v4() },
            fixture_id: Some(*fixture_id),
            key: Some(key.clone()),
            ..row.clone()
        })
        .collect()
}

/// A flash key pressed while a take rolled: one value on one parameter, for a while.
///
/// Not a stretch of a recording. A press is one act with a length, so it is kept as
/// one — an object an operator can move, stretch and delete — rather than as two change
/// points in a file nobody can edit. It goes on top of every take and cuts none
/// (`pult_render::compose_with`), so a button pressed during a second pass adds to the
/// first instead of erasing it. Told apart from a fader by `model::takes::Pass`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineFlash {
    pub id: Uuid,
    pub fixture_id: Uuid,
    /// The parameter key, spelled the way a take's rows spell it.
    pub key: String,
    pub value: crate::types::fixture::ParameterValue,
    pub at_ms: u32,
    pub duration_ms: u32,
}

impl TimelineFlash {
    /// This flash, for composing.
    pub fn flash(&self) -> pult_render::Flash<'_> {
        pult_render::Flash {
            fixture_id: self.fixture_id,
            key: &self.key,
            value: &self.value,
            from_ms: self.at_ms,
            to_ms: self.at_ms.saturating_add(self.duration_ms.max(1)),
        }
    }
}

/// A recording that has stopped and is waiting to be looked at.
///
/// **Nothing a take records is merged until somebody says so.** A pass is stored as an
/// asset at once — the bytes are safe the moment the timeline stops — and listed here;
/// the page that armed the recording asks which of it to keep, and Merge writes the
/// chosen keys into the rows in one gesture. PERSISTED, so a take nobody has answered
/// for yet survives a failover and a restart alike, and so undoing a Merge puts the
/// take back up for review rather than losing it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingTake {
    /// The page that armed the recording (`Timeline::armed_by` at the time), which is
    /// the one that opens the review by itself. Every other page offers it.
    #[serde(default)]
    pub armed_by: Option<String>,
    /// One per pass, in the order they were recorded: a looping rehearsal is a pass per
    /// lap, and a later lap is merged over an earlier one.
    pub passes: Vec<PendingPass>,    /// The cues taken while it rolled — a Go, a Back, an Off — as the events they would
    /// be on the timeline, waiting with the rest. Recorded as the state the act left the
    /// sequence in, so a Go played back goes to the cue that came next when it was taken.
    #[serde(default)]
    pub events: Vec<TimelineEvent>,
}

/// One pass of a pending take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingPass {
    /// The recording's sha.
    pub asset: String,
    /// The input it came off, where it came off a wire.
    #[serde(default)]
    pub input_id: Option<Uuid>,
    /// Each key the pass recorded and where its stretch starts. A key from the
    /// programmer starts at its first touch; a take off a wire, where every key was
    /// listening from the start, at the start.
    pub keys: Vec<PendingKey>,
    /// Where every key's stretch ends: where the pass stopped.
    pub out_ms: u32,
    /// The flash keys pressed during the pass, ready to be added.
    #[serde(default)]
    pub flashes: Vec<TimelineFlash>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingKey {
    pub fixture_id: Uuid,
    pub key: String,
    pub from_ms: u32,
}

/// Write a pass into a timeline's rows: every row already playing one of its keys is
/// cut out of `[from, out)` — each key's own `from`, its first touch — and each key
/// gets a row of its own over exactly that stretch. The old take plays up to the touch
/// and from the stop, and keys the pass never touched are left as they were.
pub fn store_take(rows: &mut Vec<TimelineTrack>, pass: &PendingPass, keep: impl Fn(Uuid, &str) -> bool) {
    for key in pass.keys.iter().filter(|k| keep(k.fixture_id, &k.key)) {
        // A key touched where the pass ended still replaced the moment it was at.
        let out = pass.out_ms.max(key.from_ms.saturating_add(1));
        cut_rows(rows, key.fixture_id, &key.key, key.from_ms, out);
        rows.push(TimelineTrack {
            id: Uuid::new_v4(),
            name: String::new(),
            asset: pass.asset.clone(),
            input_id: pass.input_id,
            offset_ms: 0,
            enabled: true,
            fixture_id: Some(key.fixture_id),
            key: Some(key.key.clone()),
            from_ms: Some(key.from_ms),
            to_ms: Some(out),
        });
    }
}

impl PendingTake {
    /// Merge what was chosen: the take of each key `takes` keeps and the flashes of
    /// each key `flashes` keeps, pass by pass in recording order. What was not chosen
    /// is dropped with the rest of the review.
    pub fn merge(
        &self,
        rows: &mut Vec<TimelineTrack>,
        into: &mut Vec<TimelineFlash>,
        takes: impl Fn(Uuid, &str) -> bool,
        flashes: impl Fn(Uuid, &str) -> bool,
    ) {
        for pass in &self.passes {
            store_take(rows, pass, &takes);
            into.extend(pass.flashes.iter().filter(|f| flashes(f.fixture_id, &f.key)).cloned());
        }
    }

    /// The cues it caught that were chosen, onto a timeline's events in time order.
    pub fn merge_events(&self, events: &mut Vec<TimelineEvent>, keep: impl Fn(Uuid) -> bool) {
        events.extend(self.events.iter().filter(|e| keep(e.id)).cloned());
        events.sort_by_key(|e| e.at_ms);
    }

    /// Whether there is anything in it to answer for.
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty() && self.events.is_empty()
    }
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

    /// Arm, or disarm, a recording.
    ///
    /// Args: `{ "inputId": "<uuid>" | null }` records what that input hears, and
    /// `{ "programmer": true, "release"?: bool }` records what the operator does — every
    /// programmer key that changes while the timeline rolls. Arming is a *state*, not
    /// an act: the recording station records from the next play to the next stop, so
    /// an operator can arm at half past six and press play at the top of the show.
    /// Nothing named, or `inputId: null`, disarms, which is also what the recording
    /// station writes back when it has stored the take.
    ///
    /// `release` (default true) gives the keys a programmer pass recorded back to the
    /// take when it is stored. Without it the fader the operator is still holding beats
    /// the take it just made, for ever, because the programmer is above a track.
    ///
    /// `armedBy` is the arming page's own id, so that page is the one that opens the
    /// review when the take stops (see [`PendingTake`]).
    #[pult_command(args = "{ inputId?: string | null, programmer?: boolean, release?: boolean, armedBy?: string }")]
    pub fn record(
        &mut self,
        args: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(by) = args.get("armedBy").and_then(|v| v.as_str()) {
            self.armed_by = Some(by.to_string());
        }
        if args.get("programmer").and_then(|v| v.as_bool()) == Some(true) {
            let release = args.get("release").and_then(|v| v.as_bool()).unwrap_or(true);
            self.recording = Some(RecordSource::Programmer { release });
            return Ok(());
        }
        self.recording = match args.get("inputId") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(RecordSource::Input(serde_json::from_value(value.clone())?)),
        };
        Ok(())
    }
}

/// What a take is recorded from.
///
/// Two sources and one shape at the end of either: decoded values against a position.
/// An input is somebody else's console on a wire, decoded through the patch; the
/// programmer is the operator at this one, already a value. Neither is bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum RecordSource {
    /// What an input hears, recorded by the station holding its socket.
    Input(Uuid),
    /// Every programmer key that changes while the timeline rolls, recorded by the
    /// leader. `release` gives those keys back to the take when it is stored.
    Programmer { release: bool },
}

/// What one station is recording onto one timeline: the LOCAL `takes` path, keyed by
/// timeline id.
///
/// LOCAL because it is a fact about the station recording — the leader for the
/// programmer, the station holding the socket for an input — and a take in memory is
/// on that machine and no other. A page on another station sees the timeline armed and
/// not this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TakeStatus {
    pub source: RecordSource,
    /// Keys touched in the pass in progress.
    pub keys: u32,
    /// Change points in the pass in progress.
    pub points: u32,
    /// Passes already stored by this take — one per lap of a looping timeline.
    pub passes: u32,
}

pub type TakeStatuses = std::collections::BTreeMap<String, TakeStatus>;

impl RecordSource {
    /// The input, where this records from one.
    pub fn input(&self) -> Option<Uuid> {
        match self {
            RecordSource::Input(id) => Some(*id),
            RecordSource::Programmer { .. } => None,
        }
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
    /// The sound on this timeline: files at positions, each going somewhere.
    ///
    /// Played by the station [`Timeline::node_id`] names, off the audio callback's own
    /// sample clock — see `infra/audio` — so the sound is the reference and everything
    /// else follows the anchor it writes. A timeline with **no** clips is an ordinary
    /// stopwatch and runs exactly as it did before any of this existed, which is what
    /// "timecode without timecode" means and is the property that must not be lost.
    ///
    /// A list rather than one file because a scare is a scream in one zone *and* a bang
    /// in another, and making that two timelines started together would be two things
    /// to keep in step that one clock already keeps in step for free.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub clips: Vec<AudioClip>,
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
    /// The flash keys recorded onto this timeline, over every take. An `Option` only
    /// because a non-`Option` column added to a show that already exists is one the
    /// show cannot be opened with; `None` is none. See [`TimelineFlash`].
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub flashes: Option<Vec<TimelineFlash>>,
    /// A recording waiting to be reviewed. See [`PendingTake`].
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub pending: Option<PendingTake>,
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
    /// Where the timeline ends, and what it does there. `None` runs for ever, as a
    /// timeline always has.
    ///
    /// A scare is a timeline, and a scare has to *finish*: until it does it is still
    /// running, and a flow told to ignore a running timeline would ignore it for the
    /// rest of the night. Acted on by the leader, as an event is, and anchored on the
    /// wall millisecond the end fell at rather than whenever the pass got there — so a
    /// loop does not drift by a pass's lateness every time round.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub end: Option<TimelineEnd>,

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
    /// What a take is being recorded from, if one is armed.
    ///
    /// SYNCED rather than LOCAL because the station that *arms* it is usually not the
    /// station recording: an operator at the booth arms a recording from the stage
    /// rack's input, and the rack has to hear about it; a tablet arms the programmer,
    /// and the leader records it.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub recording: Option<RecordSource>,
    /// The page that armed the recording, by the id it gave itself — the one that
    /// opens the review when the take stops. SYNCED with `recording`, for the same
    /// reason: the station recording is usually not the one that page is on.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub armed_by: Option<String>,
}

fn one() -> f32 {
    1.0
}

impl Timeline {
    /// The flashes, as a list however they are stored.
    pub fn all_flashes(&self) -> &[TimelineFlash] {
        self.flashes.as_deref().unwrap_or_default()
    }

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
            if let Some(sequence) = event.action.sequence_id() {
                latest.insert(sequence, event);
            }
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

/// Where a timeline ends, and what it does when it gets there.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineEnd {
    pub at_ms: u64,
    pub then: AtEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AtEnd {
    /// Stop with the playhead on the end.
    Stop,
    /// Start again from zero: an ambient bed.
    Loop,
}

/// One file on a timeline: where it starts, what of it plays, how loud, and where to.
///
/// Positions are milliseconds like everything else on a timeline. The trim is
/// `offset_ms` into the file and `length_ms` of it from there, `None` being the rest of
/// the file, so a clip dragged shorter keeps its file rather than becoming another one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioClip {
    pub id: Uuid,
    /// What it is called on the timeline: the file's name when it was added.
    #[serde(default)]
    pub name: String,
    /// The audio asset, by sha256.
    pub asset: String,
    /// The reduced waveform of [`AudioClip::asset`], as its own asset sha.
    ///
    /// Computed once by whichever station first held the file and written here, so
    /// every browser in the building fetches one small asset rather than decoding
    /// fifty megabytes. A field rather than a lookup because the alternative — deriving
    /// the peaks asset's name from the audio's — would be a second content-addressing
    /// scheme, and a tablet would have to ask for a sha that might not exist yet.
    #[serde(default)]
    pub peaks: Option<String>,
    /// Where the clip starts on the timeline.
    #[serde(default)]
    pub start_ms: u64,
    /// How far into the file it starts.
    #[serde(default)]
    pub offset_ms: u64,
    /// How much of the file it plays, from `offset_ms`. `None` is to the end.
    #[serde(default)]
    pub length_ms: Option<u64>,
    /// In decibels, so zero is the file as it was mixed and a fader reads the way every
    /// other fader in the building does.
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub fade_in_ms: u32,
    /// Not applied to a looped clip, which has no last time round to fade out of.
    #[serde(default)]
    pub fade_out_ms: u32,
    /// Go round the trimmed region for as long as the timeline runs: an ambient bed.
    #[serde(default)]
    pub looped: bool,
    pub destination: AudioDestination,
}

impl AudioClip {
    /// The gain as the mixer wants it.
    pub fn gain(&self) -> f32 {
        10f32.powf(self.gain_db / 20.0)
    }
}

/// Where a clip's sound comes out.
///
/// Per clip, because one scare plays out of the zone speaker and a prop at once. A zone
/// rather than channel numbers, because a channel number names one machine's sound card
/// and the station playing a timeline can change in the middle of it — see
/// `types::zone`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AudioDestination {
    /// A speaker zone, by id: whichever channels the playing station maps it to, and
    /// whichever node loudspeakers are in it. A mono file goes to each channel; a file
    /// with as many channels, or a multiple, goes across them in order; anything else
    /// is its downmix on each.
    Zone { zone: Uuid },
    /// An OpenHaunt node's loudspeaker, by the node's serial: the key the device list,
    /// the fixture's address and every MQTT topic already use. A clip played by the
    /// node rather than mixed here — from its own flash, or a stream.
    ///
    /// Which of a node's outputs is not here, because it cannot be: a node has one I2S
    /// bus and every output plays the same mix. Enabling one is a fact about the node.
    Node {
        serial: String,
        #[serde(default)]
        mode: NodeAudioMode,
    },
}

/// How a clip reaches a node.
///
/// Per clip and chosen by the operator, never worked out: a clip that grew past the
/// node's store and quietly became a stream would change its latency and start
/// depending on the wifi, with nothing at the desk saying so.
///
/// **Streamed by default** (changed at the gig, 2026-10-25): picking a node for a clip
/// used to mean an upload first — megabytes into the node's flash before anybody could
/// switch it to a stream. Storing is now the deliberate choice it should have been.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum NodeAudioMode {
    /// Uploaded to the node's flash ahead of time and started there at a show time.
    /// Immune to the network once it is there, and the right thing for a scare.
    Clip,
    /// Sent as it plays, over RTP from the station driving the nodes, a playout delay
    /// ahead. No size limit, and it needs the network for as long as it sounds.
    #[default]
    Stream,
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
    /// Clips that are not playing anywhere and why, keyed by clip id: a zone this
    /// station has not mapped, a channel the device has not got, a file that would not
    /// decode, a node that is not on the network. Per clip because the rest of the
    /// timeline goes on sounding.
    #[serde(default)]
    pub clip_faults: std::collections::BTreeMap<String, String>,
    /// Clips sounding through some of their destination and not the rest, keyed by clip
    /// id, each with what failed: a zone's Dante channels playing while one of its node
    /// speakers is offline. Kept apart from `clip_faults` because a clip half heard and
    /// a silent one are different things to fix in a hurry. See `zone::clip_health`.
    #[serde(default)]
    pub clip_partial: std::collections::BTreeMap<String, Vec<String>>,
    /// Which of this station's output channels each clip comes out of, keyed by clip id:
    /// its zone, resolved through this station's own map. What a panel shows as "plays
    /// on 3–4 here", and what a promoted spare's row changes to.
    #[serde(default)]
    pub clip_channels: std::collections::BTreeMap<String, Vec<u16>>,
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

/// The output device this station mixes every clip into: the LOCAL `audio_output` path.
///
/// What the clip editor needs to offer channels to route to. LOCAL for the reason
/// `audio_status` is, which has a consequence worth knowing: a browser is told about
/// the device of the station it is connected to, and the station playing a timeline
/// may be another one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioOutput {
    /// Open now, because something on this station has clips to play.
    pub open: bool,
    pub device: Option<String>,
    pub channels: u16,
    pub sample_rate: u32,
    pub fault: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_row(from: Option<u32>, to: Option<u32>) -> TimelineTrack {
        TimelineTrack {
            id: Uuid::from_u128(9),
            name: "Take 1".into(),
            asset: "abc".into(),
            input_id: None,
            offset_ms: 0,
            enabled: true,
            fixture_id: Some(Uuid::from_u128(2)),
            key: Some("Jaw".into()),
            from_ms: from,
            to_ms: to,
        }
    }

    fn windows(rows: &[TimelineTrack]) -> Vec<(Option<u32>, Option<u32>)> {
        rows.iter().map(|r| (r.from_ms, r.to_ms)).collect()
    }

    #[test]
    fn a_cut_crops_splits_and_removes_and_leaves_other_parameters_alone() {
        let jaw = Uuid::from_u128(2);
        let mut rows = vec![a_row(None, None)];
        cut_rows(&mut rows, jaw, "Jaw", 100, 200);
        assert_eq!(windows(&rows), vec![(None, Some(100)), (Some(200), None)]);
        assert_ne!(rows[0].id, rows[1].id, "the second half is a row of its own");

        cut_rows(&mut rows, jaw, "Jaw", 50, 300);
        assert_eq!(windows(&rows), vec![(None, Some(50)), (Some(300), None)]);

        cut_rows(&mut rows, jaw, "Jaw", 0, 50);
        assert_eq!(windows(&rows), vec![(Some(300), None)], "a row inside the cut goes");

        cut_rows(&mut rows, jaw, "Head", 0, 1_000);
        assert_eq!(rows.len(), 1, "another parameter's cut does not touch it");
    }

    fn a_timeline() -> Timeline {
        Timeline {
            id: Uuid::from_u128(1),
            name: "Act 1".into(),
            clips: Vec::new(),
            detected: None,
            source: TimelineSource::Internal,
            grid: Vec::new(),
            markers: Vec::new(),
            events: Vec::new(),
            tracks: Vec::new(),
            flashes: None,
            pending: None,
            speed_master: None,
            node_id: None,
            end: None,
            running: false,
            anchor_ms: 0,
            position_at_anchor_ms: 0,
            rate: 1.0,
            recording: None,
            armed_by: None,
        }
    }

    fn an_event(at_ms: u32, sequence: u128) -> TimelineEvent {
        TimelineEvent {
            id: Uuid::from_u128(at_ms as u128 + 1000),
            at_ms,
            action: TimelineAction::GoNext { sequence_id: Uuid::from_u128(sequence) },
        }
    }

    /// An opened show reads a timeline back from its columns, and the transport is not
    /// among them. It has to come back at serde's rate of one, not `f32::default()`:
    /// at zero a timeline plays, says it is running, and never moves.
    #[test]
    fn a_timeline_read_back_from_the_showfile_plays_at_rate_one() {
        use crate::sql::{ColumnGetter, PultSqlRow, SqlLiteral};
        struct Row(std::collections::HashMap<&'static str, SqlLiteral>);
        impl ColumnGetter for Row {
            fn get_text(&self, col: &str) -> Option<String> {
                match self.0.get(col)? { SqlLiteral::Text(s) => Some(s.clone()), _ => None }
            }
            fn get_int(&self, col: &str) -> Option<i64> {
                match self.0.get(col)? { SqlLiteral::Int(i) => Some(*i), _ => None }
            }
            fn get_real(&self, col: &str) -> Option<f64> {
                match self.0.get(col)? { SqlLiteral::Real(r) => Some(*r), _ => None }
            }
        }
        let timeline = a_timeline();
        let row = Row(Timeline::column_names().iter().copied().zip(timeline.to_sql_values()).collect());
        let read = Timeline::from_columns(&row).unwrap();
        assert_eq!(read.rate, 1.0);
        assert!(!read.running);
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
        assert_eq!(timeline.recording, Some(RecordSource::Input(input)));
        timeline.record(serde_json::json!({ "inputId": serde_json::Value::Null })).unwrap();
        assert_eq!(timeline.recording, None);
    }

    #[test]
    fn arming_the_programmer_releases_by_default() {
        let mut timeline = a_timeline();
        timeline.record(serde_json::json!({ "programmer": true })).unwrap();
        assert_eq!(timeline.recording, Some(RecordSource::Programmer { release: true }));
        timeline.record(serde_json::json!({ "programmer": true, "release": false })).unwrap();
        assert_eq!(timeline.recording, Some(RecordSource::Programmer { release: false }));
        timeline.record(serde_json::json!({})).unwrap();
        assert_eq!(timeline.recording, None, "naming nothing disarms");
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
