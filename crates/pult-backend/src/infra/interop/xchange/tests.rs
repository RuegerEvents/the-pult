//! Two consoles in a group, and the rules about who may do what.
//!
//! Everything here drives the manager through its own channel rather than through a
//! station, which is what lets a test say "a commit arrived from a previz" without a
//! previz. What a real station adds is in `tests/xchange.rs`.

use super::*;
use pult_mvr_xchange::message::Commit as WireCommit;
use pult_schema::events::operation::NodeId;

/// A manager with nothing behind it but an engine.
///
/// No sync handle, so nothing is published to peers; no transport, until a test says
/// so. `allowed` is the station veto, which several tests are about.
async fn a_manager(allowed: bool) -> (XchangeManager, XchangeHandle, EngineHandle) {
    let pool = std::sync::Arc::new(
        crate::infra::showfile::open_in_memory().await.expect("an in-memory showfile"),
    );
    let (engine_task, engine, _broadcast) =
        crate::engine::ShowEngine::new(NodeId(Uuid::new_v4()), pool.clone(), None);
    tokio::spawn(engine_task.run());
    let assets = crate::infra::assets::AssetStore::new(None, pool);
    let dir = std::env::temp_dir().join(format!("pult-xchange-test-{}", Uuid::new_v4()));
    let (manager, handle) = XchangeManager::new(
        "booth".into(),
        engine.clone(),
        assets,
        HostRegistry::default(),
        dir,
        XchangeLimits { allowed, keep: 3, max_file_bytes: 8 * 1024 * 1024 },
        crate::infra::net::Network::unconfigured(),
    );
    (manager, handle, engine)
}

fn a_wire_commit(file_uuid: Uuid, owner: Uuid, comment: &str) -> WireCommit {
    WireCommit {
        ver_major: 1,
        ver_minor: 6,
        file_size: 1234,
        file_uuid,
        station_uuid: owner,
        for_stations: Vec::new(),
        comment: comment.into(),
        file_name: "theirs.mvr".into(),
    }
}

/// Hand a message straight to the manager, as a connection would.
async fn incoming(manager: &mut XchangeManager, message: Message, from: Endpoint) -> Option<Reply> {
    manager.handle_incoming(message, from).await
}

#[tokio::test]
async fn a_station_that_is_vetoed_never_runs_however_the_show_is_set() {
    let (mut manager, _handle, _engine) = a_manager(false).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    manager.settings = XchangeSettings { enabled: true, ..Default::default() };

    let why = manager.why_not(&manager.settings.clone(), &manager.show.clone(), true);
    assert_eq!(why, XchangeIdle::RefusedByStation);
    assert!(!manager.should_run(&manager.settings.clone(), &manager.show.clone(), true));
}

#[tokio::test]
async fn a_follower_does_not_go_on_the_wire() {
    let (manager, _handle, _engine) = a_manager(true).await;
    let show = Some((Uuid::new_v4(), "Festival".into()));
    let settings = XchangeSettings { enabled: true, ..Default::default() };
    assert!(manager.should_run(&settings, &show, true), "the leader does");
    assert!(!manager.should_run(&settings, &show, false));
    assert_eq!(manager.why_not(&settings, &show, false), XchangeIdle::AnotherStationIsLeading);
}

/// The order matters: a console with no show open is not "not enabled", and telling
/// somebody to switch on a setting that does not exist yet is worse than saying
/// nothing.
#[tokio::test]
async fn the_most_specific_true_reason_is_the_one_given() {
    let (manager, _handle, _engine) = a_manager(true).await;
    let off = XchangeSettings::default();
    assert_eq!(manager.why_not(&off, &None, true), XchangeIdle::NoShow);
    let show = Some((Uuid::new_v4(), "Festival".into()));
    assert_eq!(manager.why_not(&off, &show, true), XchangeIdle::NotEnabled);
}

#[tokio::test]
async fn a_joining_station_is_remembered_and_answered_with_what_we_have() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));

    let theirs = Uuid::new_v4();
    let answer = incoming(
        &mut manager,
        Message::Join {
            provider: "Vectorworks".into(),
            ver_major: 1,
            ver_minor: 6,
            station_uuid: theirs,
            station_name: "Design laptop".into(),
            commits: vec![a_wire_commit(Uuid::from_u128(7), theirs, "first pass")],
        },
        Endpoint::WsClient(1),
    )
    .await;

    let Some(Reply::Message(Message::JoinRet { station_name, .. })) = answer else {
        panic!("a join was not answered");
    };
    assert_eq!(station_name, "Festival", "we answer with the show's name");
    assert_eq!(manager.stations[&theirs].provider, "Vectorworks");
    assert!(manager.stations[&theirs].joined);
    assert_eq!(manager.commits[&Uuid::from_u128(7)].comment, "first pass");
    assert!(!manager.commits[&Uuid::from_u128(7)].here, "we have the claim, not the bytes");
}

#[tokio::test]
async fn a_commit_addressed_to_other_stations_is_not_listed_here() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    let show_id = Uuid::new_v4();
    manager.show = Some((show_id, "Festival".into()));

    let mut commit = a_wire_commit(Uuid::from_u128(9), Uuid::new_v4(), "for previz only");
    commit.for_stations = vec![Uuid::from_u128(999)];
    incoming(&mut manager, Message::Commit(commit), Endpoint::WsClient(1)).await;

    assert!(manager.commits.is_empty(), "a commit for somebody else is not ours to show");
}

#[tokio::test]
async fn a_commit_for_everybody_is_listed() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    let theirs = Uuid::new_v4();
    incoming(
        &mut manager,
        Message::Commit(a_wire_commit(Uuid::from_u128(9), theirs, "moved the truss")),
        Endpoint::WsClient(1),
    )
    .await;
    assert_eq!(manager.commits[&Uuid::from_u128(9)].comment, "moved the truss");
    assert!(!manager.commits[&Uuid::from_u128(9)].ours);
}

/// This console announces what it *holds*, not what it once made. After a failover the
/// new leader has the show's commits and none of the old leader's files, and offering
/// one would be an offer that fails when somebody takes it up.
#[tokio::test]
async fn only_what_this_station_actually_has_is_offered() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    let show_id = Uuid::new_v4();
    manager.show = Some((show_id, "Festival".into()));
    let us = manager.station_uuid();

    // A commit of our own that this station does not hold — what a peer would have
    // told us about after taking over.
    manager.commits.insert(
        Uuid::from_u128(3),
        XchangeCommit {
            file_uuid: Uuid::from_u128(3),
            station_uuid: us,
            station_name: "Festival".into(),
            comment: "made on the other station".into(),
            file_name: "x.mvr".into(),
            file_size: 10,
            ours: true,
            here: false,
            at_ms: 1,
        },
    );

    assert!(manager.our_commits_on_the_wire().is_empty());

    manager
        .cache
        .put(
            &CachedCommit {
                file_uuid: Uuid::from_u128(4),
                file_size: 4,
                comment: "made here".into(),
                file_name: "y.mvr".into(),
                at_ms: 2,
            },
            b"here",
        )
        .unwrap();
    manager.load_our_commits();
    let offered = manager.our_commits_on_the_wire();
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0].file_uuid, Uuid::from_u128(4));
}

#[tokio::test]
async fn a_request_for_a_file_we_hold_is_answered_with_the_file() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    manager
        .cache
        .put(
            &CachedCommit {
                file_uuid: Uuid::from_u128(4),
                file_size: 4,
                comment: "made here".into(),
                file_name: "y.mvr".into(),
                at_ms: 2,
            },
            b"PK\x03\x04",
        )
        .unwrap();

    let answer = incoming(
        &mut manager,
        Message::Request { file_uuid: Some(Uuid::from_u128(4)), from_station: None },
        Endpoint::WsClient(1),
    )
    .await;
    assert!(matches!(answer, Some(Reply::File(bytes)) if bytes == b"PK\x03\x04"));
}

#[tokio::test]
async fn a_request_for_a_file_we_do_not_hold_is_refused_in_words() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    let answer = incoming(
        &mut manager,
        Message::Request { file_uuid: Some(Uuid::from_u128(4)), from_station: None },
        Endpoint::WsClient(1),
    )
    .await;
    let Some(Reply::Message(Message::RequestRet { response })) = answer else {
        panic!("a request was not refused properly");
    };
    assert!(!response.ok);
    assert!(response.message.contains("not available"));
}

/// A bare request means "your latest", which a station that has committed nothing
/// cannot answer — and says so rather than closing the connection.
#[tokio::test]
async fn a_bare_request_of_a_station_with_no_commits_is_refused() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    let answer =
        incoming(&mut manager, Message::Request { file_uuid: None, from_station: None }, Endpoint::WsHost)
            .await;
    let Some(Reply::Message(Message::RequestRet { response })) = answer else {
        panic!("not refused");
    };
    assert!(response.message.contains("committed nothing"));
}

/// Following a redirect from an unauthenticated station on the LAN is a person's
/// decision, and answering `OK: true` would claim we had already moved.
#[tokio::test]
async fn a_move_is_held_for_an_operator_rather_than_followed() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));

    let answer = incoming(
        &mut manager,
        Message::NewSessionHost {
            service_name: String::new(),
            service_url: "ws://previz.local:8080/mvrxchange".into(),
        },
        Endpoint::WsClient(1),
    )
    .await;

    let Some(Reply::Message(Message::NewSessionHostRet { response })) = answer else {
        panic!("not answered");
    };
    assert!(!response.ok, "saying yes would claim this console had moved");
    assert_eq!(
        manager.pending_host.as_ref().unwrap().service_url,
        "ws://previz.local:8080/mvrxchange"
    );
    assert_eq!(manager.settings.mode, XchangeMode::Tcp, "and nothing has moved yet");
}

#[tokio::test]
async fn a_move_naming_both_a_service_and_a_url_is_refused() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    let answer = incoming(
        &mut manager,
        Message::NewSessionHost {
            service_name: "Other._mvrxchange._tcp.local.".into(),
            service_url: "ws://previz.local/mvrxchange".into(),
        },
        Endpoint::WsClient(1),
    )
    .await;
    let Some(Reply::Message(Message::NewSessionHostRet { response })) = answer else {
        panic!("not answered");
    };
    assert!(!response.ok);
    assert!(manager.pending_host.is_none(), "an illegal move is not held for anybody");
}

/// Declining leaves the show exactly where it was, and clears the ask so the panel
/// stops offering it.
#[tokio::test]
async fn declining_a_move_leaves_the_group_alone() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    manager.transport = Some(Transport::Hosting);
    manager.pending_host = Some(PendingHost {
        from_station: Uuid::new_v4(),
        from_name: "Booth MA3".into(),
        service_name: "Other._mvrxchange._tcp.local.".into(),
        service_url: String::new(),
    });

    let (tx, rx) = oneshot::channel();
    manager.follow_host(false, Some(tx)).await;
    assert!(rx.await.unwrap().is_ok());
    assert!(manager.pending_host.is_none());
    assert_eq!(manager.settings.group, "Default");
}

/// A commit with no comment is one nobody can tell from the last, which is the whole
/// use a commit list is put to.
#[tokio::test]
async fn a_commit_needs_something_written_on_it() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    manager.transport = Some(Transport::Hosting);

    let (tx, rx) = oneshot::channel();
    manager.handle_ask(
        XchangeAsk::Commit { comment: "   ".into(), user_id: Uuid::nil() },
        Some(tx),
    )
    .await;
    let outcome = rx.await.unwrap();
    assert!(outcome.unwrap_err().contains("needs a comment"));
}

#[tokio::test]
async fn an_ask_on_a_station_that_is_not_running_says_why() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.idle = Some(XchangeIdle::NotEnabled);

    let (tx, rx) = oneshot::channel();
    manager.handle_ask(XchangeAsk::FollowHost { follow: true }, Some(tx)).await;
    assert!(rx.await.unwrap().unwrap_err().contains("switched off"));
}

#[tokio::test]
async fn applying_a_commit_nobody_announced_is_refused() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    manager.transport = Some(Transport::Hosting);

    let (tx, rx) = oneshot::channel();
    manager.apply(Uuid::from_u128(42), Uuid::nil(), Some(tx)).await;
    assert!(rx.await.unwrap().unwrap_err().contains("no commit by that id"));
}

/// A second fetch would have no way of telling which answer was whose: `MVR_REQUEST`
/// carries no correlation id at all.
#[tokio::test]
async fn only_one_file_is_fetched_at_a_time() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    manager.transport = Some(Transport::Hosting);
    manager.awaiting = Some(Awaiting::Relay { to: Endpoint::WsClient(1) });

    let theirs = Uuid::new_v4();
    manager.stations.insert(
        theirs,
        Known {
            name: "Design laptop".into(),
            provider: String::new(),
            endpoint: Endpoint::WsClient(2),
            fullname: None,
            joined: true,
            in_group: true,
            dialable: true,
        },
    );
    manager.note_commit(a_wire_commit(Uuid::from_u128(5), theirs, "theirs"), theirs);

    let (tx, rx) = oneshot::channel();
    manager.apply(Uuid::from_u128(5), Uuid::nil(), Some(tx)).await;
    assert!(rx.await.unwrap().unwrap_err().contains("already being fetched"));
}

/// A refusal has to release whatever is waiting, or the next apply is told a fetch is
/// in flight for ever.
#[tokio::test]
async fn a_refused_fetch_releases_the_caller() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    let (tx, rx) = oneshot::channel();
    manager.awaiting = Some(Awaiting::Ours {
        file_uuid: Uuid::from_u128(1),
        user_id: Uuid::nil(),
        from: Endpoint::WsHost,
        reply: Some(tx),
    });

    incoming(
        &mut manager,
        Message::RequestRet { response: Response::failed("the MVR is not available") },
        Endpoint::WsHost,
    )
    .await;

    assert!(rx.await.unwrap().unwrap_err().contains("not available"));
    assert!(manager.awaiting.is_none());
}

/// The panel shows this console's own row, because a group includes us — and an
/// operator who cannot see what their station is telling the room cannot check it.
#[tokio::test]
async fn the_state_carries_this_console_s_own_row_when_it_is_running() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    manager.transport = Some(Transport::Hosting);

    let state = manager.state();
    assert!(state.running);
    assert_eq!(state.stations.len(), 1);
    assert!(state.stations[0].is_us);
    assert_eq!(state.stations[0].station_name, "Festival");
    assert_eq!(state.station_uuid, pult_schema::types::xchange::station_uuid_for(manager.show.as_ref().unwrap().0));
}

#[tokio::test]
async fn a_station_that_is_not_running_carries_no_rows_and_a_reason() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.idle = Some(XchangeIdle::NoShow);
    let state = manager.state();
    assert!(!state.running);
    assert!(state.stations.is_empty());
    assert_eq!(state.idle, Some(XchangeIdle::NoShow));
    assert_eq!(state.on_station, "booth");
}

/// Newest first, because a commit list is read from the top and the one somebody
/// wants is almost always the one that just arrived.
#[tokio::test]
async fn commits_are_listed_newest_first() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    let theirs = Uuid::new_v4();
    for (n, at) in [(1u128, 100i64), (2, 300), (3, 200)] {
        let mut commit = XchangeCommit {
            file_uuid: Uuid::from_u128(n),
            station_uuid: theirs,
            station_name: String::new(),
            comment: String::new(),
            file_name: String::new(),
            file_size: 0,
            ours: false,
            here: false,
            at_ms: at,
        };
        commit.at_ms = at;
        manager.commits.insert(commit.file_uuid, commit);
    }
    let listed: Vec<u128> =
        manager.state().commits.iter().map(|c| c.file_uuid.as_u128()).collect();
    assert_eq!(listed, vec![2, 3, 1]);
}

/// Our own row is what the cache says, and a copy of it coming back round from a host
/// must not make this station claim a file it has since pruned.
#[tokio::test]
async fn our_own_commit_coming_back_from_a_host_does_not_overwrite_what_we_hold() {
    let (mut manager, _handle, _engine) = a_manager(true).await;
    manager.show = Some((Uuid::new_v4(), "Festival".into()));
    let us = manager.station_uuid();
    manager
        .cache
        .put(
            &CachedCommit {
                file_uuid: Uuid::from_u128(4),
                file_size: 4,
                comment: "ours".into(),
                file_name: "y.mvr".into(),
                at_ms: 2,
            },
            b"here",
        )
        .unwrap();
    manager.load_our_commits();

    manager.note_commit(a_wire_commit(Uuid::from_u128(4), us, "a relayed copy"), us);
    assert!(manager.commits[&Uuid::from_u128(4)].here, "still ours, still here");
    assert_eq!(manager.commits[&Uuid::from_u128(4)].comment, "ours");
}

// ── TCP mode, over a real socket ──────────────────────────────────────────────

/// Two managers in TCP mode, joined, committing and fetching over loopback.
///
/// Discovery is handed in rather than waited for: mDNS on a build machine says more
/// about the machine than about this code, and what it *would* have found is exactly a
/// [`Discovered`] with an address in it. Everything past that point — the join, the
/// commit announcement, the request, and the file coming back through the protocol's
/// own framing — is real.
#[tokio::test]
async fn two_consoles_in_tcp_mode_exchange_a_rig() {
    let a = a_running_station("A").await;
    let b = a_running_station("B").await;

    // What mDNS would have said about B, told to A.
    a.handle
        .0
        .send(XchangeCommand::Discovered(Discovered {
            station_uuid: b.station_uuid,
            station_name: "B".into(),
            fullname: "B.Test._mvrxchange._tcp.local.".into(),
            addr: format!("127.0.0.1:{}", b.port).parse().unwrap(),
            in_group: true,
        }))
        .await
        .unwrap();

    // And what it would have said about A, told to B: in TCP mode both stations
    // advertise and both browse, so each learns the other's *listening* port. Nothing
    // else ever says where a TCP station can be reached.
    b.handle
        .0
        .send(XchangeCommand::Discovered(Discovered {
            station_uuid: a.station_uuid,
            station_name: "A".into(),
            fullname: "A.Test._mvrxchange._tcp.local.".into(),
            addr: format!("127.0.0.1:{}", a.port).parse().unwrap(),
            in_group: true,
        }))
        .await
        .unwrap();

    // Each joins the other, and each answers — so each ends up knowing the other.
    settles(&a.engine, "A knows B", |s| s.stations.iter().any(|k| k.station_uuid == b.station_uuid))
        .await;
    settles(&b.engine, "B knows A", |s| s.stations.iter().any(|k| k.station_uuid == a.station_uuid))
        .await;

    // Something worth carrying.
    let fixture_type = pult_schema::types::FixtureType {
        id: Uuid::new_v4(),
        name: "Wash".into(),
        manufacturer: "Acme".into(),
        channel_count: 1,
        parameters: vec![pult_schema::types::fixture::ParameterDefinition::new(
            pult_schema::types::fixture::ParameterKind::Intensity,
            pult_schema::types::fixture::ParameterValue::Float(0.0),
        )],
        ..Default::default()
    };
    b.engine
        .set(
            vec![PathSegment::Key("fixture_types".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            serde_json::to_value(&fixture_type).unwrap(),
        )
        .await
        .expect("a type");
    b.engine
        .set(
            vec![PathSegment::Key("fixtures".into()), PathSegment::Key("__create".into())],
            Lifecycle::Persisted,
            serde_json::to_value(&pult_schema::types::Fixture {
                id: Uuid::new_v4(),
                name: "Overhead wash".into(),
                fixture_type_id: fixture_type.id,
                address: pult_schema::types::fixture::FixtureAddress::dmx(1, 1),
                position: Some(Default::default()),
                ..Default::default()
            })
            .unwrap(),
        )
        .await
        .expect("a fixture");

    // B commits; A hears about it over the connection B dials.
    let committed = b
        .handle
        .ask(XchangeAsk::Commit {
            comment: "hung the overhead".into(),
            user_id: pult_schema::types::user::User::DEFAULT_ID,
        })
        .await
        .expect("B commits");
    let file_uuid: uuid::Uuid = serde_json::from_value(committed["fileUuid"].clone()).unwrap();

    let seen = settles(&a.engine, "A heard the commit", |s| {
        s.commits.iter().any(|c| c.file_uuid == file_uuid)
    })
    .await;
    let commit = seen.commits.iter().find(|c| c.file_uuid == file_uuid).unwrap();
    assert_eq!(commit.comment, "hung the overhead");
    assert!(!commit.here, "announced, not sent");

    // And A fetches it: a real MVR_REQUEST, answered with a real archive.
    a.handle
        .ask(XchangeAsk::Apply {
            file_uuid,
            user_id: pult_schema::types::user::User::DEFAULT_ID,
        })
        .await
        .expect("A applies B's commit");

    // The rig B wrote is the rig A now has.
    let fixtures = a
        .engine
        .get(vec![PathSegment::Key("fixtures".into())])
        .await
        .expect("A has a fixtures collection");
    let fixtures: Vec<serde_json::Value> = serde_json::from_value(fixtures).unwrap_or_default();
    assert!(
        fixtures.iter().any(|f| f.get("name").and_then(|n| n.as_str()) == Some("Overhead wash")),
        "the fixture B committed is not in A's rig: {fixtures:?}",
    );
}

/// A station that is running, with its own engine, show and TCP transport.
struct Started {
    handle: XchangeHandle,
    engine: EngineHandle,
    station_uuid: Uuid,
    port: u16,
}

async fn a_running_station(name: &str) -> Started {
    let pool = std::sync::Arc::new(
        crate::infra::showfile::open_in_memory().await.expect("an in-memory showfile"),
    );
    let (engine_task, engine, _broadcast) =
        crate::engine::ShowEngine::new(NodeId(Uuid::new_v4()), pool.clone(), None);
    tokio::spawn(engine_task.run());
    // A real directory: an imported MVR carries GDTF archives and meshes, and a store
    // with nowhere to put them refuses the write — which is right for a console with
    // no show open and wrong for a station that has one.
    let asset_dir = std::env::temp_dir().join(format!("pult-xchange-assets-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&asset_dir).unwrap();
    let assets = crate::infra::assets::AssetStore::new(Some(asset_dir), pool);

    let show_id = Uuid::new_v4();
    let show = serde_json::json!({
        "id": show_id,
        "name": name,
        "created_at": chrono::Utc::now(),
        "history_depth": 500,
        "home_fade_ms": 0,
        "haze_density": 1.0,
        "haze_turbulence": 0.5,
        "fade_curves": {},
        "production": {},
        // A group of this test's own, so two runs on one machine never meet.
        "mvr_xchange": { "enabled": true, "group": format!("pult-test-{}", Uuid::new_v4()), "mode": "Tcp", "url": "" },
    });
    engine
        .set(vec![PathSegment::Key("show".into())], Lifecycle::Persisted, show)
        .await
        .expect("a show");

    let dir = std::env::temp_dir().join(format!("pult-xchange-test-{}", Uuid::new_v4()));
    let (mut manager, handle) = XchangeManager::new(
        name.into(),
        engine.clone(),
        assets,
        HostRegistry::default(),
        dir,
        XchangeLimits { allowed: true, keep: 3, max_file_bytes: 8 * 1024 * 1024 },
        crate::infra::net::Network::unconfigured(),
    );
    // Started by hand rather than by `run`, so the port is known before anything is
    // told where to find it.
    manager.reconsider().await;
    let port = match &manager.transport {
        Some(Transport::Tcp(tcp)) => tcp.port,
        _ => panic!("{name} did not come up in TCP mode"),
    };
    let station_uuid = manager.station_uuid();
    tokio::spawn(async move {
        while let Some(cmd) = manager.rx.recv().await {
            match cmd {
                XchangeCommand::Stop => break,
                XchangeCommand::Reconsider => manager.reconsider().await,
                XchangeCommand::Ask { ask, reply } => manager.handle_ask(ask, reply).await,
                XchangeCommand::Incoming { message, from, reply } => {
                    let answer = manager.handle_incoming(message, from).await;
                    let _ = reply.send(answer);
                    manager.publish().await;
                }
                XchangeCommand::FileArrived { from, bytes } => manager.handle_file(from, bytes).await,
                XchangeCommand::Discovered(found) => manager.handle_discovered(found).await,
                _ => {}
            }
        }
    });

    Started { handle, engine, station_uuid, port }
}

/// Wait for the published state to say something, or fail saying what it said instead.
async fn settles(
    engine: &EngineHandle,
    what: &str,
    ready: impl Fn(&XchangeState) -> bool,
) -> XchangeState {
    for _ in 0..400 {
        let value = engine.get(vec![PathSegment::Key("xchange".into())]).await.unwrap_or_default();
        if let Ok(state) = serde_json::from_value::<XchangeState>(value) {
            if ready(&state) {
                return state;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("never {what}");
}
