//! Two consoles in one MVR-xchange group, over a real socket.
//!
//! Out here rather than beside the manager because what is being proved needs a second
//! console to exist at all: a commit is only a commit when somebody else can fetch it,
//! and an applied one is only right when the rig it lands in is the rig that left.
//!
//! **The WebSocket half is the one that matters most**, and that is not an accident of
//! what is easy to test. The TCP half can be checked at a venue against a grandMA3 or a
//! Vectorworks; a group this console *hosts* has no second implementation anywhere, so
//! these tests are the only thing that will ever have exercised it.
//!
//! Discovery is deliberately not in here. mDNS on a build machine says more about the
//! machine than about this code, so the TCP tests hand the manager the station it would
//! have discovered and prove everything past that point. What discovery itself does is
//! in the unit tests, where `mdns-sd` is asked whether it will accept a group as part
//! of a service type — the one thing about it that could actually break.

use std::time::Duration;

use pult_backend::{Config, Running};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        fixture::{FixtureAddress, ParameterDefinition, ParameterKind, ParameterValue},
        Fixture, FixtureType, XchangeAsk, XchangeMode, XchangeSettings, XchangeState,
    },
};

async fn a_station(name: &str) -> Running {
    let show = std::env::temp_dir().join(format!("pult-xchange-{name}-{}.pult", uuid::Uuid::new_v4()));
    // Its own cache directory per station, or two consoles in one process would share
    // the commits they are meant to be exchanging.
    std::env::set_var(
        "PULT_XCHANGE_CACHE",
        std::env::temp_dir().join(format!("pult-xchange-cache-{}", uuid::Uuid::new_v4())),
    );
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

/// Switch the exchange on, in the mode a test wants.
///
/// Waits for the show row first. A station seeds its show on a task — the listener is
/// bound before the writes, deliberately — so a test that wrote settings straight away
/// would have them overwritten by the seed a moment later, and then sit watching a
/// console that is off for a reason it cannot see.
async fn exchanging(station: &Running, settings: XchangeSettings) {
    for _ in 0..400 {
        // Waiting for the *row*, not for the path: `show` answers as a singleton with
        // no row in it, so a wait that only asked whether the path resolved would go
        // on to write settings the seed then overwrote — which is what this looked
        // like, and only under a loaded machine.
        let seeded = station
            .engine
            .get(vec![PathSegment::Key("show".into())])
            .await
            .is_ok_and(|show| show.get("id").is_some_and(|id| id.is_string()));
        if seeded {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    station
        .engine
        .set(
            vec![PathSegment::Key("show".into()), PathSegment::Key("mvr_xchange".into())],
            Lifecycle::Persisted,
            serde_json::to_value(&settings).unwrap(),
        )
        .await
        .expect("the show takes its exchange settings");

    let back = station.engine.get(vec![PathSegment::Key("show".into())]).await.unwrap();
    let stored: XchangeSettings =
        serde_json::from_value(back.get("mvr_xchange").cloned().unwrap()).unwrap();
    assert_eq!(stored, settings, "the show did not keep the settings it was given");
}

/// Where a client reaches this station's hosted group.
///
/// The station binds `0.0.0.0`, which is an address to listen on and not one to
/// connect to — so the port is taken from it and the host is loopback.
fn host_url(station: &Running) -> String {
    format!("ws://127.0.0.1:{}/mvrxchange", station.http_addr.port())
}

async fn state(station: &Running) -> XchangeState {
    let value = station
        .engine
        .get(vec![PathSegment::Key("xchange".into())])
        .await
        .expect("the xchange path exists");
    serde_json::from_value(value).expect("and holds an XchangeState")
}

/// Wait for something to become true of the exchange, or say what it was instead.
///
/// Twenty seconds is a ceiling and not an expectation — the whole file runs in about
/// ten. It is that high because this binary starts fourteen complete stations, two per
/// test and all of them in parallel, and on a loaded machine the settle after a
/// settings write is a broadcast, a flush and a socket rather than anything with a
/// bound. Every wait here is on an event that *will* arrive, so a tighter budget would
/// only be a way of failing on somebody else's build machine.
async fn until(
    station: &Running,
    what: &str,
    ready: impl Fn(&XchangeState) -> bool,
) -> XchangeState {
    for _ in 0..800 {
        let now = state(station).await;
        if ready(&now) {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let last = state(station).await;
    panic!("never {what}; the exchange was {last:?}");
}

/// A fixture in the rig, so a commit has something in it worth carrying.
///
/// A type as well, because an MVR carries a GDTF per type and a fixture with none is
/// not a thing this rig can be written out of.
async fn a_fixture(station: &Running, name: &str) -> uuid::Uuid {
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
    create(station, "fixture_types", serde_json::to_value(&fixture_type).unwrap()).await;

    let fixture = Fixture {
        id: uuid::Uuid::new_v4(),
        name: name.into(),
        fixture_type_id: fixture_type.id,
        address: FixtureAddress::dmx(1, 1),
        position: Some(Default::default()),
        ..Default::default()
    };
    let id = fixture.id;
    create(station, "fixtures", serde_json::to_value(&fixture).unwrap()).await;
    id
}

async fn create(station: &Running, collection: &str, value: serde_json::Value) {
    station
        .engine
        .set(
            vec![PathSegment::Key(collection.into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            value,
        )
        .await
        .unwrap_or_else(|e| panic!("creating a {collection}: {e:?}"));
}

async fn fixture_names(station: &Running) -> Vec<String> {
    let value =
        station.engine.get(vec![PathSegment::Key("fixtures".into())]).await.unwrap_or_default();
    let fixtures: Vec<serde_json::Value> = serde_json::from_value(value).unwrap_or_default();
    let mut names: Vec<String> = fixtures
        .iter()
        .filter_map(|f| f.get("name")?.as_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

// ── A group this console hosts ────────────────────────────────────────────────

/// The whole round trip, through a group one console hosts and another joins: a rig
/// committed on the host, announced across the WebSocket, fetched by the joiner, and
/// imported into its show.
#[tokio::test]
async fn a_rig_committed_on_one_console_lands_in_the_other() {
    let host = a_station("host").await;
    let joiner = a_station("joiner").await;

    exchanging(
        &host,
        XchangeSettings {
            enabled: true,
            group: "Round trip".into(),
            mode: XchangeMode::WebSocketHost,
            url: String::new(),
        },
    )
    .await;
    until(&host, "the host started hosting", |s| s.running).await;

    exchanging(
        &joiner,
        XchangeSettings {
            enabled: true,
            group: "Round trip".into(),
            mode: XchangeMode::WebSocket,
            url: host_url(&host),
        },
    )
    .await;

    // The joiner's MVR_JOIN is answered with the host's own row, which is how each
    // learns the other is there.
    until(&joiner, "the joiner found the host", |s| s.running && s.stations.len() > 1).await;
    until(&host, "the host saw the joiner", |s| s.stations.len() > 1).await;

    a_fixture(&host, "Downstage wash").await;
    let committed = host
        .xchange
        .ask(XchangeAsk::Commit {
            comment: "the first hang".into(),
            user_id: pult_schema::types::user::User::DEFAULT_ID,
        })
        .await
        .expect("the host commits");
    let file_uuid: uuid::Uuid =
        serde_json::from_value(committed["fileUuid"].clone()).expect("a file uuid");

    // Announced, not sent: the joiner knows of the file and has none of its bytes.
    let seen = until(&joiner, "the commit was announced", |s| {
        s.commits.iter().any(|c| c.file_uuid == file_uuid)
    })
    .await;
    let commit = seen.commits.iter().find(|c| c.file_uuid == file_uuid).unwrap();
    assert_eq!(commit.comment, "the first hang");
    assert!(!commit.ours, "it is the host's");
    assert!(!commit.here, "and its bytes have not been fetched");

    assert!(
        !fixture_names(&joiner).await.contains(&"Downstage wash".to_string()),
        "nothing is applied until somebody applies it",
    );

    joiner
        .xchange
        .ask(XchangeAsk::Apply { file_uuid, user_id: pult_schema::types::user::User::DEFAULT_ID })
        .await
        .expect("the joiner applies it");

    assert!(
        fixture_names(&joiner).await.contains(&"Downstage wash".to_string()),
        "the rig that left the host is the rig that arrived",
    );
}

/// A joiner's own commit reaches the host, which is the other direction through the
/// same route — and the host holds none of its bytes either.
#[tokio::test]
async fn a_commit_from_a_client_reaches_the_host() {
    let host = a_station("host2").await;
    let joiner = a_station("joiner2").await;

    exchanging(
        &host,
        XchangeSettings {
            enabled: true,
            group: "Upstream".into(),
            mode: XchangeMode::WebSocketHost,
            url: String::new(),
        },
    )
    .await;
    until(&host, "the host started hosting", |s| s.running).await;
    exchanging(
        &joiner,
        XchangeSettings {
            enabled: true,
            group: "Upstream".into(),
            mode: XchangeMode::WebSocket,
            url: host_url(&host),
        },
    )
    .await;
    until(&joiner, "the joiner found the host", |s| s.running && s.stations.len() > 1).await;

    a_fixture(&joiner, "Followspot").await;
    let committed = joiner
        .xchange
        .ask(XchangeAsk::Commit {
            comment: "added a follow".into(),
            user_id: pult_schema::types::user::User::DEFAULT_ID,
        })
        .await
        .expect("the joiner commits");
    let file_uuid: uuid::Uuid = serde_json::from_value(committed["fileUuid"].clone()).unwrap();

    let seen =
        until(&host, "the commit reached the host", |s| s.commits.iter().any(|c| c.file_uuid == file_uuid))
            .await;
    assert_eq!(seen.commits.iter().find(|c| c.file_uuid == file_uuid).unwrap().comment, "added a follow");

    host.xchange
        .ask(XchangeAsk::Apply { file_uuid, user_id: pult_schema::types::user::User::DEFAULT_ID })
        .await
        .expect("the host applies it");
    assert!(fixture_names(&host).await.contains(&"Followspot".to_string()));
}

// ── What is refused ───────────────────────────────────────────────────────────

/// A console with the exchange switched off is not on anybody's network, and says so
/// rather than failing silently.
#[tokio::test]
async fn a_show_that_has_not_switched_it_on_runs_nothing() {
    let station = a_station("off").await;
    let now = until(&station, "the station said what it was doing", |s| s.idle.is_some()).await;
    assert!(!now.running);
    assert_eq!(now.idle, Some(pult_schema::types::XchangeIdle::NotEnabled));

    let refused = station
        .xchange
        .ask(XchangeAsk::Commit {
            comment: "nobody will hear this".into(),
            user_id: pult_schema::types::user::User::DEFAULT_ID,
        })
        .await;
    assert!(refused.unwrap_err().contains("switched off"));
}

/// Switching it off again takes the console out of the group rather than leaving a
/// connection nobody is watching.
#[tokio::test]
async fn switching_it_off_stops_it() {
    let station = a_station("stops").await;
    exchanging(
        &station,
        XchangeSettings {
            enabled: true,
            group: "Briefly".into(),
            mode: XchangeMode::WebSocketHost,
            url: String::new(),
        },
    )
    .await;
    until(&station, "it started", |s| s.running).await;

    exchanging(&station, XchangeSettings { enabled: false, ..Default::default() }).await;
    let now = until(&station, "it stopped", |s| !s.running).await;
    assert_eq!(now.idle, Some(pult_schema::types::XchangeIdle::NotEnabled));
}

/// A URL nothing is listening on is a failure a person can read, not a console that
/// quietly does nothing.
#[tokio::test]
async fn a_host_that_is_not_there_is_reported() {
    let station = a_station("nowhere").await;
    exchanging(
        &station,
        XchangeSettings {
            enabled: true,
            group: "Missing".into(),
            mode: XchangeMode::WebSocket,
            // Port 1 on loopback: reserved, and nothing will ever answer on it.
            url: "ws://127.0.0.1:1/mvrxchange".into(),
        },
    )
    .await;
    let now = until(&station, "it gave up", |s| {
        matches!(s.idle, Some(pult_schema::types::XchangeIdle::Failed(_)))
    })
    .await;
    let Some(pult_schema::types::XchangeIdle::Failed(why)) = now.idle else { unreachable!() };
    assert!(why.contains("127.0.0.1:1"), "the message should name what could not be reached: {why}");
}

/// The route exists for the life of the process and answers only while the exchange
/// has claimed it — so a client arriving at a console that is not hosting is told so
/// rather than left holding an open socket to a group that does not exist.
#[tokio::test]
async fn a_console_that_is_not_hosting_refuses_a_client() {
    let quiet = a_station("quiet").await;
    let joiner = a_station("hopeful").await;

    exchanging(
        &joiner,
        XchangeSettings {
            enabled: true,
            group: "Nobody home".into(),
            mode: XchangeMode::WebSocket,
            url: host_url(&quiet),
        },
    )
    .await;

    let now = until(&joiner, "the joiner was turned away", |s| {
        matches!(s.idle, Some(pult_schema::types::XchangeIdle::Failed(_)))
    })
    .await;
    let Some(pult_schema::types::XchangeIdle::Failed(why)) = now.idle else { unreachable!() };
    assert!(why.contains("could not reach"), "the refusal should say so: {why}");
    assert!(!state(&quiet).await.running, "and the console it asked is still not hosting");
}

// ── The state, on every station ───────────────────────────────────────────────

/// Only the leader is on the wire, and every station holds the state — so an operator
/// at a follower can see the group their own console is in, and act on it.
#[tokio::test]
async fn a_follower_sees_the_exchange_the_leader_is_running() {
    let leader = a_station("leader").await;
    let follower = a_station("follower").await;

    follower
        .sync
        .connect_peer(vec![leader.sync_addr], uuid::Uuid::new_v4(), uuid::Uuid::nil())
        .await
        .expect("the two stations connect");
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if leader.sync.peer_count().await > 0 && follower.sync.peer_count().await > 0 {
            break;
        }
    }

    exchanging(
        &leader,
        XchangeSettings {
            enabled: true,
            group: "Shared".into(),
            mode: XchangeMode::WebSocketHost,
            url: String::new(),
        },
    )
    .await;
    until(&leader, "the leader started", |s| s.running).await;

    // The follower's own path fills in from the leader's push, and says which machine
    // is holding it — which is the whole reason `on_station` is in the state.
    let seen = until(&follower, "the state reached the follower", |s| {
        s.settings.group == "Shared"
    })
    .await;
    assert!(seen.running, "the state describes the station that is running it");
    assert!(!seen.on_station.is_empty());
}
