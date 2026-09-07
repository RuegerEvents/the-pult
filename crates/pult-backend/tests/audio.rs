//! Timecode into a station, and a tempo out of one.
//!
//! Out here rather than beside the manager because both halves need a whole station:
//! an anchor is written through the engine and replicated, an event is fired by the
//! engine's own pass, and a speed master is a row the leader writes. Neither can be
//! reached from a unit test of an actor.
//!
//! **No test opens an audio device.** CI has none, and a test that needed one would be
//! a test nobody runs. The seam is `AudioHandle::feed_timecode`, which pushes f32
//! samples into exactly the queue a `cpal` input thread pushes them into — so what is
//! exercised here is the real decoder, the real chase, the real anchor write and the
//! real firing, with a generated stream standing in for a cable.
//!
//! Assertions are **positions and counts, never milliseconds of wall time**, for the
//! reason `tests/counts.rs` gives: a timing threshold on a shared runner flaps, a
//! flapping gate gets disabled, and a disabled gate is worse than none.

use std::time::Duration;

use pult_audio::LtcRate as CodecRate;
use pult_backend::{Config, Running};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        cue::Cue,
        sequence::Sequence,
        speedmaster::SpeedMaster,
        timeline::{
            GridSegment, LtcRate, Timeline, TimelineAction, TimelineEvent, TimelineSource,
        },
    },
};

async fn a_station() -> Running {
    let show = std::env::temp_dir().join(format!("pult-audio-{}.pult", uuid::Uuid::new_v4()));
    pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        show: Some(show.clone()),
        identity: Some(show.with_extension("node")),
        ..Config::default()
    })
    .await
    .expect("a station starts")
}

async fn create(station: &Running, table: &str, value: serde_json::Value) {
    station
        .engine
        .set(
            vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            value,
        )
        .await
        .unwrap_or_else(|e| panic!("{table} is created: {e}"));
}

async fn read<T: serde::de::DeserializeOwned>(station: &Running, table: &str) -> Vec<T> {
    let value =
        station.engine.get(vec![PathSegment::Key(table.into())]).await.expect("the collection");
    serde_json::from_value(value).unwrap_or_default()
}

async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{what} never happened");
}

fn a_timeline(source: TimelineSource, events: Vec<TimelineEvent>) -> Timeline {
    Timeline {
        id: uuid::Uuid::new_v4(),
        name: "Act 1".into(),
        audio: None,
        peaks: None,
        detected: None,
        source,
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

/// A sequence with two cues, so a Go has somewhere to go.
async fn a_sequence(station: &Running) -> uuid::Uuid {
    let a_cue = |number: f64, name: &str| Cue {
        id: uuid::Uuid::new_v4(),
        name: name.into(),
        number,
        fade_in_ms: 0,
        fade_out_ms: 0,
        follow_mode: pult_schema::types::cue::FollowMode::Manual,
        captures: Vec::new(),
        easing: None,
        is_active: false,
    };
    let first = a_cue(1.0, "Cue 1");
    let second = a_cue(2.0, "Cue 2");
    create(station, "cues", serde_json::to_value(&first).unwrap()).await;
    create(station, "cues", serde_json::to_value(&second).unwrap()).await;
    let sequence = Sequence {
        id: uuid::Uuid::new_v4(),
        name: "Act".into(),
        cue_ids: vec![first.id, second.id],
        active_cue_index: None,
        went_at: None,
    };
    create(station, "sequences", serde_json::to_value(&sequence).unwrap()).await;
    sequence.id
}

/// Timecode arriving anchors the show and fires what it crosses.
///
/// The whole chase in one test. A generated LTC stream at `00:01:00:00` is pushed into
/// the manager; the decoder locks; the anchor is rewritten **from the timecode's
/// position, not from anything this station worked out**; and the event written at 60 s
/// is crossed and Goes the cue.
///
/// What is asserted is the *position*, and that is the point: an anchor a second out
/// would fire the Go a second late, and on a musical cue a second is a bar.
#[tokio::test]
async fn a_station_chases_timecode_and_fires_what_it_crosses() {
    let station = a_station().await;
    let sequence_id = a_sequence(&station).await;

    // A minute in, at 25 fps, with no offset. Sixty seconds is far enough from zero
    // that a station that ignored the timecode entirely could not accidentally pass.
    let rate = LtcRate::F25;
    let mut timeline = a_timeline(
        TimelineSource::Ltc { fps: rate, offset_frames: 0 },
        vec![TimelineEvent {
            id: uuid::Uuid::new_v4(),
            at_ms: 60_000,
            action: TimelineAction::GoNext { sequence_id },
        }],
    );
    timeline.running = true;
    timeline.anchor_ms = pult_schema::clock::now_ms();
    timeline.position_at_anchor_ms = 0;
    let timeline_id = timeline.id;
    create(&station, "timelines", serde_json::to_value(&timeline).unwrap()).await;

    let audio = station.audio.clone().expect("a station has an audio manager");
    // The manager has to have seen the timeline — and opened its (absent) input device
    // — before there is a decoder to feed. It is told on the engine's own pass, so this
    // waits for the row rather than sleeping a constant.
    eventually("the timeline reaches the audio manager", || async {
        !read::<Timeline>(&station, "timelines").await.is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Two seconds of generated code from 00:01:00:00, pushed the way a sound card
    // would push it. Fed twice, because the very first frame of a stream is partial:
    // a decoder that has just been switched on starts mid-bit.
    for _ in 0..2 {
        let samples = pult_audio::ltc::encode(0, 1, 0, 0, CodecRate::F25, 25, 48_000);
        audio.feed_timecode(timeline_id, samples, 48_000).await;
    }

    eventually("the anchor follows the timecode", || async {
        let timelines: Vec<Timeline> = read(&station, "timelines").await;
        let Some(timeline) = timelines.first() else { return false };
        let position = timeline.position_at(pult_schema::clock::now_ms());
        // A second of slack: the code is a second long and the pass that reads it can
        // fall anywhere in it. What is being asserted is that the playhead went to
        // *a minute* rather than staying near zero.
        position >= 59_000 && position < 63_000
    })
    .await;

    eventually("the event at a minute fires", || async {
        let sequences: Vec<Sequence> = read(&station, "sequences").await;
        sequences.first().is_some_and(|s| s.active_cue_index.is_some())
    })
    .await;

    // And the station says what it is doing, which is the row the panel reads. Waited
    // for rather than read at once: the status is published on its own one-second
    // tick, so reading it the instant the anchor moved is reading it before it exists.
    let statuses = || async {
        let value = station
            .engine
            .get(vec![PathSegment::Key("audio_status".into())])
            .await
            .expect("audio_status is a LOCAL path");
        serde_json::from_value::<pult_schema::types::timeline::AudioStatuses>(value)
            .unwrap_or_default()
    };
    eventually("the station publishes what its timecode is doing", || async {
        statuses().await.contains_key(&timeline_id.to_string())
    })
    .await;
    let found = statuses().await;
    let row = found.get(&timeline_id.to_string()).expect("a row for the chasing timeline");
    let ltc = row.ltc.as_ref().expect("an LTC status");
    // The *last* frame decoded, which is somewhere inside the second that was fed —
    // a stream is a second long and what is reported is where it got to, not where it
    // began. Asserted as a range for that reason and not as a tolerance on a clock:
    // nothing here depends on how fast the machine running it is.
    let at = ltc.position_ms.expect("a decoded position");
    assert!((60_000..61_000).contains(&at), "the decoded position was {at}");
    assert_eq!(ltc.drop_frame, Some(false), "25 fps does not drop frames");

    station.shutdown("the test is over").await;
}

/// An offset is subtracted, which is what makes a reel that starts at an hour a show
/// that starts at zero.
#[tokio::test]
async fn a_timecode_offset_moves_the_show_to_zero() {
    let station = a_station().await;

    // An hour of 25 fps frames is 90 000 of them.
    let mut timeline = a_timeline(
        TimelineSource::Ltc { fps: LtcRate::F25, offset_frames: 90_000 },
        Vec::new(),
    );
    timeline.running = true;
    timeline.anchor_ms = pult_schema::clock::now_ms();
    let timeline_id = timeline.id;
    create(&station, "timelines", serde_json::to_value(&timeline).unwrap()).await;

    let audio = station.audio.clone().expect("an audio manager");
    eventually("the timeline reaches the audio manager", || async {
        !read::<Timeline>(&station, "timelines").await.is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    for _ in 0..2 {
        // 01:00:10:00 — ten seconds past the offset.
        let samples = pult_audio::ltc::encode(1, 0, 10, 0, CodecRate::F25, 25, 48_000);
        audio.feed_timecode(timeline_id, samples, 48_000).await;
    }

    eventually("the offset lands the show ten seconds in", || async {
        let statuses: pult_schema::types::timeline::AudioStatuses = serde_json::from_value(
            station
                .engine
                .get(vec![PathSegment::Key("audio_status".into())])
                .await
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        statuses
            .get(&timeline_id.to_string())
            .and_then(|row| row.ltc.as_ref())
            .and_then(|ltc| ltc.position_ms)
            .is_some_and(|at| (10_000..11_000).contains(&at))
    })
    .await;

    station.shutdown("the test is over").await;
}

/// A timeline drives a speed master's tempo, and steps it at the segment boundary.
///
/// The grid says 120 for the first ten seconds and 90 after them; the timeline is
/// played from just before the boundary, and the master's `bpm` is asserted on both
/// sides of it. `t0` moving is what makes the change a bounded step in phase rather
/// than a drift — the new rate and the anchor it is measured from arrive together.
#[tokio::test]
async fn a_running_timeline_steps_its_speed_master() {
    let station = a_station().await;

    let master = SpeedMaster {
        id: uuid::Uuid::new_v4(),
        name: "Chases".into(),
        bpm: 60.0,
        multiplier: 1.0,
        running: true,
        t0: 0,
    };
    create(&station, "speed_masters", serde_json::to_value(&master).unwrap()).await;

    let mut timeline = a_timeline(TimelineSource::Internal, Vec::new());
    timeline.speed_master = Some(master.id);
    timeline.grid = vec![
        GridSegment { at_ms: 0, bpm: 120.0, beats_per_bar: 4 },
        GridSegment { at_ms: 10_000, bpm: 90.0, beats_per_bar: 4 },
    ];
    // Played from 9.5 s, so the boundary is half a second away and the test does not
    // sit through ten seconds of song.
    timeline.running = true;
    timeline.anchor_ms = pult_schema::clock::now_ms();
    timeline.position_at_anchor_ms = 9_500;
    create(&station, "timelines", serde_json::to_value(&timeline).unwrap()).await;

    let bpm_now = || async {
        read::<SpeedMaster>(&station, "speed_masters").await.first().map(|m| m.bpm)
    };

    eventually("the first segment's tempo is written", || async {
        bpm_now().await == Some(120.0)
    })
    .await;
    let before = read::<SpeedMaster>(&station, "speed_masters").await[0].t0;

    eventually("the second segment steps it", || async { bpm_now().await == Some(90.0) }).await;
    let after = read::<SpeedMaster>(&station, "speed_masters").await[0].t0;
    assert!(after > before, "the anchor moves with the tempo, or the beat drifts");

    // And it stays there when the timeline stops: an operator chasing a song wants the
    // chases at its tempo through the applause.
    station
        .engine
        .set(
            vec![
                PathSegment::Key("timelines".into()),
                PathSegment::Id(timeline.id),
                PathSegment::Key("stop".into()),
            ],
            Lifecycle::Synced,
            serde_json::json!({}),
        )
        .await
        .expect("the timeline stops");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(bpm_now().await, Some(90.0), "stopping keeps the last tempo");

    station.shutdown("the test is over").await;
}

/// A timeline with no audio and no timecode runs exactly as it did before any of this
/// existed. The property that must not be lost: audio never gates the transport.
#[tokio::test]
async fn a_timeline_with_no_sound_runs_the_way_it_always_did() {
    let station = a_station().await;
    let sequence_id = a_sequence(&station).await;

    let mut timeline = a_timeline(
        TimelineSource::Internal,
        vec![TimelineEvent {
            id: uuid::Uuid::new_v4(),
            at_ms: 200,
            action: TimelineAction::GoNext { sequence_id },
        }],
    );
    timeline.running = true;
    timeline.anchor_ms = pult_schema::clock::now_ms();
    create(&station, "timelines", serde_json::to_value(&timeline).unwrap()).await;

    eventually("the event fires with nothing playing", || async {
        let sequences: Vec<Sequence> = read(&station, "sequences").await;
        sequences.first().is_some_and(|s| s.active_cue_index.is_some())
    })
    .await;

    // And nothing was said about audio at all, because there is none to say anything
    // about — an empty row would be a panel full of "no".
    let statuses: pult_schema::types::timeline::AudioStatuses = serde_json::from_value(
        station
            .engine
            .get(vec![PathSegment::Key("audio_status".into())])
            .await
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    assert!(statuses.is_empty(), "a silent timeline reports nothing: {statuses:?}");

    station.shutdown("the test is over").await;
}
