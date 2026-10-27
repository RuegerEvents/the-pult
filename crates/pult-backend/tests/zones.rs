//! A speaker zone across a failover: two stations whose sound cards are wired
//! differently, and the leader killed while a clip routed to the zone is playing.
//!
//! Its own binary rather than a test beside `failover.rs`'s. Two stations in one process
//! already share one show clock (`pult_schema::clock` is per process); beside this one,
//! the maze test there failed its 20 s wait for the spare to hold the maze twice in a row,
//! and passes alone. Two pairs killing their leaders in one process are not a thing a
//! venue does, so they are not made to share one here.

use std::time::Duration;

use pult_backend::{Config, Running};
use pult_schema::{path::PathSegment, types::timeline::Timeline};
use uuid::Uuid;

/// Preferences of this binary's own: no sound card, so no test opens the machine's
/// speakers or its microphone, and no recent list that is the operator's.
fn own_preferences() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("pult-zones-prefs-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("a preferences directory");
        let path = dir.join("preferences.toml");
        std::fs::write(
            &path,
            "[audio]\ninput = \"no input device in tests\"\noutput = \"no output device in tests\"\n\n\
             [audio.names.\"no output device in tests\"]\nalias = \"Garden Dante\"\n\
             channels = [\"FOH L\", \"FOH R\", \"Area 2\"]\n",
        )
        .expect("a preferences file");
        // SAFETY: inside `call_once`, before any station in this process has started.
        unsafe { std::env::set_var("PULT_PREFERENCES", &path) };
    });
}

fn a_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn a_station(name: &str) -> Running {
    own_preferences();
    let show = std::env::temp_dir().join(format!("pult-zones-{name}-{}.pult", Uuid::new_v4()));
    pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        show: Some(show.clone()),
        identity: Some(show.with_extension("node")),
        openhaunt_broker_port: a_free_port(),
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

/// A clip goes to a speaker zone, and a zone is whichever channels the station playing
/// it maps it to. The spare's card is wired differently from the leader's — the zone is
/// channel 3 on one and channel 5 on the other — and when the leader dies mid-clip the
/// spare plays the zone on *its* channel 5, never on the dead machine's 3.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spare_plays_a_zone_on_its_own_channels() {
    use pult_schema::{
        lifecycle::Lifecycle,
        types::{
            timeline::{AudioClip, AudioDestination, AudioStatuses, TimelineSource},
            SpeakerZone,
        },
    };

    let leader = a_station("leader").await;
    let spare = a_station("spare").await;
    let at = format!("127.0.0.1:{}", leader.sync_addr.port());
    spare.call_rpc("session.joinAt", serde_json::json!({ "address": at })).await.expect("the spare joins");
    eventually("the spare is a follower", Duration::from_secs(10), || async {
        is_follower(&spare).await && !is_follower(&leader).await
    })
    .await;

    let create = |station: &Running, table: &str, value: serde_json::Value| {
        let engine = station.engine.clone();
        let table = table.to_string();
        async move {
            engine
                .set(
                    vec![PathSegment::Key(table.clone()), PathSegment::Key("__create".into())],
                    Lifecycle::Persisted,
                    value,
                )
                .await
                .unwrap_or_else(|e| panic!("{table} is created: {e}"));
        }
    };
    let zone = SpeakerZone {
        folder: None,
        id: Uuid::new_v4(),
        name: "Area 2".into(),
        color: None,
        width: 1,
        channels: [(leader.node_id, vec![3]), (spare.node_id, vec![5])].into_iter().collect(),
        nodes: Vec::new(),
        notes: String::new(),
    };
    create(&leader, "speaker_zones", serde_json::to_value(&zone).unwrap()).await;
    let clip = Uuid::new_v4();
    let timeline = Timeline {
        folder: None,
        id: Uuid::new_v4(),
        name: "Area 2 scare".into(),
        clips: vec![AudioClip {
            id: clip,
            name: "Skeleton laugh".into(),
            // A file nobody has: what is asserted is where it would come out, which is
            // worked out before a byte of it is decoded.
            asset: "00".into(),
            peaks: None,
            start_ms: 0,
            offset_ms: 0,
            length_ms: None,
            gain_db: 0.0,
            fade_in_ms: 0,
            fade_out_ms: 0,
            repeats: 1,
            destination: AudioDestination::Zone { zone: zone.id },
        }],
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
        running: true,
        anchor_ms: pult_schema::clock::now_ms(),
        position_at_anchor_ms: 0,
        rate: 1.0,
        recording: None,
        armed_by: None,
    };
    create(&leader, "timelines", serde_json::to_value(&timeline).unwrap()).await;

    let channels_on = |station: &Running| {
        let engine = station.engine.clone();
        let (timeline, clip) = (timeline.id.to_string(), clip.to_string());
        async move {
            let statuses: AudioStatuses = engine
                .get(vec![PathSegment::Key("audio_status".into())])
                .await
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            statuses.get(&timeline).and_then(|s| s.clip_channels.get(&clip).cloned())
        }
    };
    eventually("the leader plays the zone on its channel 3", Duration::from_secs(10), || async {
        channels_on(&leader).await == Some(vec![3])
    })
    .await;
    eventually("the spare has the zone and the clip", Duration::from_secs(10), || async {
        !read::<SpeakerZone>(&spare, "speaker_zones").await.is_empty()
            && read::<Timeline>(&spare, "timelines").await.iter().any(|t| t.running)
    })
    .await;
    assert_eq!(channels_on(&spare).await, None, "a follower plays nothing");

    leader.kill().await;
    eventually("the spare takes over", Duration::from_secs(20), || async { !is_follower(&spare).await }).await;
    eventually("the spare plays the zone on its own channel 5", Duration::from_secs(10), || async {
        channels_on(&spare).await == Some(vec![5])
    })
    .await;

    spare.shutdown("the test is over").await;
}

/// The booth maps the spare's zones: every station publishes what it plays through and
/// what it calls the channels, so another station's matrix can show them by name.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_station_says_what_its_channels_are_called() {
    use pult_schema::types::StationAudio;

    let leader = a_station("named-leader").await;
    let spare = a_station("named-spare").await;
    // The spare has written its own rows before it joins — the ordinary case for a
    // spare switched on before anybody sessions it — and they still have to arrive.
    let spare_id = spare.node_id.0;
    eventually("the spare has published its card to itself", Duration::from_secs(10), || async {
        let rows: Vec<StationAudio> = read(&spare, "station_audio").await;
        rows.iter().any(|row| row.id == spare_id)
    })
    .await;
    let at = format!("127.0.0.1:{}", leader.sync_addr.port());
    spare.call_rpc("session.joinAt", serde_json::json!({ "address": at })).await.expect("the spare joins");
    eventually("the spare is a follower", Duration::from_secs(10), || async {
        is_follower(&spare).await && !is_follower(&leader).await
    })
    .await;

    eventually("the leader reads the spare's card and its names", Duration::from_secs(15), || async {
        let rows: Vec<StationAudio> = read(&leader, "station_audio").await;
        rows.iter().any(|row| {
            row.id == spare_id
                && row.alias.as_deref() == Some("Garden Dante")
                && row.channel_names == ["FOH L", "FOH R", "Area 2"]
                && row.device.as_deref() == Some("no output device in tests")
        })
    })
    .await;
}

