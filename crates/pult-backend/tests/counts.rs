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
//! property that used to be found in a theater rather than in CI:
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
        interfaces: Default::default(),
        priority: Default::default(),
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

/// A cue-only store and an Update across three cues are each **one Ctrl-Z**.
///
/// Both are one act and both are several writes to *different* rows: a cue-only store
/// writes the cue and the compensation in the one after it, and an Update writes every
/// cue driving a key the operator has hold of. The oplog folds repeats of one path
/// inside a gesture into one row and cannot fold writes to different rows — so what is
/// asserted here is the thing that actually matters: every write carries the same
/// gesture, and one undo takes all of them back.
///
/// The writes are made the way the browser makes them rather than by the browser,
/// because the property under test belongs to the engine: that a gesture holds across a
/// burst spanning several rows, which is exactly what `stores/gesture.ts` relies on.
#[tokio::test]
async fn a_store_and_an_update_are_each_one_undo() {
    let station = a_station().await;
    let fixtures = a_rig(&station, 3).await;
    let who = Uuid::new_v4();

    let cue = |number: f64| {
        serde_json::json!({
            "id": Uuid::new_v4(),
            "name": format!("Cue {number}"),
            "number": number,
            "captures": [],
            "follow_mode": "Manual",
            "fade_in_ms": 0,
            "fade_out_ms": 0,
            "easing": null,
            "is_active": false,
        })
    };
    let cues: Vec<serde_json::Value> = (1..=3).map(|n| cue(n as f64)).collect();
    for row in &cues {
        create(&station, "cues", row.clone()).await;
    }

    let capture = |fixture: Uuid, level: f32| {
        serde_json::json!({
            "fixture_id": fixture,
            "parameter_kind": "Intensity",
            "value": { "type": "Float", "value": level },
            "fade_in_ms": 0,
            "fade_out_ms": 0,
            "delay_in_ms": 0,
            "effect": null,
            "easing": null,
            "preset": null,
        })
    };

    let ids: Vec<Uuid> =
        cues.iter().map(|row| serde_json::from_value(row["id"].clone()).unwrap()).collect();
    let gesture = Uuid::new_v4();
    for (index, id) in ids.iter().enumerate() {
        station
            .engine
            .set_as(
                who,
                Some(gesture),
                vec![
                    PathSegment::Key("cues".into()),
                    PathSegment::Id(*id),
                    PathSegment::Key("captures".into()),
                ],
                Lifecycle::Persisted,
                serde_json::json!([capture(fixtures[index], 0.5)]),
            )
            .await
            .expect("the store writes");
    }

    let history = station.engine.history(50).await;
    let gestures: std::collections::HashSet<_> =
        history.iter().filter_map(|entry| entry.gesture).collect();
    assert_eq!(
        gestures.len(),
        1,
        "a store across three cues was {} acts. Cue only and Update are each one, and \
         an operator taking one back must not have to guess how many presses it takes.",
        gestures.len()
    );

    // And one undo puts all three back, which is what "one Ctrl-Z" actually means.
    station.engine.undo(who, false).await;
    for id in &ids {
        let captures = station
            .engine
            .get(vec![
                PathSegment::Key("cues".into()),
                PathSegment::Id(*id),
                PathSegment::Key("captures".into()),
            ])
            .await
            .expect("the cue is readable");
        assert_eq!(
            captures.as_array().map(Vec::len),
            Some(0),
            "one undo left a cue still holding what the store put in it"
        );
    }
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
                "catalog": "f34-3m",
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

/// The same rule, for the block of a paperwork sheet.
///
/// A sheet's `blocks` is one column, so dragging a viewport rewrites the whole array
/// once per animation frame — sixty writes of a list, where the truss above writes one
/// transform. If the gesture is dropped somewhere on that path, putting a viewport back
/// where it was is a key somebody holds down, and each press moves it a pixel.
#[tokio::test]
async fn a_dragged_sheet_block_is_one_row() {
    let station = a_station().await;
    let who = Uuid::new_v4();
    let gesture = Uuid::new_v4();

    let sheet = Uuid::new_v4();
    let block = |x: f32| {
        serde_json::json!([{
            "type": "Viewport",
            "rect": { "x": x, "y": 18.0, "w": 190.0, "h": 200.0 },
            "title": "Plan",
            "view": "Plan",
            "projection": "Orthographic",
            "scale": { "type": "Fit" },
            "style": { "type": "Drafting", "lines": "Hidden", "ink": "Mono" },
            "layers": null,
            "labels": [],
            "scale_bar": true,
            "orientation_mark": true
        }])
    };

    station
        .engine
        .set(
            vec![PathSegment::Key("sheets".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            serde_json::json!({
                "id": sheet,
                "name": "Plan",
                "sort_order": 0,
                "paper": "A3",
                "landscape": true,
                "frame": true,
                "title_block": true,
                "blocks": block(18.0)
            }),
        )
        .await
        .expect("the sheet is made");

    for step in 0..60 {
        station
            .engine
            .set_as(
                who,
                Some(gesture),
                vec![
                    PathSegment::Key("sheets".into()),
                    PathSegment::Id(sheet),
                    PathSegment::Key("blocks".into()),
                ],
                Lifecycle::Persisted,
                block(18.0 + step as f32),
            )
            .await
            .expect("the drag writes");
    }

    let history = station.engine.history(100).await;
    let mine = history.iter().filter(|entry| entry.gesture == Some(gesture)).count();
    assert_eq!(
        mine, 1,
        "dragging a block across sixty frames became {mine} entries in the history. \
         Laying out a sheet is a drag like any other and has to be one Ctrl-Z."
    );
}

/// Dragging a clip and the event above it together on a timeline's lanes rewrites two
/// columns per animation frame, `clips` and `events`, because each is one column of the
/// timeline row. The oplog folds the sixty writes of each column into one row and keeps
/// the two columns apart, so the gate is the one `a_store_and_an_update_are_each_one_undo`
/// uses: one row per column, both carrying the gesture, and **one undo puts both back** —
/// a group drag that undid half of itself would leave the Go off its sound.
#[tokio::test]
async fn dragging_a_clip_and_an_event_together_is_one_row() {
    let station = a_station().await;
    let who = Uuid::new_v4();
    let gesture = Uuid::new_v4();
    let timeline = Uuid::new_v4();
    let clip = Uuid::new_v4();
    let event = Uuid::new_v4();
    let sequence = Uuid::new_v4();
    let zone = Uuid::new_v4();

    station
        .engine
        .set(
            vec![PathSegment::Key("timelines".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            serde_json::json!({
                "id": timeline, "name": "Area 2 scare", "clips": [], "detected": null,
                "source": "Internal", "grid": [], "markers": [], "events": [], "tracks": [],
                "speed_master": null, "node_id": null, "end": null, "running": false,
                "anchor_ms": 0, "position_at_anchor_ms": 0, "rate": 1.0, "recording": null
            }),
        )
        .await
        .expect("the timeline is made");

    let column = |field: &str| {
        vec![PathSegment::Key("timelines".into()), PathSegment::Id(timeline), PathSegment::Key(field.into())]
    };
    for step in 0..60u64 {
        let clips = serde_json::json!([{
            "id": clip, "name": "Skeleton laugh", "asset": "00", "peaks": null,
            "start_ms": 300 + step * 10, "offset_ms": 0, "length_ms": null, "gain_db": 0.0,
            "fade_in_ms": 0, "fade_out_ms": 0, "looped": false,
            "destination": { "Zone": { "zone": zone } }
        }]);
        let events = serde_json::json!([{
            "id": event, "at_ms": step * 10, "action": { "GoNext": { "sequence_id": sequence } }
        }]);
        station.engine.set_as(who, Some(gesture), column("clips"), Lifecycle::Persisted, clips).await.expect("the clip moves");
        station.engine.set_as(who, Some(gesture), column("events"), Lifecycle::Persisted, events).await.expect("the event moves");
    }

    let history = station.engine.history(200).await;
    let mine = history.iter().filter(|entry| entry.gesture == Some(gesture)).count();
    assert_eq!(
        mine, 2,
        "sixty frames of two columns became {mine} rows; the oplog should fold each column's \
         repeats into one"
    );

    station.engine.undo(who, false).await;
    for (field, empty) in [("clips", "the clip"), ("events", "the event")] {
        let now = station.engine.get(column(field)).await.expect("the timeline is readable");
        assert_eq!(
            now.as_array().map(Vec::len),
            Some(0),
            "one undo left {empty} where the drag put it: a group drag is one Ctrl-Z"
        );
    }
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

/// A maze running all night writes **no** History rows.
///
/// Every PIR edge is a sensed value, every edge a flow counts is a variable write, and
/// a night is thousands of each. All of it is unattributed — the oplog keeps it for a
/// peer catching up and prunes it by age — so none of it is a row in the History panel
/// or something for Ctrl-Z. If this counts above zero, an operator's own edits are
/// being pushed out of their undo by people walking through a maze.
#[tokio::test]
async fn a_pir_counting_into_a_variable_writes_no_history() {
    use pult_schema::types::{
        flow::{Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerAction, TriggerCondition, TriggerSource},
        Variable, VariableKind, VariableOp,
    };

    let station = a_station().await;
    let pir = a_rig(&station, 1).await[0];
    let inside = Uuid::new_v4();
    create(
        &station,
        "variables",
        serde_json::to_value(Variable {
            folder: None,
            id: inside,
            name: "Area 1 inside".into(),
            kind: VariableKind::Number,
            initial: 0.0,
            min: Some(0.0),
            max: None,
            value: None,
        })
        .unwrap(),
    )
    .await;

    let flow = Uuid::new_v4();
    create(&station, "flows", serde_json::json!(Flow { folder: None, id: flow, name: "Count in".into(), enabled: true })).await;
    let node = |kind: FlowNodeKind| FlowNode {
        id: Uuid::new_v4(),
        flow_id: flow,
        kind,
        x: 0.0,
        y: 0.0,
        active: false,
        last_fired_at: None,
    };
    let source = node(FlowNodeKind::Source(TriggerSource::Parameter {
        fixture_id: pir,
        parameter: ParameterKind::Contact(0),
    }));
    let edge = node(FlowNodeKind::Condition(TriggerCondition::RisingEdge));
    let count = node(FlowNodeKind::Action(TriggerAction::SetVariable {
        variable_id: inside,
        op: VariableOp::Add(1.0),
    }));
    for (from, to) in [(&source, &edge), (&edge, &count)] {
        create(
            &station,
            "flow_edges",
            serde_json::to_value(FlowEdge {
                id: Uuid::new_v4(),
                flow_id: flow,
                from_node: from.id,
                from_port: 0,
                to_node: to.id,
                to_port: 0,
            })
            .unwrap(),
        )
        .await;
    }
    for n in [&source, &edge, &count] {
        create(&station, "flow_nodes", serde_json::to_value(n).unwrap()).await;
    }

    let value = || async {
        station
            .engine
            .get(vec![
                PathSegment::Key("variables".into()),
                PathSegment::Id(inside),
                PathSegment::Key("value".into()),
            ])
            .await
            .ok()
            .and_then(|v| v.as_f64())
    };
    for walked in 1..=5 {
        for closed in [true, false] {
            station
                .engine
                .set_sensed_value(pir, "Contact:0".into(), serde_json::json!({ "type": "Bool", "value": closed }))
                .await
                .expect("the PIR reports");
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while value().await != Some(walked as f64) {
            assert!(tokio::time::Instant::now() < deadline, "group {walked} was never counted");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    // A phone resetting it is the same kind of write.
    station.engine.set_variable(inside, VariableOp::Reset).await.expect("a reset lands");

    let rows = station.engine.history(50).await.len();
    assert_eq!(
        rows, 0,
        "five groups through a PIR and a reset became {rows} History rows. None of that is \
         anybody's edit, and every row of it pushes an operator's own work out of Ctrl-Z."
    );
}

/// A new show's effect pool is there, and nobody put it there: seeding it is not an
/// operator's act, so the first Ctrl-Z on a fresh show reaches the operator's own first
/// change rather than taking "Fire" out of the pool.
#[tokio::test]
async fn a_new_show_has_its_effect_pool_and_no_history_for_it() {
    let station = a_station().await;
    let pool = station
        .engine
        .get(vec![PathSegment::Key("effect_templates".into())])
        .await
        .expect("the pool");
    let count = match &pool {
        serde_json::Value::Object(rows) => rows.len(),
        serde_json::Value::Array(rows) => rows.len(),
        _ => 0,
    };
    assert_eq!(count, 9, "the Halloween templates are seeded: {pool}");

    let history = station.engine.history(100).await;
    let seeded = history
        .iter()
        .filter(|op| op.path.first() == Some(&PathSegment::Key("effect_templates".into())))
        .count();
    assert_eq!(seeded, 0, "seeding the pool wrote {seeded} rows into the history");
}

/// Applying a template across a selection is a write per fixture per layer, to as
/// many programmer rows, and it is one act: the Effects panel opens a gesture round all
/// of them. As with a store across three cues, the oplog cannot fold writes to
/// different rows, so what is asserted is what one Ctrl-Z means: one gesture, and one
/// undo takes every fixture's effect back.
#[tokio::test]
async fn applying_an_effect_to_a_selection_is_one_undo() {
    let station = a_station().await;
    let fixtures = a_rig(&station, 12).await;
    let who = Uuid::new_v4();
    let gesture = Uuid::new_v4();

    for (i, fixture) in fixtures.iter().enumerate() {
        station
            .engine
            .set_as(
                who,
                Some(gesture),
                vec![PathSegment::Key("programmer_values".into()), PathSegment::Key("__create".into())],
                Lifecycle::Synced,
                serde_json::json!({
                    "id": Uuid::new_v4(),
                    "fixture_id": fixture,
                    "parameter_kind": "Intensity",
                    "value": { "type": "Float", "value": 0.0 },
                    "effect": {
                        "effect_id": Uuid::nil(),
                        "curve": { "Shape": "Flicker" },
                        "rate": { "Hz": 3.0 },
                        "low": { "type": "Float", "value": 0.35 },
                        "high": { "type": "Float", "value": 1.0 },
                        "width": 0.5,
                        "direction": "Forward",
                        "phase": i as f32 / 12.0,
                        "spread": { "Random": { "seed": 1 } },
                        "t0": 1000,
                        "template": Uuid::nil()
                    },
                    "preset": null,
                    "locked": false
                }),
            )
            .await
            .expect("the programmer takes the effect");
    }

    let history = station.engine.history(100).await;
    let gestures: std::collections::HashSet<_> =
        history.iter().filter_map(|entry| entry.gesture).collect();
    assert_eq!(gestures.len(), 1, "applying one effect to twelve fixtures was {} acts", gestures.len());

    station.engine.undo(who, false).await;
    let held = station
        .engine
        .get(vec![PathSegment::Key("programmer_values".into())])
        .await
        .expect("the programmer");
    let left = match &held {
        serde_json::Value::Object(rows) => rows.len(),
        serde_json::Value::Array(rows) => rows.len(),
        _ => 0,
    };
    assert_eq!(left, 0, "one undo left {left} fixtures still running the effect");
}
