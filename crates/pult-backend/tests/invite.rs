//! One station inviting another into its session, over the invited station's own
//! socket — the way a Pi in a rack is brought in from the desk.
//!
//! The guest is put in the host's list of stations on the network by hand rather than
//! found: discovery is mDNS, which a test machine may not carry, and what is under test
//! here is the invitation, not the finding.

use std::time::Duration;

use pult_backend::{api::rpcs, Config, Running};
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::session::{NearbyStation, SessionState},
};

async fn a_station(name: &str) -> Running {
    let show = std::env::temp_dir().join(format!("pult-invite-{name}-{}.pult", uuid::Uuid::new_v4()));
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

fn path(segments: &[&str]) -> Vec<PathSegment> {
    segments.iter().map(|s| PathSegment::Key(s.to_string())).collect()
}

async fn session(station: &Running) -> SessionState {
    serde_json::from_value(station.engine.get(path(&["session"])).await.unwrap()).unwrap()
}

async fn show_id(station: &Running) -> serde_json::Value {
    station.engine.get(path(&["show"])).await.unwrap()["id"].clone()
}

/// Wait for something that happens on its own time, or say what never did.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invited_station_joins_and_takes_the_hosts_show() {
    let host = a_station("host").await;
    let guest = a_station("guest").await;
    assert_ne!(show_id(&host).await, show_id(&guest).await, "two shows to begin with");

    // The host has to have said where it syncs before it can tell anybody.
    eventually("the host publishing its row", || async {
        host.engine
            .get(vec![PathSegment::Key("stations".into()), PathSegment::Id(host.node_id.0)])
            .await
            .is_ok()
    })
    .await;

    let mut state = session(&host).await;
    state.nearby.push(NearbyStation {
        node_id: guest.node_id,
        hostname: "guest".into(),
        http_addr: format!("127.0.0.1:{}", guest.http_addr.port()),
        sync_addr: format!("127.0.0.1:{}", guest.sync_addr.port()),
        follows: false,
        show_name: "Guest".into(),
        session_id: None,
    });
    host.engine
        .set(path(&["session"]), Lifecycle::Local, serde_json::to_value(&state).unwrap())
        .await
        .unwrap();

    rpcs::dispatch("session.invite", serde_json::json!({ "nodeId": guest.node_id.0 }), &host.rpc)
        .await
        .expect("the invitation is accepted");

    assert!(session(&host).await.is_advertising, "a lone host starts a session to invite into");
    assert!(session(&guest).await.is_follower, "the guest follows the host");
    let host_show = show_id(&host).await;
    eventually("the guest taking the host's show", || {
        let want = host_show.clone();
        let guest = &guest;
        async move { show_id(guest).await == want }
    })
    .await;
}

/// An invitation goes to a station this one can see, and to nobody else.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_station_nobody_can_see_cannot_be_invited() {
    let host = a_station("alone").await;
    let why = rpcs::dispatch(
        "session.invite",
        serde_json::json!({ "nodeId": uuid::Uuid::new_v4() }),
        &host.rpc,
    )
    .await
    .expect_err("there is nobody to invite");
    assert!(why.contains("not on the network"), "{why}");
}
