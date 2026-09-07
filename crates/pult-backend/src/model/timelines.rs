//! Firing what a timeline has written against a position.
//!
//! A pure state machine like [`super::playback::Playback`], and for the same reason:
//! what it returns is a list of things to do, and a test can drive a whole song
//! through it in microseconds without a clock or a socket anywhere near it.
//!
//! **Only the leader fires.** The engine checks that; this object does not know what a
//! session is. What it does know is that an event becomes an ordinary Go — the same
//! `run_synced_command` a follow cue goes through — carrying `at` set to the wall
//! millisecond the *event* fell at rather than the moment this pass happened to run.
//! Which is what makes every station anchor the cue on the same millisecond, however
//! late the leader's pass was.
//!
//! **A timeline can drive a speed master, and the step is bounded.** Where one names a
//! master, crossing a grid segment writes that master's `bpm` and its `t0` — the wall
//! millisecond that segment's downbeat fell at, worked out backwards from the anchor.
//! One write per segment and none in between, which is the discipline
//! `types::speedmaster` already lives by: a tempo change is a bounded step in phase
//! rather than a drift, because the new rate and the anchor it is measured from arrive
//! together. A locate re-states it, since the playhead has moved to a different bar;
//! **stopping does not**, so the chases go on running at the song's tempo through the
//! applause.
//!
//! **A locate takes the tracked state and fires nothing crossed backwards.** Jumping
//! into the second chorus should leave each sequence on the cue it would have been on,
//! which is [`Timeline::tracked_at`] — the latest event per sequence at or before the
//! new position — and not a burst of forty Gos in one millisecond. The same rule
//! applies to a fresh play from a position, because they are the same act as far as
//! the cues are concerned.

use std::collections::HashMap;

use pult_schema::types::timeline::Timeline;
use uuid::Uuid;

/// One thing a timeline wants done.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineFire {
    pub sequence_id: Uuid,
    /// The registered command's name, as the engine's command table spells it.
    pub command: &'static str,
    pub args: serde_json::Value,
    /// The wall millisecond the event fell at, which is what a Go anchors on.
    pub at: u64,
}

/// A recording that has just stopped, and where its playhead was when it did.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineStopped {
    pub timeline_id: Uuid,
    pub position_ms: u64,
    /// The assets that were playing, so the caller can sample each one where it
    /// stopped and let its keys go.
    pub tracks: Vec<String>,
}

/// A speed master a timeline wants set to a tempo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineTempo {
    pub master_id: Uuid,
    pub bpm: f32,
    /// The console millisecond the segment's downbeat fell at. Worked out backwards
    /// from the anchor rather than taken as "now", so a pass that ran late still puts
    /// the "one" where it belonged and the beat does not walk.
    pub t0: u64,
}

/// What one pass came to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimelinePass {
    pub fire: Vec<TimelineFire>,
    pub stopped: Vec<TimelineStopped>,
    pub masters: Vec<TimelineTempo>,
}

/// What each timeline was doing at the end of the last pass.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Seen {
    running: bool,
    /// The position events have been fired up to, inclusive. `None` for a timeline
    /// nothing has been fired for yet, which is what makes an event at position zero
    /// fire when the timeline is played from zero.
    fired_to: Option<u64>,
    /// Where the playhead was, and the console millisecond that pass ran at.
    ///
    /// The pair is what tells a jump from ordinary progress, and it does it **exactly
    /// rather than by a tolerance**: the new transport is asked where the playhead was
    /// at the *previous* pass's moment, and if that is not where the previous pass saw
    /// it then somebody moved it. Ordinary progress leaves the transport alone and the
    /// two agree to the millisecond; a locate, or a play from a position, rewrites the
    /// anchor and they do not. Guessing "how far should it have got by now" would have
    /// needed a tolerance, and a tolerance on a console that was busy is a Go that
    /// silently did not happen.
    position_ms: u64,
    at_ms: u64,
    /// Which grid segment the playhead was in, so a tempo is written when one is
    /// crossed and not forty times a second while it is not.
    segment: Option<usize>,
}

#[derive(Default)]
pub struct Timelines {
    seen: HashMap<Uuid, Seen>,
}

impl Timelines {
    /// One pass, at a console millisecond.
    ///
    /// `timelines` is the collection as it stands. A timeline that has gone is
    /// forgotten; one that is running has whatever it crossed since the last pass
    /// fired; one that has jumped — located, or played from somewhere — takes its
    /// tracked state instead.
    pub fn pass(&mut self, wall_ms: u64, timelines: &[Timeline]) -> TimelinePass {
        let mut out = TimelinePass::default();
        self.seen.retain(|id, _| timelines.iter().any(|timeline| timeline.id == *id));

        for timeline in timelines {
            let position = timeline.position_at(wall_ms);
            let before = self.seen.get(&timeline.id).copied();
            let was_running = before.is_some_and(|seen| seen.running);
            // The segment the playhead is in, whatever the timeline is doing: kept in
            // `Seen` even while stopped, so pressing play in the middle of a song does
            // not restate a tempo that has not changed.
            let segment = segment_at(timeline, position);

            if !timeline.running {
                if was_running {
                    out.stopped.push(TimelineStopped {
                        timeline_id: timeline.id,
                        position_ms: position,
                        tracks: timeline
                            .tracks
                            .iter()
                            .filter(|track| track.enabled)
                            .map(|track| track.asset.clone())
                            .collect(),
                    });
                }
                // A stopped timeline keeps its place in the record rather than being
                // forgotten: pressing play again must not re-fire everything before
                // the playhead.
                self.seen.insert(
                    timeline.id,
                    Seen {
                        running: false,
                        fired_to: before.and_then(|s| s.fired_to),
                        position_ms: position,
                        at_ms: wall_ms,
                        segment,
                    },
                );
                continue;
            }

            // Somebody moved the playhead — located it, or played from a position —
            // as against it having simply run on. See [`Seen::position_ms`].
            let jumped = match before {
                None => true,
                Some(seen) => timeline.position_at(seen.at_ms) != seen.position_ms,
            };

            if jumped {
                for event in timeline.tracked_at(position) {
                    out.fire.push(fire_at(event, wall_ms));
                }
                // Always restated after a jump, even into the same segment: the
                // playhead is somewhere else in the bar, so the master's `t0` is now
                // wrong by however far it moved.
                if let Some(tempo) = tempo_of(timeline, segment, position, wall_ms) {
                    out.masters.push(tempo);
                }
                self.seen.insert(
                    timeline.id,
                    Seen {
                        running: true,
                        fired_to: Some(position),
                        position_ms: position,
                        at_ms: wall_ms,
                        segment,
                    },
                );
                continue;
            }

            if segment != before.and_then(|seen| seen.segment) {
                if let Some(tempo) = tempo_of(timeline, segment, position, wall_ms) {
                    out.masters.push(tempo);
                }
            }

            let fired_to = before.and_then(|seen| seen.fired_to);
            for event in timeline.crossed(fired_to, position) {
                // The wall millisecond the *event* fell at, worked out backwards from
                // the playhead — so a pass that ran 40 ms late still anchors the cue
                // where it belonged. A rate of zero, and a position before the anchor,
                // both fall back to now rather than dividing by nothing.
                let late = position.saturating_sub(event.at_ms as u64);
                let rate = timeline.rate.max(0.0);
                let at = if rate > 0.0 {
                    wall_ms.saturating_sub((late as f64 / rate as f64) as u64)
                } else {
                    wall_ms
                };
                out.fire.push(fire_at(event, at));
            }
            self.seen.insert(
                timeline.id,
                Seen {
                    running: true,
                    fired_to: Some(position),
                    position_ms: position,
                    at_ms: wall_ms,
                    segment,
                },
            );
        }
        out
    }

    /// The next console millisecond a pass would do something at.
    ///
    /// A deadline the way a follow cue's is, so a station with a song running sleeps
    /// until the next Go rather than polling. A rate of zero never arrives anywhere,
    /// and says so.
    pub fn next_deadline(&self, wall_ms: u64, timelines: &[Timeline]) -> Option<u64> {
        timelines
            .iter()
            .filter(|timeline| timeline.running && timeline.rate > 0.0)
            .filter_map(|timeline| {
                let position = timeline.position_at(wall_ms);
                let next = timeline.next_event_after(position)?;
                let ahead = (next.at_ms as u64).saturating_sub(position);
                Some(wall_ms + (ahead as f64 / timeline.rate as f64) as u64)
            })
            .min()
    }
}

/// Which grid segment a position falls in: the last one that has started.
///
/// `None` before the first segment, and for a timeline with no grid at all — which is
/// most of them, and is why nothing here has to be told whether a song has been
/// analysed.
fn segment_at(timeline: &Timeline, position_ms: u64) -> Option<usize> {
    timeline.grid.iter().rposition(|segment| segment.at_ms as u64 <= position_ms)
}

/// The tempo a timeline wants its master at, and where that segment's downbeat fell.
fn tempo_of(
    timeline: &Timeline,
    segment: Option<usize>,
    position_ms: u64,
    wall_ms: u64,
) -> Option<TimelineTempo> {
    let master_id = timeline.speed_master?;
    let segment = timeline.grid.get(segment?)?;
    // The wall millisecond the playhead was at this segment's start. A rate of zero
    // never got anywhere, and falls back to now rather than dividing by nothing — the
    // same guard `fire_at`'s caller applies.
    let ahead = position_ms.saturating_sub(segment.at_ms as u64);
    let rate = timeline.rate.max(0.0);
    let t0 = if rate > 0.0 {
        wall_ms.saturating_sub((ahead as f64 / rate as f64) as u64)
    } else {
        wall_ms
    };
    Some(TimelineTempo { master_id, bpm: segment.bpm, t0 })
}

fn fire_at(event: &pult_schema::types::timeline::TimelineEvent, at: u64) -> TimelineFire {
    let (command, mut args) = event.action.command();
    if let Some(object) = args.as_object_mut() {
        object.insert("at".into(), serde_json::json!(at));
    }
    TimelineFire { sequence_id: event.action.sequence_id(), command, args, at }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pult_schema::types::timeline::{TimelineAction, TimelineEvent, TimelineSource};

    fn a_timeline(events: Vec<TimelineEvent>) -> Timeline {
        Timeline {
            id: Uuid::from_u128(1),
            name: "Song".into(),
            audio: None,
            peaks: None,
            detected: None,
            source: TimelineSource::Internal,
            grid: Vec::new(),
            markers: Vec::new(),
            events,
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

    fn go_next(at_ms: u32, sequence: u128) -> TimelineEvent {
        TimelineEvent {
            id: Uuid::from_u128(at_ms as u128 + 900),
            at_ms,
            action: TimelineAction::GoNext { sequence_id: Uuid::from_u128(sequence) },
        }
    }

    fn playing(mut timeline: Timeline, anchor: u64, from: u64) -> Timeline {
        timeline.running = true;
        timeline.anchor_ms = anchor;
        timeline.position_at_anchor_ms = from;
        timeline
    }

    /// The whole of what a timeline does, and the one thing it must never do twice.
    #[test]
    fn an_event_fires_once_when_the_playhead_crosses_it() {
        let mut timelines = Timelines::default();
        let timeline = playing(a_timeline(vec![go_next(1_000, 7)]), 10_000, 0);

        assert!(timelines.pass(10_500, std::slice::from_ref(&timeline)).fire.is_empty());

        let fired = timelines.pass(11_200, std::slice::from_ref(&timeline)).fire;
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].command, "goNext");
        assert_eq!(fired[0].sequence_id, Uuid::from_u128(7));
        assert_eq!(fired[0].at, 11_000, "anchored where the event fell, not where the pass ran");

        assert!(timelines.pass(20_000, &[timeline]).fire.is_empty(), "and not again");
    }

    /// A jump takes the tracked state: one Go per sequence, and nothing that was
    /// jumped over backwards.
    #[test]
    fn locating_takes_the_tracked_state_rather_than_firing_everything() {
        let mut timelines = Timelines::default();
        let events = vec![go_next(0, 1), go_next(1_000, 1), go_next(1_500, 2), go_next(9_000, 1)];
        let timeline = playing(a_timeline(events), 10_000, 0);
        timelines.pass(10_000, std::slice::from_ref(&timeline));

        // Located to 5 s while running.
        let mut located = timeline.clone();
        located.anchor_ms = 12_000;
        located.position_at_anchor_ms = 5_000;
        let fired = timelines.pass(12_000, std::slice::from_ref(&located)).fire;
        assert_eq!(fired.len(), 2, "one per sequence: the latest event each has passed");

        // And nothing fires again as it carries on from there.
        assert!(timelines.pass(12_500, &[located]).fire.is_empty());
    }

    /// Pressing play must not re-fire everything before the playhead.
    #[test]
    fn stopping_and_starting_again_does_not_repeat_what_was_already_fired() {
        let mut timelines = Timelines::default();
        let timeline = playing(a_timeline(vec![go_next(500, 1)]), 0, 0);
        assert_eq!(timelines.pass(1_000, std::slice::from_ref(&timeline)).fire.len(), 1);

        let mut stopped = timeline.clone();
        stopped.running = false;
        stopped.position_at_anchor_ms = 1_000;
        stopped.anchor_ms = 1_000;
        timelines.pass(1_000, std::slice::from_ref(&stopped));

        let mut again = stopped.clone();
        again.running = true;
        again.anchor_ms = 5_000;
        let fired = timelines.pass(5_100, &[again]).fire;
        assert!(fired.is_empty(), "the playhead has not gone back over it");
    }

    #[test]
    fn a_timeline_that_stops_says_which_recordings_were_playing() {
        let mut timelines = Timelines::default();
        let mut timeline = playing(a_timeline(vec![]), 0, 0);
        timeline.tracks = vec![
            pult_schema::types::timeline::TimelineTrack {
                id: Uuid::from_u128(4),
                name: "Take 1".into(),
                asset: "abc".into(),
                input_id: None,
                offset_ms: 0,
                enabled: true,
            },
            pult_schema::types::timeline::TimelineTrack {
                id: Uuid::from_u128(5),
                name: "Take 2".into(),
                asset: "def".into(),
                input_id: None,
                offset_ms: 0,
                enabled: false,
            },
        ];
        timelines.pass(1_000, std::slice::from_ref(&timeline));

        let mut stopped = timeline.clone();
        stopped.running = false;
        stopped.position_at_anchor_ms = 2_000;
        let pass = timelines.pass(3_000, &[stopped]);

        assert_eq!(pass.stopped.len(), 1);
        assert_eq!(pass.stopped[0].position_ms, 2_000);
        assert_eq!(pass.stopped[0].tracks, vec!["abc".to_string()], "only the enabled one");
    }

    fn with_grid(mut timeline: Timeline, master: Uuid) -> Timeline {
        use pult_schema::types::timeline::GridSegment;
        timeline.speed_master = Some(master);
        timeline.grid = vec![
            GridSegment { at_ms: 0, bpm: 120.0, beats_per_bar: 4 },
            GridSegment { at_ms: 10_000, bpm: 90.0, beats_per_bar: 4 },
        ];
        timeline
    }

    /// A tempo is written when a segment is crossed and **not** in between, which is
    /// the whole of what "a bounded step" means: forty writes a second to a SYNCED
    /// field would be a tempo that drifts as fast as the network is slow.
    #[test]
    fn a_timeline_steps_its_speed_master_at_each_grid_segment() {
        let master = Uuid::from_u128(42);
        let mut timelines = Timelines::default();
        let timeline = with_grid(playing(a_timeline(vec![]), 10_000, 0), master);

        // The first pass is a jump — nothing has been seen before — and states the
        // segment the playhead is in.
        let first = timelines.pass(10_000, std::slice::from_ref(&timeline)).masters;
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].master_id, master);
        assert_eq!(first[0].bpm, 120.0);
        assert_eq!(first[0].t0, 10_000, "the segment at zero began when the timeline did");

        // Running on inside the same segment writes nothing at all.
        assert!(timelines.pass(12_000, std::slice::from_ref(&timeline)).masters.is_empty());
        assert!(timelines.pass(15_000, std::slice::from_ref(&timeline)).masters.is_empty());

        // And the second segment is one write, anchored where its downbeat fell —
        // which is 10 s after the timeline started, not whenever this pass ran.
        let crossed = timelines.pass(20_500, std::slice::from_ref(&timeline)).masters;
        assert_eq!(crossed.len(), 1);
        assert_eq!(crossed[0].bpm, 90.0);
        assert_eq!(crossed[0].t0, 20_000, "the downbeat, not the pass");

        assert!(timelines.pass(22_000, &[timeline]).masters.is_empty());
    }

    /// A locate restates the tempo even into the same segment: the playhead is
    /// somewhere else in the bar, so the master's anchor is now wrong by however far
    /// it moved.
    #[test]
    fn locating_restates_the_tempo_because_the_bar_has_moved() {
        let master = Uuid::from_u128(42);
        let mut timelines = Timelines::default();
        let timeline = with_grid(playing(a_timeline(vec![]), 10_000, 0), master);
        timelines.pass(10_000, std::slice::from_ref(&timeline));

        let mut located = timeline.clone();
        located.anchor_ms = 12_000;
        located.position_at_anchor_ms = 4_000;
        let after = timelines.pass(12_000, &[located]).masters;
        assert_eq!(after.len(), 1, "the same segment, and still restated");
        assert_eq!(after[0].bpm, 120.0);
        assert_eq!(after[0].t0, 8_000, "four seconds before now, which is where zero fell");
    }

    /// Stopping keeps the last tempo: an operator who has been chasing a song still
    /// wants the chases running at its tempo in the applause.
    #[test]
    fn stopping_writes_no_tempo_at_all() {
        let master = Uuid::from_u128(42);
        let mut timelines = Timelines::default();
        let timeline = with_grid(playing(a_timeline(vec![]), 0, 0), master);
        timelines.pass(1_000, std::slice::from_ref(&timeline));

        let mut stopped = timeline.clone();
        stopped.running = false;
        stopped.position_at_anchor_ms = 2_000;
        assert!(timelines.pass(3_000, &[stopped]).masters.is_empty());
    }

    /// A timeline with a grid and no master, and one with a master and no grid, both
    /// write nothing — which is most timelines, and is why nothing here has to be told
    /// whether a song has been analysed.
    #[test]
    fn a_timeline_with_nothing_to_drive_drives_nothing() {
        let mut timelines = Timelines::default();
        let mut no_master = with_grid(playing(a_timeline(vec![]), 0, 0), Uuid::from_u128(1));
        no_master.speed_master = None;
        assert!(timelines.pass(1_000, &[no_master]).masters.is_empty());

        let mut no_grid = playing(a_timeline(vec![]), 0, 0);
        no_grid.speed_master = Some(Uuid::from_u128(1));
        let mut fresh = Timelines::default();
        assert!(fresh.pass(1_000, &[no_grid.clone()]).masters.is_empty());

        // And a playhead before the first segment has no tempo to be at yet.
        no_grid.grid = vec![pult_schema::types::timeline::GridSegment {
            at_ms: 30_000,
            bpm: 128.0,
            beats_per_bar: 4,
        }];
        let mut before = Timelines::default();
        assert!(before.pass(1_000, &[no_grid]).masters.is_empty());
    }

    #[test]
    fn the_next_event_is_a_deadline_in_wall_time() {
        let timelines = Timelines::default();
        let timeline = playing(a_timeline(vec![go_next(4_000, 1)]), 10_000, 0);
        assert_eq!(timelines.next_deadline(10_000, std::slice::from_ref(&timeline)), Some(14_000));

        // At double speed it arrives in half the wall time.
        let mut fast = timeline.clone();
        fast.rate = 2.0;
        assert_eq!(timelines.next_deadline(10_000, std::slice::from_ref(&fast)), Some(12_000));

        let mut stopped = timeline;
        stopped.running = false;
        assert_eq!(timelines.next_deadline(10_000, &[stopped]), None);
    }
}
