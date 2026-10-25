//! A take recorded from the programmer, played back onto a wire.
//!
//! The whole of the path in one station: the operator's writes land in the
//! programmer, the leader's engine hands each one to the take at the moment it applied
//! it, the stop stores the take and gives the faders back, and the stored take — read
//! back off the asset store as bytes — drives an Art-Net output on the next play. Then
//! the two overdubs a skeleton is built from: a pass on another key goes beside the
//! first take, and a pass on the same key is spliced into it.
//!
//! **Values and orders, never milliseconds**, for the reason `tests/counts.rs` gives.
//! Where a pass lands is asserted by which values are in a take and in what order.

use std::time::Duration;

use pult_backend::{Config, Running};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        fixture::{
            Fixture, FixtureAddress, FixtureType, ParameterDefinition, ParameterKind,
            ParameterValue,
        },
        output::{OutputConfig, OutputKind},
        programmer::{programmer_entry_id, ProgrammerValue},
        timeline::{Timeline, TimelineSource, TimelineTrack},
    },
};
use uuid::Uuid;

async fn a_station() -> Running {
    let show = std::env::temp_dir().join(format!("pult-takes-{}.pult", Uuid::new_v4()));
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

async fn create(station: &Running, table: &str, lifecycle: Lifecycle, value: serde_json::Value) {
    station
        .engine
        .set(vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())], lifecycle, value)
        .await
        .unwrap_or_else(|e| panic!("{table} is created: {e}"));
}

async fn read<T: serde::de::DeserializeOwned>(station: &Running, table: &str) -> Vec<T> {
    let value = station.engine.get(vec![PathSegment::Key(table.into())]).await.expect("the collection");
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

/// A command on the timeline, carrying `at` the way the panel does.
async fn timeline(station: &Running, id: Uuid, command: &str, mut args: serde_json::Value) {
    args["at"] = serde_json::json!(pult_schema::types::sequence::now_ms());
    station
        .engine
        .set(
            vec![PathSegment::Key("timelines".into()), PathSegment::Id(id), PathSegment::Key(command.into())],
            Lifecycle::Synced,
            args,
        )
        .await
        .unwrap_or_else(|e| panic!("{command}: {e}"));
}

/// Hold a parameter in the programmer, the way a fader does: one row per key, created
/// the first time and its value written after.
async fn hold(station: &Running, fixture_id: Uuid, kind: ParameterKind, key: &str, value: f32) {
    let id: Uuid = programmer_entry_id(&fixture_id.to_string(), key).parse().unwrap();
    let held: Vec<ProgrammerValue> = read(station, "programmer_values").await;
    if held.iter().any(|entry| entry.id == id) {
        station
            .engine
            .set(
                vec![PathSegment::Key("programmer_values".into()), PathSegment::Id(id), PathSegment::Key("value".into())],
                Lifecycle::Synced,
                serde_json::to_value(ParameterValue::Float(value)).unwrap(),
            )
            .await
            .expect("the fader moves");
    } else {
        let entry = ProgrammerValue {
            id,
            fixture_id,
            parameter_kind: kind,
            value: ParameterValue::Float(value),
            effect: None,
            preset: None,
            locked: false,
        };
        create(station, "programmer_values", Lifecycle::Synced, serde_json::to_value(entry).unwrap()).await;
    }
}

/// The rows a timeline plays, with whatever take is waiting merged first: the review
/// is a person's, and here every take is kept the moment it is waiting.
async fn tracks_of(station: &Running, id: Uuid) -> Vec<TimelineTrack> {
    let timelines: Vec<Timeline> = read(station, "timelines").await;
    if timelines.iter().any(|t| t.id == id && t.pending.is_some()) {
        pult_backend::api::rpcs::dispatch("take.merge", serde_json::json!({ "timelineId": id }), &station.rpc)
            .await
            .expect("the take merges");
    }
    let timelines: Vec<Timeline> = read(station, "timelines").await;
    timelines.into_iter().find(|t| t.id == id).map(|t| t.tracks).unwrap_or_default()
}

async fn armed(station: &Running, id: Uuid) -> bool {
    let timelines: Vec<Timeline> = read(station, "timelines").await;
    timelines.iter().any(|t| t.id == id && t.recording.is_some())
}

/// One take, as bytes off the station's own asset route and through the one codec.
async fn decoded(station: &Running, sha: &str) -> pult_render::Track {
    let bytes = reqwest::get(format!("http://{}/assets/{sha}", station.http_addr))
        .await
        .expect("the asset route answers")
        .bytes()
        .await
        .expect("the bytes");
    pult_render::decode(&bytes).expect("a take this build can read")
}

fn values(track: &pult_render::Track, key: &str) -> Vec<f32> {
    let key = track.keys.iter().find(|k| k.key == key).unwrap_or_else(|| panic!("{key} is in the take"));
    key.points
        .iter()
        .map(|p| match p.value {
            ParameterValue::Float(v) => v,
            ref other => panic!("a float, not {other:?}"),
        })
        .collect()
}

/// Exact as bytes, so the wire can be asserted by equality.
fn level(byte: u8) -> f32 {
    byte as f32 / 255.0
}

#[tokio::test]
async fn a_take_from_the_programmer_plays_back_and_overdubs() {
    let station = a_station().await;

    // A skeleton with a jaw and a head: two channels at 1.1 and 1.2.
    let skeleton_type = FixtureType {
        id: Uuid::new_v4(),
        name: "Skeleton".into(),
        manufacturer: "Acme".into(),
        channel_count: 2,
        parameters: vec![
            ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0)),
            ParameterDefinition::new(ParameterKind::Zoom, ParameterValue::Float(0.0)),
        ],
        ..Default::default()
    };
    let skeleton = Fixture {
        id: Uuid::new_v4(),
        name: "Skeleton".into(),
        fixture_type_id: skeleton_type.id,
        address: FixtureAddress::dmx(1, 1),
        ..Default::default()
    };
    create(&station, "fixture_types", Lifecycle::Persisted, serde_json::to_value(&skeleton_type).unwrap()).await;
    create(&station, "fixtures", Lifecycle::Persisted, serde_json::to_value(&skeleton).unwrap()).await;

    // An Art-Net output at a socket this test holds, so what the take puts on a wire
    // is read as the bytes a node would get.
    let wire = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    create(
        &station,
        "outputs",
        Lifecycle::Persisted,
        serde_json::to_value(OutputConfig {
            id: Uuid::new_v4(),
            name: "Skeleton node".into(),
            kind: OutputKind::Artnet,
            target: Some(wire.local_addr().unwrap().to_string()),
            universes: vec![1],
            enabled: true,
            node_id: Some(station.node_id),
            interfaces: Default::default(),
            priority: Default::default(),
        })
        .unwrap(),
    )
    .await;

    let song = Timeline {
        id: Uuid::new_v4(),
        name: "Scare".into(),
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
    };
    create(&station, "timelines", Lifecycle::Persisted, serde_json::to_value(&song).unwrap()).await;

    // ── The jaw ───────────────────────────────────────────────────────────────
    timeline(&station, song.id, "record", serde_json::json!({ "programmer": true })).await;
    timeline(&station, song.id, "play", serde_json::json!({ "fromMs": 0 })).await;
    for byte in [51u8, 153, 255] {
        tokio::time::sleep(Duration::from_millis(250)).await;
        hold(&station, skeleton.id, ParameterKind::Intensity, "Intensity", level(byte)).await;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    timeline(&station, song.id, "stop", serde_json::json!({})).await;

    eventually("the take was stored and the timeline disarmed", || async {
        !armed(&station, song.id).await && tracks_of(&station, song.id).await.len() == 1
    })
    .await;
    let take = tracks_of(&station, song.id).await.remove(0);
    assert_eq!(take.input_id, None, "a take from the programmer names no input");
    let jaw = decoded(&station, &take.asset).await;
    assert_eq!(jaw.keys.len(), 1, "only the key that moved");
    assert_eq!(values(&jaw, "Intensity"), vec![level(51), level(153), level(255)]);
    let positions: Vec<u32> = jaw.keys[0].points.iter().map(|p| p.ms).collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "change points ascend: {positions:?}");
    eventually("the fader was given back to the take", || async {
        read::<ProgrammerValue>(&station, "programmer_values").await.is_empty()
    })
    .await;

    // ── Played back ───────────────────────────────────────────────────────────
    // From just after the second change point, where the take says 153 and nothing
    // else is driving the jaw: the programmer is empty and the home value is zero.
    timeline(&station, song.id, "play", serde_json::json!({ "fromMs": positions[1] + 50 })).await;
    let mut buf = [0u8; 1024];
    let mut heard = None;
    for _ in 0..200 {
        let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(100), wire.recv(&mut buf)).await else {
            continue;
        };
        // ArtDmx: "Art-Net\0", opcode 0x5000, and the slots from byte 18.
        if n > 18 && &buf[..8] == b"Art-Net\0" && buf[8..10] == [0x00, 0x50] && buf[18] == 153 {
            heard = Some(buf[18]);
            break;
        }
    }
    timeline(&station, song.id, "stop", serde_json::json!({})).await;
    assert_eq!(heard, Some(153), "the take drove the wire with the value it recorded");

    // ── The head, on a second pass ───────────────────────────────────────────
    timeline(&station, song.id, "record", serde_json::json!({ "programmer": true })).await;
    timeline(&station, song.id, "play", serde_json::json!({ "fromMs": 0 })).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    hold(&station, skeleton.id, ParameterKind::Zoom, "Zoom", level(102)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    timeline(&station, song.id, "stop", serde_json::json!({})).await;
    eventually("the head's pass was stored", || async {
        !armed(&station, song.id).await && tracks_of(&station, song.id).await.len() == 2
    })
    .await;
    let after_head = tracks_of(&station, song.id).await;
    assert_eq!(after_head[0].asset, take.asset, "the jaw's take was not rewritten");
    let head = decoded(&station, &after_head[1].asset).await;
    assert_eq!(head.keys.len(), 1);
    assert_eq!(values(&head, "Zoom"), vec![level(102)]);

    // ── A better end to the jaw ──────────────────────────────────────────────
    // Well after the first take's last point, so the splice keeps all of it, adds the
    // new move, and resumes the old take's value where the pass stopped.
    timeline(&station, song.id, "record", serde_json::json!({ "programmer": true })).await;
    timeline(&station, song.id, "play", serde_json::json!({ "fromMs": 10_000 })).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    hold(&station, skeleton.id, ParameterKind::Intensity, "Intensity", level(204)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    timeline(&station, song.id, "stop", serde_json::json!({})).await;
    eventually("the jaw's pass was spliced in", || async {
        let tracks = tracks_of(&station, song.id).await;
        !armed(&station, song.id).await && tracks.len() == 2 && tracks[0].asset != take.asset
    })
    .await;
    let after_splice = tracks_of(&station, song.id).await;
    assert_eq!(after_splice[1].asset, after_head[1].asset, "the head's take was not touched");
    let spliced = decoded(&station, &after_splice[0].asset).await;
    assert_eq!(
        values(&spliced, "Intensity"),
        vec![level(51), level(153), level(255), level(204), level(255)],
        "the old take, the new move, and the old take resuming"
    );
}

/// A scare looping round while the operator gets the jaw right: every lap that was
/// touched is a pass, each later lap is spliced into the take the earlier one made, and
/// the fader is given back only when the timeline finally stops.
#[tokio::test]
async fn every_lap_of_a_looping_rehearsal_is_a_pass() {
    let station = a_station().await;
    let dimmer_type = FixtureType {
        id: Uuid::new_v4(),
        name: "Jaw".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0))],
        ..Default::default()
    };
    let jaw = Fixture {
        id: Uuid::new_v4(),
        name: "Jaw".into(),
        fixture_type_id: dimmer_type.id,
        address: FixtureAddress::dmx(1, 1),
        ..Default::default()
    };
    create(&station, "fixture_types", Lifecycle::Persisted, serde_json::to_value(&dimmer_type).unwrap()).await;
    create(&station, "fixtures", Lifecycle::Persisted, serde_json::to_value(&jaw).unwrap()).await;

    let scare = Timeline {
        id: Uuid::new_v4(),
        name: "Scare".into(),
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
        end: Some(pult_schema::types::timeline::TimelineEnd {
            at_ms: 800,
            then: pult_schema::types::timeline::AtEnd::Loop,
        }),
        running: false,
        anchor_ms: 0,
        position_at_anchor_ms: 0,
        rate: 1.0,
        recording: None,
        armed_by: None,
    };
    create(&station, "timelines", Lifecycle::Persisted, serde_json::to_value(&scare).unwrap()).await;

    timeline(&station, scare.id, "record", serde_json::json!({ "programmer": true })).await;
    timeline(&station, scare.id, "play", serde_json::json!({ "fromMs": 0 })).await;
    // Lap one: a move half way round.
    tokio::time::sleep(Duration::from_millis(500)).await;
    hold(&station, jaw.id, ParameterKind::Intensity, "Intensity", level(51)).await;
    // Lap two, after the wrap at 800: an earlier move.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let held: Vec<ProgrammerValue> = read(&station, "programmer_values").await;
    assert_eq!(held.len(), 1, "the wrap did not take the fader out of the operator's hand");
    hold(&station, jaw.id, ParameterKind::Intensity, "Intensity", level(153)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    timeline(&station, scare.id, "stop", serde_json::json!({})).await;

    eventually("both laps were stored", || async { !armed(&station, scare.id).await }).await;
    let tracks = tracks_of(&station, scare.id).await;
    assert_eq!(tracks.len(), 1, "the second lap went into the first lap's take");
    let take = decoded(&station, &tracks[0].asset).await;
    assert_eq!(
        values(&take, "Intensity"),
        vec![level(153), level(51)],
        "lap two's early move, then lap one's later one, which lap two never reached"
    );
    eventually("the fader was given back at the stop", || async {
        read::<ProgrammerValue>(&station, "programmer_values").await.is_empty()
    })
    .await;
}
