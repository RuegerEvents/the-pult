//! Device manager tests.
//!
//! Discovery is injected, so nothing here touches multicast. Where a test needs a
//! node to answer HTTP, it starts a small axum server on an ephemeral port and
//! records what arrived — which is also how the mains flag and Identify are checked
//! without hardware.

use std::sync::{Arc, Mutex};

use axum::{routing::{get, post}, Json, Router};
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        devices::DevicesState,
        fixture::{Fixture, FixtureAddress, FixtureType},
        openhaunt,
    },
};

use super::*;
use crate::{engine::ShowEngine, infra::showfile};

// ── Harness ───────────────────────────────────────────────────────────────────

struct Harness {
    engine: EngineHandle,
    devices: DeviceHandle,
    directory: watch::Receiver<DeviceDirectory>,
}

async fn harness() -> Harness {
    let pool = Arc::new(showfile::open_in_memory().await.expect("open in-memory showfile"));
    let (engine, engine_handle, _broadcast) =
        ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
    tokio::spawn(engine.run());

    let (manager, devices, directory) = DeviceManager::new(engine_handle.clone());
    tokio::spawn(manager.run());

    Harness { engine: engine_handle, devices, directory }
}

impl Harness {
    async fn resolve(&self, serial: &str, module_type: u16, addr: Option<std::net::SocketAddr>) {
        let mut txt = BTreeMap::new();
        txt.insert("sn".to_string(), serial.to_string());
        txt.insert("mod".to_string(), format!("{module_type:#06x}"));
        txt.insert("name".to_string(), format!("Node {serial}"));
        txt.insert("fw".to_string(), "0.1.0".to_string());
        txt.insert("v".to_string(), "1".to_string());
        txt.insert("caps".to_string(), "dmx,rdm".to_string());

        // No stub node: point at a closed local port, so the manager's `/info` call
        // is refused at once rather than sitting out its timeout.
        let (ip, port) = match addr {
            Some(addr) => (addr.ip().to_string(), addr.port()),
            None => ("127.0.0.1".to_string(), 1),
        };
        self.event(DeviceEvent::Resolved {
            serial: serial.to_string(),
            ip,
            port,
            host: format!("openhaunt-{serial}.local."),
            txt,
        })
        .await;
    }

    /// Send an event and wait for the manager to have finished with it. Every
    /// command is answered in order, so a round trip through Identify orders behind.
    async fn event(&self, event: DeviceEvent) {
        self.devices.0.send(DeviceCommand::Event(event)).await.unwrap();
        let _ = self.devices.identify("wait-for-the-queue-to-drain".into()).await;
    }

    async fn state(&self) -> DevicesState {
        let value = self
            .engine
            .get(vec![PathSegment::Key("devices".into())])
            .await
            .expect("devices is a LOCAL path and always answers");
        serde_json::from_value(value).expect("devices state round-trips")
    }

    async fn fixtures(&self) -> Vec<Fixture> {
        let value = self.engine.get(vec![PathSegment::Key("fixtures".into())]).await.unwrap();
        serde_json::from_value(value).unwrap()
    }

    async fn fixture_types(&self) -> Vec<FixtureType> {
        let value = self.engine.get(vec![PathSegment::Key("fixture_types".into())]).await.unwrap();
        serde_json::from_value(value).unwrap()
    }

    async fn become_follower(&self) {
        self.engine
            .set(
                vec![PathSegment::Key("session".into())],
                Lifecycle::Local,
                serde_json::json!({
                    "is_advertising": false, "is_follower": true,
                    "session_id": Uuid::new_v4(), "discovered": [],
                }),
            )
            .await
            .unwrap();
    }
}

// ── A node that answers ───────────────────────────────────────────────────────

#[derive(Default)]
struct NodeLog {
    identified: usize,
    outputs: Vec<serde_json::Value>,
}

/// A stand-in for a node's HTTP API, on an ephemeral port, recording what it was
/// asked to do.
async fn a_node(module_flags: u32) -> (std::net::SocketAddr, Arc<Mutex<NodeLog>>) {
    let log = Arc::new(Mutex::new(NodeLog::default()));

    let info_log = log.clone();
    let identify_log = log.clone();
    let state_log = log.clone();
    let app = Router::new()
        .route(
            "/api/v1/info",
            get(move || {
                let _ = &info_log;
                async move { Json(serde_json::json!({ "module": { "flags": module_flags } })) }
            }),
        )
        .route(
            "/api/v1/identify",
            post(move || {
                let log = identify_log.clone();
                async move {
                    log.lock().unwrap().identified += 1;
                    Json(serde_json::json!({ "ok": true }))
                }
            }),
        )
        .route(
            "/api/v1/state",
            post(move |Json(body): Json<serde_json::Value>| {
                let log = state_log.clone();
                async move {
                    log.lock().unwrap().outputs.push(body);
                    Json(serde_json::json!({ "ok": true }))
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, log)
}

// ── Discovery ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_resolved_node_shows_up_with_what_its_txt_record_said() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;

    let state = h.state().await;
    let device = state.discovered.get("1a2b3c").expect("the device is listed");
    assert_eq!(device.name, "Node 1a2b3c");
    assert_eq!(device.module_type, openhaunt::MODULE_TYPE_DIGITAL_IN);
    assert_eq!(device.module_name, "Digital Inputs", "the module name fills in from the id");
    assert_eq!(device.caps, vec!["dmx", "rdm"]);
    assert!(device.online);
    assert_eq!(device.adopted_fixture_id, None);
}

#[tokio::test]
async fn resolving_the_same_node_twice_updates_it_rather_than_duplicating_it() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_MAINS_RELAY, None).await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_MAINS_RELAY, None).await;

    assert_eq!(h.state().await.discovered.len(), 1);
}

#[tokio::test]
async fn a_relay_module_is_flagged_as_mains_before_anyone_is_asked() {
    let h = harness().await;
    h.resolve("4d5e6f", openhaunt::MODULE_TYPE_MAINS_RELAY, None).await;

    assert!(
        h.state().await.discovered["4d5e6f"].is_mains,
        "the warning has to be up before an HTTP round trip, not after",
    );
}

#[tokio::test]
async fn a_node_that_says_it_switches_mains_is_believed_over_its_module_id() {
    let (addr, _log) = a_node(openhaunt::MODULE_FLAG_MAINS).await;
    let h = harness().await;
    // A dry-contact module, which this console would not otherwise warn about.
    h.resolve("4d5e6f", openhaunt::MODULE_TYPE_DRY_CONTACT, Some(addr)).await;

    assert!(h.state().await.discovered["4d5e6f"].is_mains);
}

#[tokio::test]
async fn an_unadopted_node_that_goes_quiet_is_forgotten() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;
    h.event(DeviceEvent::Removed { serial: "1a2b3c".into() }).await;

    assert!(h.state().await.discovered.is_empty());
}

#[tokio::test]
async fn an_adopted_node_that_goes_quiet_stays_listed_as_offline() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;
    h.devices.adopt("1a2b3c".into()).await.unwrap();

    h.event(DeviceEvent::Removed { serial: "1a2b3c".into() }).await;

    let state = h.state().await;
    let device = state.discovered.get("1a2b3c").expect("an adopted device stays listed");
    assert!(!device.online);
    assert!(device.adopted_fixture_id.is_some(), "its fixture is still patched");
    assert_eq!(h.fixtures().await.len(), 1);
}

#[tokio::test]
async fn a_node_that_comes_back_keeps_the_fixture_it_was_adopted_as() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;
    let fixture_id = h.devices.adopt("1a2b3c".into()).await.unwrap();

    h.event(DeviceEvent::Removed { serial: "1a2b3c".into() }).await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;

    let state = h.state().await;
    assert!(state.discovered["1a2b3c"].online);
    assert_eq!(state.discovered["1a2b3c"].adopted_fixture_id, Some(fixture_id));
}

// ── Adoption ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn adopting_a_node_patches_it_as_a_fixture_of_its_module_s_type() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;

    let fixture_id = h.devices.adopt("1a2b3c".into()).await.unwrap();

    let fixtures = h.fixtures().await;
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].id, fixture_id);
    assert_eq!(
        fixtures[0].address,
        FixtureAddress::OpenHaunt { serial: "1a2b3c".into(), universe: None },
        "only a gateway carries a universe",
    );

    let types = h.fixture_types().await;
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].id, openhaunt::builtin_fixture_type_id(openhaunt::MODULE_TYPE_DIGITAL_IN));
    assert_eq!(types[0].parameters.len(), 8);
}

#[tokio::test]
async fn adopting_two_of_the_same_module_makes_one_fixture_type() {
    let h = harness().await;
    h.resolve("aaa", openhaunt::MODULE_TYPE_MAINS_RELAY, None).await;
    h.resolve("bbb", openhaunt::MODULE_TYPE_MAINS_RELAY, None).await;

    h.devices.adopt("aaa".into()).await.unwrap();
    h.devices.adopt("bbb".into()).await.unwrap();

    assert_eq!(h.fixtures().await.len(), 2);
    assert_eq!(h.fixture_types().await.len(), 1, "the type id is derived, not random");
}

#[tokio::test]
async fn adopting_twice_is_not_an_error_and_patches_once() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;

    let first = h.devices.adopt("1a2b3c".into()).await.unwrap();
    let second = h.devices.adopt("1a2b3c".into()).await.unwrap();

    assert_eq!(first, second);
    assert_eq!(h.fixtures().await.len(), 1);
}

#[tokio::test]
async fn a_gateway_is_adopted_onto_a_universe_nothing_else_is_using() {
    let h = harness().await;
    h.resolve("gate1", openhaunt::MODULE_TYPE_DMX_OUT, None).await;
    h.resolve("gate2", openhaunt::MODULE_TYPE_DMX_OUT, None).await;

    h.devices.adopt("gate1".into()).await.unwrap();
    h.devices.adopt("gate2".into()).await.unwrap();

    let universes: Vec<Option<u16>> = h
        .fixtures()
        .await
        .iter()
        .map(|f| match &f.address {
            FixtureAddress::OpenHaunt { universe, .. } => *universe,
            FixtureAddress::Dmx { universe, .. } => Some(*universe),
        })
        .collect();
    assert_eq!(universes, vec![Some(1), Some(2)], "two gateways must not share a universe");
}

#[tokio::test]
async fn a_module_this_console_does_not_know_is_refused_rather_than_guessed_at() {
    let h = harness().await;
    h.resolve("weird", 0x00ff, None).await;

    let error = h.devices.adopt("weird".into()).await.unwrap_err();
    assert!(error.contains("0x00ff"), "the message has to name the module: {error}");
    assert!(h.fixtures().await.is_empty());
}

#[tokio::test]
async fn adopting_a_device_nobody_has_seen_fails() {
    let h = harness().await;
    assert!(h.devices.adopt("ghost".into()).await.is_err());
}

#[tokio::test]
async fn a_follower_refuses_to_adopt() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;
    h.become_follower().await;

    let error = h.devices.adopt("1a2b3c".into()).await.unwrap_err();
    assert!(error.contains("leading the session"), "{error}");
    assert!(h.fixtures().await.is_empty(), "a follower must patch nothing");
}

// ── Forgetting ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn forgetting_a_device_unpatches_its_fixture_but_leaves_the_device_listed() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;
    h.devices.adopt("1a2b3c".into()).await.unwrap();

    h.devices.forget("1a2b3c".into()).await.unwrap();

    assert!(h.fixtures().await.is_empty());
    let state = h.state().await;
    assert!(state.discovered.contains_key("1a2b3c"), "it is still on the network");
    assert_eq!(state.discovered["1a2b3c"].adopted_fixture_id, None);
}

#[tokio::test]
async fn deleting_the_fixture_by_hand_un_adopts_the_device() {
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;
    let fixture_id = h.devices.adopt("1a2b3c".into()).await.unwrap();

    h.engine
        .set(
            super::delete_path("fixtures", fixture_id),
            Lifecycle::Persisted,
            serde_json::Value::Null,
        )
        .await
        .unwrap();
    // The manager notices on the next thing it hears about that device.
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_DIGITAL_IN, None).await;

    assert_eq!(h.state().await.discovered["1a2b3c"].adopted_fixture_id, None);
}

// ── Talking to a node ─────────────────────────────────────────────────────────

#[tokio::test]
async fn identify_reaches_the_node() {
    let (addr, log) = a_node(0).await;
    let h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_OLED, Some(addr)).await;

    h.devices.identify("1a2b3c".into()).await.unwrap();

    assert_eq!(log.lock().unwrap().identified, 1);
}

#[tokio::test]
async fn setting_an_output_posts_the_port_the_node_numbers_it_by() {
    let (addr, log) = a_node(0).await;
    let h = harness().await;
    h.resolve("4d5e6f", openhaunt::MODULE_TYPE_MAINS_RELAY, Some(addr)).await;

    h.devices.set_output("4d5e6f".into(), 0, serde_json::json!({ "state": true }));
    // Ordered behind the SetOutput on the same channel.
    let _ = h.devices.identify("4d5e6f".into()).await;

    let outputs = log.lock().unwrap().outputs.clone();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0], serde_json::json!({ "outputs": { "0": { "state": true } } }));
}

// ── Directory ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_directory_carries_where_each_device_is() {
    let mut h = harness().await;
    h.resolve("1a2b3c", openhaunt::MODULE_TYPE_MAINS_RELAY, None).await;

    let directory = h.directory.borrow_and_update().clone();
    let entry = directory.entries.get("1a2b3c").expect("the device is in the directory");
    assert_eq!(entry.ip, "127.0.0.1");
    assert_eq!(entry.module_type, openhaunt::MODULE_TYPE_MAINS_RELAY);
    assert!(entry.online);
}

// ── Parsing ───────────────────────────────────────────────────────────────────

#[test]
fn a_module_id_is_read_as_hex_or_decimal() {
    assert_eq!(parse_module_type("0x0002"), Some(2));
    assert_eq!(parse_module_type("0X0007"), Some(7));
    assert_eq!(parse_module_type(" 4 "), Some(4));
    assert_eq!(parse_module_type("nonsense"), None);
}

#[test]
fn a_removed_service_name_gives_up_its_serial() {
    let event = device_event(mdns_sd::ServiceEvent::ServiceRemoved(
        SERVICE_TYPE.into(),
        format!("openhaunt-1a2b3c.{SERVICE_TYPE}"),
    ));
    assert!(matches!(event, Some(DeviceEvent::Removed { serial }) if serial == "1a2b3c"));
}
