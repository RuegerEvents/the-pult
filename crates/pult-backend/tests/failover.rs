//! A hot spare: two stations running the Garden's maze, and the leader killed in the
//! middle of a scare.
//!
//! Out here rather than beside the sync layer because what is being proved is the
//! whole station, not the election: that what the show was doing when the leader went
//! is what the survivor goes on doing. The election itself is tested where it lives.
//!
//! The leader is **killed**, not shut down — `Running::kill` closes its sockets and
//! stops its tasks without telling anybody, the way a kernel tidies up after `kill -9`.
//! A clean shutdown would be the case that matters least: nobody yanks a cable politely.
//!
//! Two stations in one process share one thing a real pair would not: the show clock
//! (`pult_schema::clock` is per process), so its continuity is not something this can
//! see. Each has a broker of its own behind its own door, though, so a node walking its
//! broker list to the new leader is (doc 15): a simulated node that only the dead leader
//! ever knew the address of.

use std::time::Duration;

use openhaunt_node_sim::{Input, ModuleKind, SimConfig};
use pult_backend::{
    demo::Demo,
    infra::devices::{DeviceCommand, DeviceEvent},
    Config, Running,
};
use pult_schema::{
    path::PathSegment,
    types::{station::Station, timeline::Timeline, Fixture, Sequence, Variable},
};
use uuid::Uuid;

/// Preferences of this binary's own: no sound card, so no test opens the machine's
/// speakers or its microphone, and no recent list that is the operator's.
fn own_preferences() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("pult-failover-prefs-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("a preferences directory");
        let path = dir.join("preferences.toml");
        std::fs::write(
            &path,
            "[audio]\ninput = \"no input device in tests\"\noutput = \"no output device in tests\"\n",
        )
        .expect("a preferences file");
        // SAFETY: inside `call_once`, before any station in this process has started.
        unsafe { std::env::set_var("PULT_PREFERENCES", &path) };
    });
}

/// A port nothing is listening on, for the broker: the default is 1883, which a
/// machine with Mosquitto on it already has.
fn a_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn a_station(name: &str, demo: Option<Demo>, broker: u16) -> Running {
    own_preferences();
    let show = std::env::temp_dir().join(format!("pult-failover-{name}-{}.pult", Uuid::new_v4()));
    pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        show: Some(show.clone()),
        identity: Some(show.with_extension("node")),
        demo,
        openhaunt_broker_port: broker,
        ..Config::default()
    })
    .await
    .expect("a station starts")
}

async fn read<T: serde::de::DeserializeOwned>(station: &Running, table: &str) -> Vec<T> {
    station
        .engine
        .get(vec![PathSegment::Key(table.into())])
        .await
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

async fn eventually<F, Fut>(what: &str, within: Duration, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let until = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < until {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never: {what}");
}

async fn is_follower(station: &Running) -> bool {
    station
        .engine
        .get(vec![PathSegment::Key("session".into())])
        .await
        .ok()
        .and_then(|v| v["is_follower"].as_bool())
        .unwrap_or(false)
}

async fn inside(station: &Running, name: &str) -> f64 {
    let vars: Vec<Variable> = read(station, "variables").await;
    vars.iter().find(|v| v.name == name).map(|v| v.current()).unwrap_or(f64::NAN)
}

async fn timeline(station: &Running, id: Uuid) -> Timeline {
    let timelines: Vec<Timeline> = read(station, "timelines").await;
    timelines.into_iter().find(|t| t.id == id).expect("the timeline")
}

/// A PIR seeing somebody and then not, on whichever station is driving the nodes.
async fn walk(station: &Running, fixture: Uuid, port: u8) {
    for closed in [true, false] {
        station
            .engine
            .set_sensed_value(
                fixture,
                format!("Contact:{port}"),
                serde_json::json!({ "type": "Bool", "value": closed }),
            )
            .await
            .expect("the sensor reading lands");
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

async fn lamp(station: &Running, entrance: Uuid, relay: u8) -> Option<bool> {
    let fixtures: Vec<Fixture> = read(station, "fixtures").await;
    let entrance = fixtures.iter().find(|f| f.id == entrance)?;
    entrance
        .live_fades
        .get(&format!("Switch:{relay}"))
        .map(|fade| fade.to == pult_schema::types::ParameterValue::Bool(true))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_spare_carries_on_when_the_leader_dies_mid_scare() {
    // A broker each, the way two machines have: what the node is told is two doors.
    let leader = a_station("leader", Some(Demo::Garden), a_free_port()).await;
    let spare = a_station("spare", None, a_free_port()).await;

    // By address, the way a spare is joined on a network whose mDNS does not reach —
    // and the only way here, where this process may not send multicast at all.
    let at = format!("127.0.0.1:{}", leader.sync_addr.port());
    spare
        .call_rpc("session.joinAt", serde_json::json!({ "address": at }))
        .await
        .expect("the spare joins");
    eventually("the spare is a follower", Duration::from_secs(10), || async {
        is_follower(&spare).await && !is_follower(&leader).await
    })
    .await;

    // The whole maze has to have reached the spare before it can be the spare.
    let fixtures: Vec<Fixture> = {
        let mut fixtures: Vec<Fixture> = Vec::new();
        for _ in 0..400 {
            fixtures = read(&leader, "fixtures").await;
            let seeded = fixtures.iter().any(|f| f.name == "Entrance node")
                && !read::<Variable>(&leader, "variables").await.is_empty()
                && read::<Timeline>(&leader, "timelines").await.len() >= 5;
            if seeded {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        fixtures
    };
    let node = |name: &str| fixtures.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name}")).id;
    let (area1, area2, entrance) = (node("Area 1 node"), node("Area 2 node"), node("Entrance node"));
    let timelines: Vec<Timeline> = read(&leader, "timelines").await;
    let scare1 = timelines.iter().find(|t| t.name == "Area 1 scare").expect("a scare").id;
    eventually("the spare holds the maze", Duration::from_secs(20), || async {
        read::<Fixture>(&spare, "fixtures").await.len() == fixtures.len()
            && read::<Timeline>(&spare, "timelines").await.len() == timelines.len()
            && read::<Variable>(&spare, "variables").await.len()
                == read::<Variable>(&leader, "variables").await.len()
    })
    .await;

    // Every byte the show stands on, on the spare as well, while there is still
    // somebody to fetch it from: a clip it has not got is a scare with no sound in it
    // the moment the leader goes.
    let wanted: Vec<String> = timelines
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.asset.clone()).chain(t.tracks.iter().map(|k| k.asset.clone())))
        .collect();
    assert!(!wanted.is_empty(), "the garden has sound and takes");
    let spare_assets = spare.bundle.as_ref().expect("a show is open").assets_dir();
    eventually("the spare holds every clip and take", Duration::from_secs(30), || {
        let wanted = wanted.clone();
        let dir = spare_assets.clone();
        async move { wanted.iter().all(|sha| dir.join(sha).exists()) }
    })
    .await;

    // Area 1's node, real enough to fail over: an OpenHaunt simulator the *leader* is
    // told about, the way its mDNS browse would tell it, and the spare never is. Whatever
    // brings it to the spare after the kill, it is not discovery.
    let serial = fixtures
        .iter()
        .find(|f| f.id == area1)
        .and_then(|f| f.address.serial())
        .expect("area 1 is a node")
        .to_string();
    let area1_node = openhaunt_node_sim::start(SimConfig::new(ModuleKind::NodeV0, serial.clone()))
        .await
        .expect("the simulated node starts");
    let txt = [
        ("sn", serial.as_str()),
        ("mod", "0x0f01"),
        ("name", "Area 1"),
        ("fw", "0.4.0-sim"),
        ("v", "1"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    leader
        .devices
        .0
        .send(DeviceCommand::Event(DeviceEvent::Resolved {
            serial: serial.clone(),
            ip: "127.0.0.1".into(),
            port: area1_node.http_addr.port(),
            host: format!("openhaunt-{serial}.local."),
            txt,
        }))
        .await
        .expect("the leader hears of the node");
    let station_broker = |station: &Running| {
        let station = station.node_id.0;
        let leader = &leader;
        async move {
            read::<Station>(leader, "stations")
                .await
                .into_iter()
                .find(|s| s.id == station)
                .map(|s| s.broker_addr)
                .unwrap_or_default()
        }
    };
    // Each station writes its own row on its own two-second report, and the spare's
    // reaches the leader by replication: waited for, not read once and assumed.
    eventually("both stations say where their door is", Duration::from_secs(10), || async {
        !station_broker(&leader).await.is_empty() && !station_broker(&spare).await.is_empty()
    })
    .await;
    let leader_door = station_broker(&leader).await;
    let spare_door = station_broker(&spare).await;
    eventually("the node is on the leader, and knows the spare", Duration::from_secs(20), || {
        let snapshot = area1_node.snapshot.clone();
        let (leader_door, spare_door) = (leader_door.clone(), spare_door.clone());
        async move {
            let s = snapshot.borrow().clone();
            s.mqtt_connected
                && s.broker.as_deref() == Some(leader_door.as_str())
                && s.brokers == vec![leader_door, spare_door]
        }
    })
    .await;

    // The flows meet the sensors once, then the first group walks into area 1.
    for fixture in [area1, area2] {
        for port in [4, 5] {
            leader
                .engine
                .set_sensed_value(fixture, format!("Contact:{port}"), serde_json::json!({ "type": "Bool", "value": false }))
                .await
                .unwrap();
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    walk(&leader, area1, 4).await;
    eventually("area 1 counts the group in, on both", Duration::from_secs(5), || async {
        inside(&leader, "Area 1 inside").await == 1.0 && inside(&spare, "Area 1 inside").await == 1.0
    })
    .await;
    eventually("the scare runs, on both", Duration::from_secs(5), || async {
        timeline(&spare, scare1).await.running
    })
    .await;

    // What the scare has already done, as the spare saw it: a sequence Go fired at a
    // position before the kill must not be fired again by the station taking over.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = timeline(&spare, scare1).await;
    let gone_before: Vec<(Uuid, Option<u64>)> = read::<Sequence>(&spare, "sequences")
        .await
        .into_iter()
        .map(|s| (s.id, s.went_at))
        .collect();

    // ── The leader dies. ──
    let dead = leader.node_id;
    leader.kill().await;

    eventually("the spare takes over", Duration::from_secs(20), || async {
        !is_follower(&spare).await
    })
    .await;
    assert_ne!(spare.sync.leader().await, Some(dead), "nobody is waiting for the dead one");

    // The maze's state survived: SYNCED, so the spare had it all along.
    assert_eq!(inside(&spare, "Area 1 inside").await, 1.0, "occupancy survives the failover");

    // The scare goes on from where it was rather than starting again.
    let after = timeline(&spare, scare1).await;
    assert_eq!(after.anchor_ms, before.anchor_ms, "nobody restarted the scare");
    assert_eq!(
        after.position_at_anchor_ms, before.position_at_anchor_ms,
        "or located it",
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let gone_after: Vec<(Uuid, Option<u64>)> = read::<Sequence>(&spare, "sequences")
        .await
        .into_iter()
        .map(|s| (s.id, s.went_at))
        .collect();
    let fired_at_kill = before.position_at(pult_schema::clock::now_ms().saturating_sub(500));
    for event in &before.events {
        if u64::from(event.at_ms) + 50 < fired_at_kill {
            let Some(target) = event.action.sequence_id() else { continue };
            let was = gone_before.iter().find(|(id, _)| *id == target).map(|(_, at)| *at);
            let is = gone_after.iter().find(|(id, _)| *id == target).map(|(_, at)| *at);
            assert_eq!(was, is, "an event crossed before the kill was not fired again");
        }
    }

    // The flows now fire on the spare, once each: the group leaves, and the count is
    // zero rather than minus one; two more walk in and it is two rather than four.
    walk(&spare, area1, 5).await;
    eventually("the group is counted out by the spare", Duration::from_secs(5), || async {
        inside(&spare, "Area 1 inside").await == 0.0
    })
    .await;
    walk(&spare, area1, 4).await;
    walk(&spare, area1, 4).await;
    eventually("two groups in", Duration::from_secs(5), || async {
        inside(&spare, "Area 1 inside").await == 2.0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(inside(&spare, "Area 1 inside").await, 2.0, "each edge counted once");

    // The node walked its list to the spare, which never saw it over mDNS, and its next
    // edge counts once there: three in, not two and not four.
    eventually("the node is on the spare's broker", Duration::from_secs(30), || {
        let snapshot = area1_node.snapshot.clone();
        let spare_door = spare_door.clone();
        async move {
            let s = snapshot.borrow().clone();
            s.mqtt_connected && s.broker.as_deref() == Some(spare_door.as_str())
        }
    })
    .await;
    assert!(
        area1_node.snapshot.borrow().brokers.first() == Some(&spare_door),
        "and was told the spare leads now",
    );
    for closed in [true, false] {
        area1_node.inputs.send(Input::Contact { port: 4, state: closed }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    eventually("the node's own edge counts a third group in", Duration::from_secs(5), || async {
        inside(&spare, "Area 1 inside").await == 3.0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(inside(&spare, "Area 1 inside").await, 3.0, "the node's edge counted once");

    // The light at the entrance follows area 2 on the spare.
    walk(&spare, area2, 4).await;
    eventually("red while area 2 is busy", Duration::from_secs(5), || async {
        lamp(&spare, entrance, 0).await == Some(true) && lamp(&spare, entrance, 1).await == Some(false)
    })
    .await;
    walk(&spare, area2, 5).await;
    eventually("green when it is free", Duration::from_secs(5), || async {
        lamp(&spare, entrance, 1).await == Some(true) && lamp(&spare, entrance, 0).await == Some(false)
    })
    .await;

    // The outputs moved: the spare's own row says it is the leader and runs the
    // OpenHaunt output that drives the nodes.
    eventually("the spare's row says it leads and drives the nodes", Duration::from_secs(10), || async {
        read::<Station>(&spare, "stations")
            .await
            .iter()
            .find(|s| s.id == spare.node_id.0)
            .is_some_and(|s| s.is_leader && s.output_plugins.iter().any(|p| p.contains("penHaunt")))
    })
    .await;

    area1_node.stop.stop().await;
    spare.shutdown("the test is over").await;
}
