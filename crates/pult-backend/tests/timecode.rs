//! A wire read back into the console, and a position that Goes a cue.
//!
//! Out here rather than beside the connectors because both halves need a whole
//! station: an input is a real socket on a real port that a real output has to send
//! into, and a timeline event is fired by the engine's own pass against the engine's
//! own clock. Neither can be reached from a unit test of an actor.
//!
//! Both assertions are **counts and values, never milliseconds**, for the reason
//! `tests/counts.rs` gives: a timing threshold on a shared runner flaps, a flapping
//! gate gets disabled, and a disabled gate is worse than none. What is waited for is a
//! condition, with a generous ceiling.

use std::time::Duration;

use pult_backend::{Config, Running};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        cue::{Cue, FollowMode, ParameterCapture},
        fixture::{
            Fixture, FixtureAddress, FixtureType, ParameterDefinition, ParameterKind,
            ParameterValue,
        },
        input::{InputConfig, InputKind},
        output::{OutputConfig, OutputKind},
        programmer::ProgrammerValue,
        sequence::Sequence,
        timeline::{Timeline, TimelineAction, TimelineEvent, TimelineSource},
    },
};

async fn a_station() -> Running {
    let show = std::env::temp_dir().join(format!("pult-timecode-{}.pult", uuid::Uuid::new_v4()));
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

/// A dimmer at 1.1, and the type it is patched as. `resting` is where its intensity
/// sits when nothing is driving it, which is what puts a byte on the wire without
/// anything having to be programmed.
async fn a_dimmer(station: &Running, resting: f32) -> uuid::Uuid {
    let fixture_type = FixtureType {
        id: uuid::Uuid::new_v4(),
        name: "Dimmer".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition::new(
            ParameterKind::Intensity,
            ParameterValue::Float(0.0),
        )],
        ..Default::default()
    };
    let fixture = Fixture {
        id: uuid::Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: fixture_type.id,
        address: FixtureAddress::dmx(1, 1),
        home_values: [("Intensity".to_string(), ParameterValue::Float(resting))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    create(station, "fixture_types", serde_json::to_value(&fixture_type).unwrap()).await;
    create(station, "fixtures", serde_json::to_value(&fixture).unwrap()).await;
    fixture.id
}

/// Wait for a condition, or give up loudly. Generous, because what is being asserted
/// is that a thing happens at all.
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

/// One station sending sACN at its own loopback and reading it back in.
///
/// The whole of the input path in one test: an output renders the patch and puts real
/// E1.31 on the wire; an input joins the mapped universe on loopback, parses it,
/// merges it and holds the image; and `input.grab` decodes it **through the patch** and
/// writes a value — not a byte — into the programmer.
///
/// The value is what is asserted, and it is the point. A grab that wrote 191 rather
/// than 0.75 would put a raw slot into a show, and a recording made of that could
/// never be played back onto a rig that had been repatched.
#[tokio::test]
async fn a_universe_this_station_sent_comes_back_in_as_a_programmer_value() {
    let station = a_station().await;
    // 0.74902 is 191/255 exactly, so the round trip through a byte is lossless and the
    // assertion below can be an equality rather than a tolerance — which is what makes
    // it a gate rather than a guess about rounding.
    let expected = ParameterValue::Float(191.0 / 255.0);
    let fixture_id = a_dimmer(&station, 191.0 / 255.0).await;

    create(
        &station,
        "outputs",
        serde_json::to_value(OutputConfig {
            id: uuid::Uuid::new_v4(),
            name: "Loopback".into(),
            kind: OutputKind::Sacn,
            // Unicast at our own listener rather than the multicast group: a test must
            // not depend on this machine's multicast routing, and the packet on the
            // wire is byte for byte the same one.
            target: Some(format!("127.0.0.1:{}", 5568)),
            universes: vec![1],
            enabled: true,
            node_id: Some(station.node_id),
            interfaces: Default::default(),
            priority: Default::default(),
        })
        .unwrap(),
    )
    .await;

    create(
        &station,
        "inputs",
        serde_json::to_value(InputConfig {
            id: uuid::Uuid::new_v4(),
            name: "Guest".into(),
            kind: InputKind::Sacn,
            node_id: Some(station.node_id),
            interfaces: Default::default(),
            universes: [(1u16, 1u16)].into_iter().collect(),
            enabled: true,
        })
        .unwrap(),
    )
    .await;

    // Nothing is in the programmer: the fixture's own home value is what the output
    // is carrying, so the number the grab writes cannot be a number that was already
    // there.
    let entry_id: uuid::Uuid =
        pult_schema::types::programmer::programmer_entry_id(&fixture_id.to_string(), "Intensity")
            .parse()
            .unwrap();
    assert!(read::<ProgrammerValue>(&station, "programmer_values").await.is_empty());

    let input = station.input.clone().expect("the station has an input manager");
    eventually("the input heard the output", || {
        let input = input.clone();
        async move { input.grab(None, vec![fixture_id]).await.is_ok() }
    })
    .await;

    // The station RPC, which is what a browser calls: it decodes and writes under the
    // caller's authorship in one gesture.
    let grabbed = station
        .call_rpc("input.grab", serde_json::json!({ "fixtureIds": [fixture_id] }))
        .await
        .expect("the grab is answered");
    assert_eq!(grabbed["written"], 1, "one parameter was written");

    let held: Vec<ProgrammerValue> = read(&station, "programmer_values").await;
    let entry = held.iter().find(|e| e.id == entry_id).expect("the programmer entry");
    assert_eq!(entry.value, expected, "the wire came back as a value, not as a byte");
}

/// A timeline event Goes a cue when the playhead crosses it.
///
/// The other half, and the one thing a timeline is for. Asserted as *which cue is
/// active*, not as when: the event is written at 100 ms and the position is a function
/// of the anchor, so the only machine-dependent part is how soon the engine's pass
/// notices — which is what `eventually` absorbs.
#[tokio::test]
async fn a_timeline_event_goes_the_cue_it_names() {
    let station = a_station().await;
    let fixture_id = a_dimmer(&station, 0.0).await;

    let cue = Cue {
        id: uuid::Uuid::new_v4(),
        name: "Cue 1".into(),
        number: 1.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
        follow_mode: FollowMode::Manual,
        captures: vec![ParameterCapture {
            fixture_id,
            parameter_kind: ParameterKind::Intensity,
            value: ParameterValue::Float(1.0),
            fade_in_ms: 0,
            fade_out_ms: 0,
            delay_in_ms: 0,
            easing: None,
            effect: None,
            preset: None,
        }],
        easing: None,
        is_active: false,
    };
    let sequence = Sequence {
        id: uuid::Uuid::new_v4(),
        name: "Act 1".into(),
        cue_ids: vec![cue.id],
        active_cue_index: None,
        went_at: None,
    };
    create(&station, "cues", serde_json::to_value(&cue).unwrap()).await;
    create(&station, "sequences", serde_json::to_value(&sequence).unwrap()).await;

    let timeline = Timeline {
        id: uuid::Uuid::new_v4(),
        name: "Song".into(),
        audio: None,
        peaks: None,
        detected: None,
        source: TimelineSource::Internal,
        grid: Vec::new(),
        markers: Vec::new(),
        events: vec![TimelineEvent {
            id: uuid::Uuid::new_v4(),
            at_ms: 100,
            action: TimelineAction::GoToCue { sequence_id: sequence.id, cue_id: cue.id },
        }],
        tracks: Vec::new(),
        speed_master: None,
        node_id: None,
        running: false,
        anchor_ms: 0,
        position_at_anchor_ms: 0,
        rate: 1.0,
        recording: None,
    };
    create(&station, "timelines", serde_json::to_value(&timeline).unwrap()).await;

    // Nothing has fired yet, which is the half of this that a pass firing on sight
    // would break.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before: Vec<Sequence> = read(&station, "sequences").await;
    assert_eq!(before[0].active_cue_index, None, "a stopped timeline Goes nothing");

    // Play, carrying `at` the way the panel's transport does.
    station
        .engine
        .set(
            vec![
                PathSegment::Key("timelines".into()),
                PathSegment::Id(timeline.id),
                PathSegment::Key("play".into()),
            ],
            Lifecycle::Synced,
            serde_json::json!({ "at": pult_schema::types::sequence::now_ms() }),
        )
        .await
        .expect("the timeline plays");

    eventually("the cue went", || async {
        let sequences: Vec<Sequence> = read(&station, "sequences").await;
        sequences.first().is_some_and(|s| s.active_cue_index == Some(0))
    })
    .await;
}
