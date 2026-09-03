//! Two stations, one sync link, and a console watching the other's wire.
//!
//! Out here rather than beside the connectors because the thing being proved needs
//! a second station to exist at all: **only the station holding a socket can say
//! what went through it**, so a console looking at an output it does not run has to
//! ask the station that does, and read the answer back over the link.
//!
//! Two rules fail silently and are what these tests are for. An ask that never
//! reaches the peer shows an empty panel that reads as a connector saying nothing.
//! And an ask that is never withdrawn leaves a station drawing a universe forty
//! times a second, for ever, for a console that closed hours ago — on the network
//! that is also carrying the show.

use std::time::Duration;

use pult_backend::{Config, Running};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::output::{OutputConfig, OutputKind, OutputView, SectionBody},
};

async fn a_station() -> Running {
    let show = std::env::temp_dir().join(format!("pult-wire-{}.pult", uuid::Uuid::new_v4()));
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

async fn join(host: &Running, joiner: &Running) {
    joiner
        .sync
        .connect_peer(vec![host.sync_addr], uuid::Uuid::new_v4(), uuid::Uuid::nil())
        .await
        .expect("the two stations connect");

    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if host.sync.peer_count().await > 0 && joiner.sync.peer_count().await > 0 {
            return;
        }
    }
    panic!("the stations never saw each other");
}

/// An Art-Net output on `station`, pointed at a socket in this process so that
/// something is genuinely put on a wire and the dedup cache has an image in it.
async fn an_artnet_output(station: &Running) -> uuid::Uuid {
    let listener = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let output = OutputConfig {
        id: uuid::Uuid::new_v4(),
        name: "House".into(),
        kind: OutputKind::Artnet,
        target: Some(listener.local_addr().unwrap().to_string()),
        universes: vec![],
        enabled: true,
        node_id: Some(station.node_id),
    };
    station
        .engine
        .set(
            vec![PathSegment::Key("outputs".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            serde_json::to_value(&output).unwrap(),
        )
        .await
        .expect("the output is created");
    // Held open: a closed socket makes the first send fail with a refusal on some
    // platforms, which is a different test.
    std::mem::forget(listener);
    output.id
}

/// A fixture, so that the connector has a universe to carry at all.
async fn a_fixture(station: &Running) {
    let fixture_type = pult_schema::types::fixture::FixtureType {
        id: uuid::Uuid::new_v4(),
        name: "Dimmer".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![pult_schema::types::fixture::ParameterDefinition::new(
            pult_schema::types::fixture::ParameterKind::Intensity,
            pult_schema::types::fixture::ParameterValue::Float(0.0),
        )],
        ..Default::default()
    };
    let fixture = pult_schema::types::fixture::Fixture {
        id: uuid::Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: fixture_type.id,
        address: pult_schema::types::fixture::FixtureAddress::dmx(1, 1),
        ..Default::default()
    };
    for (table, value) in [
        ("fixture_types", serde_json::to_value(&fixture_type).unwrap()),
        ("fixtures", serde_json::to_value(&fixture).unwrap()),
    ] {
        station
            .engine
            .set(
                vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())],
                Lifecycle::Persisted,
                value,
            )
            .await
            .expect("the rig is created");
    }
}

/// Ask for a peer's output the way `output.watch` does: register the ask here, then
/// send the new answer down the link.
async fn watch(from: &Running, on: &Running, output: uuid::Uuid, who: uuid::Uuid, focus: Option<&str>) {
    let ask = from
        .viewers
        .watch(on.node_id, output, who, focus.map(str::to_string))
        .expect("the ask moved");
    from.sync.watch_peer_output(on.node_id, output, ask).await;
}

async fn let_go(from: &Running, on: &Running, output: uuid::Uuid, who: uuid::Uuid) {
    let ask = from.viewers.unwatch(on.node_id, output, who).expect("the ask moved");
    from.sync.watch_peer_output(on.node_id, output, ask).await;
}

/// The next view of `about`'s output to reach `watcher`'s browsers, or nothing.
async fn seen(
    rx: &mut tokio::sync::broadcast::Receiver<(pult_schema::path::Path, serde_json::Value)>,
    about: &Running,
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
            if view.node_id == about.node_id {
                return Some(view);
            }
        }
    }
}

#[tokio::test]
async fn a_console_sees_its_own_wire() {
    let station = a_station().await;
    a_fixture(&station).await;
    let output = an_artnet_output(&station).await;

    let mut here = station.updates.0.subscribe();
    station.viewers.watch(station.node_id, output, uuid::Uuid::new_v4(), Some("1".to_string()));

    let view = seen(&mut here, &station, Duration::from_secs(5))
        .await
        .expect("this station answers about its own output");
    assert_eq!(view.output_id, output);
}

#[tokio::test]
async fn a_console_sees_what_a_peers_wire_is_carrying() {
    let roof = a_station().await;
    let booth = a_station().await;
    a_fixture(&roof).await;
    let output = an_artnet_output(&roof).await;
    join(&roof, &booth).await;

    let mut at_the_booth = booth.updates.0.subscribe();
    watch(&booth, &roof, output, uuid::Uuid::new_v4(), Some("1")).await;

    let view = seen(&mut at_the_booth, &roof, Duration::from_secs(5))
        .await
        .expect("the roof answers within five seconds");
    assert_eq!(view.output_id, output);
    assert_eq!(view.focus.as_deref(), Some("1"));

    let SectionBody::Universes(traffic) = &view.sections[0].body else {
        panic!("Art-Net describes itself as universes");
    };
    assert_eq!(
        traffic.focused.as_ref().map(|f| f.universe),
        Some(1),
        "and the universe that was asked for is the one that came back"
    );
    assert_eq!(traffic.focused.as_ref().unwrap().channels.len(), 512);
}

#[tokio::test]
async fn a_station_nobody_is_watching_puts_nothing_on_the_link() {
    let roof = a_station().await;
    let booth = a_station().await;
    a_fixture(&roof).await;
    let output = an_artnet_output(&roof).await;
    join(&roof, &booth).await;

    let mut at_the_booth = booth.updates.0.subscribe();
    assert!(
        seen(&mut at_the_booth, &roof, Duration::from_millis(800)).await.is_none(),
        "an output nobody has asked about is drawn nowhere and sent nowhere"
    );

    // And once asked, it arrives — so the silence above was the rule and not a
    // connection that never worked.
    watch(&booth, &roof, output, uuid::Uuid::new_v4(), None).await;
    assert!(seen(&mut at_the_booth, &roof, Duration::from_secs(5)).await.is_some());
}

#[tokio::test]
async fn letting_go_stops_the_peer_drawing() {
    let roof = a_station().await;
    let booth = a_station().await;
    a_fixture(&roof).await;
    let output = an_artnet_output(&roof).await;
    join(&roof, &booth).await;

    let alice = uuid::Uuid::new_v4();
    let mut at_the_booth = booth.updates.0.subscribe();
    watch(&booth, &roof, output, alice, Some("1")).await;
    assert!(seen(&mut at_the_booth, &roof, Duration::from_secs(5)).await.is_some());

    let_go(&booth, &roof, output, alice).await;
    // Long enough for the withdrawal to cross and for several draws to have been
    // skipped: an ask that is never withdrawn is the failure nobody notices, because
    // the panel that caused it has been shut for hours.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut after = booth.updates.0.subscribe();
    assert!(
        seen(&mut after, &roof, Duration::from_millis(800)).await.is_none(),
        "the roof stopped drawing when the last watcher let go"
    );
    assert!(
        !roof.viewers.any_on(roof.node_id),
        "and it is keeping nothing on the booth's behalf"
    );
}
