//! Counts, not milliseconds.
//!
//! `scripts/demo.sh --measure` is a script somebody runs before a release, and it is
//! deliberately not a gate: two runs of the same show vary by a few per cent on one
//! machine and by far more across machines, so a threshold in milliseconds on a shared
//! runner flaps, a flapping gate gets disabled, and a disabled gate is worse than
//! none.
//!
//! What *can* be a gate is a figure that does not flap. These three are counts —
//! machine-independent, the same on a laptop and on a runner — and each one guards a
//! property that used to be found in a theatre rather than in CI:
//!
//! - **A running show pushes nothing at a browser.** Nothing stores what a parameter
//!   is doing, so a fade in progress is not a change to the show. If this ever counts
//!   above zero, the engine has started writing values again and every console on the
//!   network is back to receiving a few thousand messages a second during a cue.
//! - **A gesture is one row.** One drag of a beam spot must be one thing to take back,
//!   not one per frame. This is what makes Ctrl-Z usable rather than a key somebody
//!   holds down.
//! - **A settled rig changes no universes.** The DMX dedup is what keeps an idle show
//!   off the network. A regression here is invisible on screen and floods a wire.
//!
//! A fourth candidate — engine messages per cue take — was considered and left out on
//! purpose. Its honest bound varies with the number of sequences and captures, and a
//! gate whose threshold is arguable gets loosened until it means nothing.

use std::time::Duration;

use futures::StreamExt;

use pult_backend::{Config, Running};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        fixture::{
            Fixture, FixtureAddress, FixtureType, ParameterDefinition, ParameterKind,
            ParameterValue,
        },
        output::{OutputConfig, OutputKind, OutputView, SectionBody},
    },
};
use uuid::Uuid;

async fn a_station() -> Running {
    let show = std::env::temp_dir().join(format!("pult-counts-{}.pult", Uuid::new_v4()));
    pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        show: Some(show.clone()),
        // Told, rather than taken from the machine: two stations in one test binary
        // sharing an id would break the vector clock's tie-break.
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
        .expect("the row is created");
}

/// A rig of dimmers, enough of them that a per-fixture push would be unmistakable.
async fn a_rig(station: &Running, count: usize) -> Vec<Uuid> {
    let fixture_type = FixtureType {
        id: Uuid::new_v4(),
        name: "Dimmer".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![ParameterDefinition::new(
            ParameterKind::Intensity,
            ParameterValue::Float(0.0),
        )],
        ..Default::default()
    };
    create(station, "fixture_types", serde_json::to_value(&fixture_type).unwrap()).await;

    let mut ids = Vec::new();
    for n in 0..count {
        let fixture = Fixture {
            id: Uuid::new_v4(),
            name: format!("Dimmer {n}"),
            fixture_type_id: fixture_type.id,
            address: FixtureAddress::dmx(1 + (n as u16 / 512), 1 + (n as u16 % 512)),
            ..Default::default()
        };
        ids.push(fixture.id);
        create(station, "fixtures", serde_json::to_value(&fixture).unwrap()).await;
    }
    ids
}

/// An Art-Net output pointed at a socket in this process, so something genuinely
/// reaches a wire and the dedup cache has an image to compare against.
async fn an_output(station: &Running) -> Uuid {
    let listener = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let output = OutputConfig {
        id: Uuid::new_v4(),
        name: "House".into(),
        kind: OutputKind::Artnet,
        target: Some(listener.local_addr().unwrap().to_string()),
        universes: vec![],
        enabled: true,
        node_id: Some(station.node_id),
    };
    create(station, "outputs", serde_json::to_value(&output).unwrap()).await;
    // Held open: a closed socket makes the first send fail with a refusal on some
    // platforms, which is a different test.
    std::mem::forget(listener);
    output.id
}

/// How many updates about `fixtures` the station pushes over `window`.
async fn fixture_updates(station: &Running, window: Duration) -> usize {
    let mut updates = station.updates.subscribe_all();
    let mut seen = 0usize;
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return seen;
        }
        match tokio::time::timeout(left, updates.next()).await {
            Ok(Some((path, _))) => {
                if matches!(path.first(), Some(PathSegment::Key(key)) if key == "fixtures") {
                    seen += 1;
                }
            }
            Ok(None) => return seen,
            Err(_) => return seen,
        }
    }
}

#[tokio::test]
async fn a_running_fade_pushes_nothing_at_a_browser() {
    let station = a_station().await;
    let fixtures = a_rig(&station, 200).await;
    an_output(&station).await;

    // Put a long fade on every fixture, the way taking a cue would.
    for id in &fixtures {
        station
            .engine
            .set(
                vec![
                    PathSegment::Key("programmer_values".into()),
                    PathSegment::Key("__create".into()),
                ],
                Lifecycle::Synced,
                serde_json::json!({
                    "id": Uuid::new_v4(),
                    "fixture_id": id,
                    "parameter_kind": "Intensity",
                    "value": { "type": "Float", "value": 1.0 },
                }),
            )
            .await
            .ok();
    }

    // Let the burst of writes drain. What is being counted is the *running* show, not
    // the act of starting it — the writes above are a change to the show and are
    // supposed to be pushed.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let pushed = fixture_updates(&station, Duration::from_secs(4)).await;
    assert_eq!(
        pushed, 0,
        "a show that is merely running pushed {pushed} fixture updates in four seconds. \
         Nothing stores what a parameter is doing, so there is nothing to push: if this \
         is above zero the engine has started writing values again, and every connected \
         console is back to a few thousand messages a second during a cue."
    );
}

#[tokio::test]
async fn one_gesture_is_one_row_however_many_writes_it_took() {
    let station = a_station().await;
    let fixtures = a_rig(&station, 1).await;
    let who = Uuid::new_v4();
    let gesture = Uuid::new_v4();

    // A drag, as the rig view makes one: a value written per animation frame, all of
    // it inside one gesture because it is one act.
    for step in 0..60 {
        station
            .engine
            .set_as(
                who,
                Some(gesture),
                vec![
                    PathSegment::Key("fixtures".into()),
                    PathSegment::Id(fixtures[0]),
                    PathSegment::Key("home_values".into()),
                ],
                Lifecycle::Persisted,
                serde_json::json!({ "Intensity": { "type": "Float", "value": step as f32 / 60.0 } }),
            )
            .await
            .expect("the drag writes");
    }

    let history = station.engine.history(50).await;
    let mine =
        history.iter().filter(|entry| entry.gesture == Some(gesture)).count();
    assert_eq!(
        mine, 1,
        "a drag of sixty frames became {mine} entries in the history. One drag is one \
         act and has to be one Ctrl-Z: if this grows, taking back an aim becomes a key \
         somebody holds down."
    );
}

/// The same, for the thing an operator drags far more often than a light: a truss.
///
/// A gizmo writes a placement per animation frame, and moving a bar across a stage is
/// one act however many frames it took. The fixture case above is the mirror of this
/// one, and the two are here rather than one because they go through different paths:
/// a programmer value and a `scene_objects` transform.
#[tokio::test]
async fn a_dragged_truss_is_one_row() {
    let station = a_station().await;
    let who = Uuid::new_v4();
    let gesture = Uuid::new_v4();

    let truss = Uuid::new_v4();
    station
        .engine
        .set(
            vec![
                PathSegment::Key("scene_objects".into()),
                PathSegment::Key("__create".into()),
            ],
            Lifecycle::Persisted,
            serde_json::json!({
                "id": truss,
                "name": "Downstage bar",
                "kind": "Truss",
                "transform": {
                    "position": { "x": 0.0, "y": 6.0, "z": 0.0 },
                    "rotation": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                },
                "parent": null,
                "layer": null,
                "class": null,
                "geometry": [],
                "symbol": null,
                "catalogue": "f34-3m",
                "properties": {},
                "locked": false
            }),
        )
        .await
        .expect("the truss is drawn");

    for step in 0..60 {
        station
            .engine
            .set_as(
                who,
                Some(gesture),
                vec![
                    PathSegment::Key("scene_objects".into()),
                    PathSegment::Id(truss),
                    PathSegment::Key("transform".into()),
                ],
                Lifecycle::Persisted,
                serde_json::json!({
                    "position": { "x": step as f32 / 60.0, "y": 6.0, "z": 0.0 },
                    "rotation": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                }),
            )
            .await
            .expect("the drag writes");
    }

    let history = station.engine.history(100).await;
    let mine = history.iter().filter(|entry| entry.gesture == Some(gesture)).count();
    assert_eq!(
        mine, 1,
        "dragging a truss across sixty frames became {mine} entries in the history. \
         Moving a bar is one act and has to be one Ctrl-Z: if this grows, putting a \
         truss back where it was becomes a key somebody holds down."
    );
}

#[tokio::test]
async fn a_settled_rig_changes_no_universes() {
    let station = a_station().await;
    a_rig(&station, 50).await;
    let output = an_output(&station).await;

    // Watch universe 1, which is what opening the wire panel on it does.
    let mut here = station.updates.0.subscribe();
    station.viewers.watch(station.node_id, output, Uuid::new_v4(), Some("1".to_string()));

    /// The next drawn view of this station's own output.
    async fn seen(
        rx: &mut tokio::sync::broadcast::Receiver<(pult_schema::path::Path, serde_json::Value)>,
        within: Duration,
    ) -> Option<OutputView> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return None;
            }
            let Ok(Ok((path, value))) = tokio::time::timeout(left, rx.recv()).await else {
                return None;
            };
            if path.first() != Some(&PathSegment::Key("output_traffic".into())) {
                continue;
            }
            if let Ok(view) = serde_json::from_value::<OutputView>(value) {
                return Some(view);
            }
        }
    }

    // Let the rig settle: everything above is a change to the show, and a universe
    // that has just been patched has genuinely just changed.
    // Long enough that every universe's last *change* is the patching above and is
    // well over a second old. A settled connector still draws views, for its
    // keep-alive, so this waits rather than draining until empty — the stream never
    // goes empty, which is itself the thing being relied on below.
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Now nothing is moving. Whatever the connector draws from here on may still be
    // *sent* — a keep-alive is not movement — but it must not have *changed*.
    //
    // The first second of views is discarded, for the same reason the measurement
    // script discards its first window: the rig coming up is a genuine change, and a
    // view drawn just after it honestly reports one. Without this the test counts the
    // show starting and calls it a leak. That the count did **not** grow when the
    // window was tripled is what said it was one event rather than a continuous one.
    let mut drawn = 0usize;
    let mut changed_recently = 0usize;
    let begin = tokio::time::Instant::now() + Duration::from_secs(1);
    let until = begin + Duration::from_secs(5);
    while tokio::time::Instant::now() < until {
        let Some(view) = seen(&mut here, Duration::from_secs(2)).await else { break };
        if tokio::time::Instant::now() < begin {
            continue;
        }
        let Some(SectionBody::Universes(traffic)) = view.sections.first().map(|s| &s.body) else {
            continue;
        };
        drawn += 1;
        // A universe whose image changed inside the last second, on a rig where
        // nothing is driving anything, is the dedup having stopped working.
        changed_recently += traffic.universes.iter().filter(|u| u.changed_ms_ago < 1000).count();
    }

    assert_eq!(
        changed_recently, 0,
        "a settled rig reported {changed_recently} universes as having changed inside the \
         last second, across {drawn} drawn views. Nothing is driving anything, so no \
         universe image can differ from the one before it: if this is above zero the DMX \
         dedup has stopped working and every idle show is flooding its network."
    );
}
