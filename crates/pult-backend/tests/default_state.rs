//! The show as it comes up: saved from what is running, and put back by one act.

use pult_backend::{infra::default_state, Config, Running};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    registry::path_lifecycle,
    types::{
        cue::{Cue, FollowMode},
        sequence::Sequence,
        timeline::{Timeline, TimelineSource},
    },
};
use serde_json::json;
use uuid::Uuid;

async fn a_station() -> Running {
    let show = std::env::temp_dir().join(format!("pult-default-{}.pult", Uuid::new_v4()));
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

fn field(table: &str, id: Uuid, name: &str) -> Vec<PathSegment> {
    vec![PathSegment::Key(table.into()), PathSegment::Id(id), PathSegment::Key(name.into())]
}

async fn write(station: &Running, path: Vec<PathSegment>, value: serde_json::Value) {
    let lifecycle = path_lifecycle(&path);
    station.engine.set(path, lifecycle, value).await.expect("the write lands");
}

async fn create(station: &Running, table: &str, value: serde_json::Value) {
    let path = vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())];
    station.engine.set(path, Lifecycle::Persisted, value).await.expect("the row is created");
}

async fn sequence(station: &Running, id: Uuid) -> Sequence {
    let row = station.engine.get(vec![PathSegment::Key("sequences".into()), PathSegment::Id(id)]).await.unwrap();
    serde_json::from_value(row).unwrap()
}

async fn running(station: &Running, id: Uuid) -> bool {
    let row = station.engine.get(field("timelines", id, "running")).await.unwrap();
    row.as_bool().unwrap_or(false)
}

#[tokio::test]
async fn a_saved_default_state_comes_back_in_one_act() {
    let station = a_station().await;
    let who = Uuid::new_v4();

    let cues: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
    for (i, id) in cues.iter().enumerate() {
        let cue = Cue {
            id: *id,
            name: format!("Cue {}", i + 1),
            number: (i + 1) as f64,
            captures: Vec::new(),
            follow_mode: FollowMode::Manual,
            fade_in_ms: 0,
            fade_out_ms: 0,
            easing: None,
            is_active: false,
        };
        create(&station, "cues", serde_json::to_value(&cue).unwrap()).await;
    }
    let house = Uuid::new_v4();
    let other = Uuid::new_v4();
    for (id, ids) in [(house, cues.clone()), (other, vec![cues[0]])] {
        create(&station, "sequences", json!({ "id": id, "name": "S", "cue_ids": ids, "active_cue_index": null, "went_at": null, "master": null })).await;
    }
    let bed = Timeline {
        id: Uuid::new_v4(),
        name: "Ambience".into(),
        clips: Vec::new(),
        detected: None,
        source: TimelineSource::default(),
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
        folder: None,
    };
    create(&station, "timelines", serde_json::to_value(&bed).unwrap()).await;

    // The house sequence on cue 2 at 60%, the ambience running: saved.
    write(&station, field("sequences", house, "goToCue"), json!({ "cueId": cues[1] })).await;
    write(&station, field("sequences", house, "master"), json!(0.6)).await;
    write(&station, field("timelines", bed.id, "play"), json!({ "fromMs": 0 })).await;
    let saved = default_state::save(&station.engine, who).await.expect("saved");
    assert_eq!((saved.sequences, saved.timelines), (1, 1));

    // Then somebody runs the show somewhere else entirely.
    write(&station, field("sequences", house, "off"), json!({})).await;
    write(&station, field("sequences", house, "master"), json!(null)).await;
    write(&station, field("sequences", other, "goNext"), json!({})).await;
    write(&station, field("timelines", bed.id, "stop"), json!({})).await;

    assert!(default_state::apply(&station.engine, who).await.expect("applied"));
    let house_now = sequence(&station, house).await;
    assert_eq!(house_now.active_cue_index, Some(1), "back on cue 2");
    assert_eq!(house_now.master, Some(0.6), "at its level");
    assert_eq!(sequence(&station, other).await.active_cue_index, None, "and what it did not name is off");
    assert!(running(&station, bed.id).await, "and the ambience runs again");
}
