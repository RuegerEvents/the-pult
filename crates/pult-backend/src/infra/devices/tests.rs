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
        fixture::{
            Fixture, FixtureAddress, FixtureType, ParameterDirection, ParameterKind,
        },
        openhaunt,
    },
};

use super::*;
use crate::{engine::ShowEngine, infra::showfile};

// ── The catalogue, as a device would know it ──────────────────────────────────
//
// A test-side table on purpose. In a unit test this *is* the firmware, and the
// firmware is the only thing entitled to know what a module's ports are — the
// console keeps no such table, which is what these tests are here to hold it to.

const DMX_OUT: u16 = 0x0001;
const DIGITAL_IN: u16 = 0x0002;
const WS2812: u16 = 0x0003;
const MAINS_RELAY: u16 = 0x0004;
const OLED: u16 = 0x0005;
const DRY_CONTACT: u16 = 0x0006;
const ENVIRONMENT: u16 = 0x0007;

/// What a module's firmware puts on the wire: the name in its TXT record, its
/// descriptor flags, and the ports it describes in `GET /api/v1/info`.
struct Firmware {
    name: &'static str,
    flags: u32,
    description: serde_json::Value,
}

fn firmware(module_type: u16) -> Firmware {
    let (name, flags, description) = match module_type {
        DMX_OUT => (
            "DMX Gateway",
            0,
            serde_json::json!({ "ports": [], "dmx": { "protocols": ["sacn"], "universes": 1 } }),
        ),
        DIGITAL_IN => (
            "Digital Inputs",
            0,
            serde_json::json!({ "ports": (0..8).map(|n| serde_json::json!({
                "port": n, "name": format!("Input {}", n + 1),
                "access": "readonly", "dataType": "boolean", "class": "contact",
            })).collect::<Vec<_>>() }),
        ),
        // The module that can trace everything, which is what makes it the one to
        // test offload against.
        WS2812 => (
            "LED Strip",
            0,
            serde_json::json!({ "ports": [
                { "port": 0, "name": "Strip colour", "access": "readwrite",
                  "dataType": "color", "class": "color",
                  "effects": { "shapes": ["sine", "triangle", "square", "saw-up", "saw-down"],
                               "steps": true, "transitions": true } },
                { "port": 1, "name": "Brightness", "access": "readwrite",
                  "dataType": "number", "unit": "percent", "minimum": 0, "maximum": 1,
                  "default": 0, "class": "intensity",
                  "effects": { "shapes": ["sine", "triangle", "square", "saw-up", "saw-down"],
                               "steps": true, "transitions": true } },
            ]}),
        ),
        MAINS_RELAY => (
            "Mains Relay",
            openhaunt::MODULE_FLAG_MAINS,
            serde_json::json!({ "ports": [
                { "port": 0, "name": "Relay", "access": "readwrite",
                  "dataType": "boolean", "default": 0, "class": "switch" },
            ]}),
        ),
        OLED => (
            "Display",
            0,
            serde_json::json!({ "ports": [
                { "port": 0, "name": "Line", "access": "readwrite",
                  "dataType": "string", "class": "text" },
            ]}),
        ),
        DRY_CONTACT => (
            "Dry Contacts",
            0,
            serde_json::json!({ "ports": (0..4).map(|n| serde_json::json!({
                "port": n, "name": format!("Contact {}", n + 1),
                "access": "readwrite", "dataType": "boolean", "class": "switch",
            })).collect::<Vec<_>>() }),
        ),
        ENVIRONMENT => (
            "Environment Sensor",
            0,
            serde_json::json!({ "ports": [
                { "port": 0, "name": "Temperature", "access": "readonly", "dataType": "number",
                  "unit": "degree-celsius", "minimum": -40, "maximum": 85, "class": "temperature" },
                { "port": 1, "name": "Humidity", "access": "readonly", "dataType": "number",
                  "unit": "percent", "minimum": 0, "maximum": 100, "class": "humidity" },
                { "port": 2, "name": "Air quality", "access": "readonly", "dataType": "number",
                  "unit": "parts-per-million", "minimum": 0, "maximum": 5000,
                  "class": "air-quality" },
            ]}),
        ),
        other => panic!("no firmware written for module {other:#06x}"),
    };
    Firmware { name, flags, description }
}

/// The fixture type a described module becomes, for a test that wants to compare.
fn described(module_type: u16) -> openhaunt::NodeDescription {
    serde_json::from_value(firmware(module_type).description).expect("a description parses")
}

// ── Harness ───────────────────────────────────────────────────────────────────

pub(super) struct Harness {
    pub engine: EngineHandle,
    pub devices: DeviceHandle,
    pub directory: watch::Receiver<DeviceDirectory>,
}

/// A port nothing is listening on, found by binding and letting go.
///
/// rumqttd is told a concrete address and never says which one it got, so `0` is
/// not usable — and the broker is a process-wide singleton anyway, so the first
/// test to want one fixes the port for all of them.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a free port")
        .local_addr()
        .expect("a bound address")
        .port()
}

pub(super) async fn harness() -> Harness {
    let pool = Arc::new(showfile::open_in_memory().await.expect("open in-memory showfile"));
    let (engine, engine_handle, _broadcast) =
        ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
    tokio::spawn(engine.run());

    let (manager, devices, directory) =
        DeviceManager::new(NodeId(Uuid::new_v4()), engine_handle.clone(), free_port());
    tokio::spawn(manager.run());

    Harness { engine: engine_handle, devices, directory }
}

impl Harness {
    /// Discovery, as though mDNS had resolved a node at `addr`.
    pub(super) async fn resolve_at(
        &self,
        serial: &str,
        module_type: u16,
        addr: std::net::SocketAddr,
    ) {
        self.resolve(serial, module_type, Some(addr)).await
    }

    async fn resolve(&self, serial: &str, module_type: u16, addr: Option<std::net::SocketAddr>) {
        // Without an address of its own, the node still has to be somewhere: only
        // the device knows what it is, so a test node that answers nothing could
        // never be adopted. One is started here describing the module in question.
        let addr = match addr {
            Some(addr) => addr,
            None => {
                let module = firmware(module_type);
                a_node(module.flags, module.description).await.0
            }
        };
        let mut txt = BTreeMap::new();
        txt.insert("modname".to_string(), firmware(module_type).name.to_string());
        self.resolve_with(serial, module_type, addr, txt).await
    }

    /// Discovery with extra TXT keys, for the few facts a node states before
    /// anything has been asked of it.
    async fn resolve_with(
        &self,
        serial: &str,
        module_type: u16,
        addr: std::net::SocketAddr,
        extra: BTreeMap<String, String>,
    ) {
        let mut txt = BTreeMap::new();
        txt.insert("sn".to_string(), serial.to_string());
        txt.insert("mod".to_string(), format!("{module_type:#06x}"));
        txt.insert("name".to_string(), format!("Node {serial}"));
        txt.insert("fw".to_string(), "0.1.0".to_string());
        txt.insert("v".to_string(), "1".to_string());
        txt.insert("caps".to_string(), "dmx,rdm".to_string());
        txt.extend(extra);

        self.event(DeviceEvent::Resolved {
            serial: serial.to_string(),
            ip: addr.ip().to_string(),
            port: addr.port(),
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

    /// A second manager against the same engine, as a restarted console would be.
    pub(super) async fn with_a_fresh_device_manager(&self) -> Harness {
        self.devices.0.send(DeviceCommand::Stop).await.unwrap();
        let (manager, devices, directory) =
            DeviceManager::new(NodeId(Uuid::new_v4()), self.engine.clone(), free_port());
        tokio::spawn(manager.run());
        Harness { engine: self.engine.clone(), devices, directory }
    }

    pub(super) async fn state(&self) -> DevicesState {
        let value = self
            .engine
            .get(vec![PathSegment::Key("devices".into())])
            .await
            .expect("devices is a LOCAL path and always answers");
        serde_json::from_value(value).expect("devices state round-trips")
    }

    pub(super) async fn fixtures(&self) -> Vec<Fixture> {
        let value = self.engine.get(vec![PathSegment::Key("fixtures".into())]).await.unwrap();
        serde_json::from_value(value).unwrap()
    }

    async fn outputs(&self) -> Vec<OutputConfig> {
        let value = self.engine.get(vec![PathSegment::Key("outputs".into())]).await.unwrap();
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
    configs: Vec<serde_json::Value>,
}

/// A stand-in for a node's HTTP API, on an ephemeral port, recording what it was
/// asked to do.
///
/// `description` is what it serves alongside its descriptor from `/api/v1/info` —
/// the ports and, on a gateway, the universe. `json!({})` is firmware that
/// describes nothing, which the console is entitled to refuse.
async fn a_node(
    module_flags: u32,
    description: serde_json::Value,
) -> (std::net::SocketAddr, Arc<Mutex<NodeLog>>) {
    let log = Arc::new(Mutex::new(NodeLog::default()));

    let info_log = log.clone();
    let identify_log = log.clone();
    let state_log = log.clone();
    let config_log = log.clone();
    let app = Router::new()
        .route(
            "/api/v1/info",
            get(move || {
                let _ = &info_log;
                let mut info = serde_json::json!({ "module": { "flags": module_flags } });
                if let Some(described) = description.as_object() {
                    for (key, value) in described {
                        info[key] = value.clone();
                    }
                }
                async move { Json(info) }
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
        )
        .route(
            "/api/v1/config",
            post(move |Json(body): Json<serde_json::Value>| {
                let log = config_log.clone();
                async move {
                    log.lock().unwrap().configs.push(body);
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
    h.resolve("1a2b3c", DIGITAL_IN, None).await;

    let state = h.state().await;
    let device = state.discovered.get("1a2b3c").expect("the device is listed");
    assert_eq!(device.name, "Node 1a2b3c");
    assert_eq!(device.module_type, DIGITAL_IN);
    assert_eq!(device.module_name, "Digital Inputs", "the module name fills in from the id");
    assert_eq!(device.caps, vec!["dmx", "rdm"]);
    assert!(device.online);
    assert_eq!(device.adopted_fixture_id, None);
}

#[tokio::test]
async fn resolving_the_same_node_twice_updates_it_rather_than_duplicating_it() {
    let h = harness().await;
    h.resolve("1a2b3c", MAINS_RELAY, None).await;
    h.resolve("1a2b3c", MAINS_RELAY, None).await;

    assert_eq!(h.state().await.discovered.len(), 1);
}

#[tokio::test]
async fn a_node_that_says_it_switches_mains_is_warned_about() {
    // A dry-contact module wired to mains. Nothing about the module id says so;
    // only the descriptor the node serves does, and that is what is believed.
    let (addr, _log) =
        a_node(openhaunt::MODULE_FLAG_MAINS, firmware(DRY_CONTACT).description).await;
    let h = harness().await;
    h.resolve("4d5e6f", DRY_CONTACT, Some(addr)).await;

    assert!(h.state().await.discovered["4d5e6f"].is_mains);
}

#[tokio::test]
async fn a_txt_record_that_claims_mains_warns_before_an_http_round_trip() {
    // The warning is worth showing a moment early rather than a round trip late,
    // and a node that never answers `/info` is exactly when it matters. Port 1 is
    // nothing's, so the call is refused at once rather than sitting out a timeout.
    let h = harness().await;
    let mut txt = BTreeMap::new();
    txt.insert("mains".to_string(), "1".to_string());
    h.resolve_with("4d5e6f", DRY_CONTACT, ([127, 0, 0, 1], 1).into(), txt).await;

    assert!(h.state().await.discovered["4d5e6f"].is_mains);
}

#[tokio::test]
async fn an_unadopted_node_that_goes_quiet_is_forgotten() {
    let h = harness().await;
    h.resolve("1a2b3c", DIGITAL_IN, None).await;
    h.event(DeviceEvent::Removed { serial: "1a2b3c".into() }).await;

    assert!(h.state().await.discovered.is_empty());
}

#[tokio::test]
async fn an_adopted_node_that_goes_quiet_stays_listed_as_offline() {
    let h = harness().await;
    h.resolve("1a2b3c", DIGITAL_IN, None).await;
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
    h.resolve("1a2b3c", DIGITAL_IN, None).await;
    let fixture_id = h.devices.adopt("1a2b3c".into()).await.unwrap();

    h.event(DeviceEvent::Removed { serial: "1a2b3c".into() }).await;
    h.resolve("1a2b3c", DIGITAL_IN, None).await;

    let state = h.state().await;
    assert!(state.discovered["1a2b3c"].online);
    assert_eq!(state.discovered["1a2b3c"].adopted_fixture_id, Some(fixture_id));
}

// ── Adoption ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn adopting_a_node_patches_it_as_a_fixture_of_its_module_s_type() {
    let h = harness().await;
    h.resolve("1a2b3c", DIGITAL_IN, None).await;

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
    assert_eq!(
        types[0].id,
        openhaunt::fixture_type_id(DIGITAL_IN, &described(DIGITAL_IN)),
        "the type follows from what the node said about itself",
    );
    assert_eq!(types[0].name, "Digital Inputs");
    assert_eq!(types[0].parameters.len(), 8);
}

#[tokio::test]
async fn adopting_gives_the_show_an_openhaunt_output_once() {
    // The plugin that drives a node's ports only runs where an `outputs` row says
    // so. Adoption is the operator asking for the node to be driven, so the row
    // appears then — and one row covers every node this station drives.
    let h = harness().await;
    assert!(h.outputs().await.is_empty(), "nothing is configured before anything is adopted");

    h.resolve("1a2b3c", DIGITAL_IN, None).await;
    h.devices.adopt("1a2b3c".into()).await.unwrap();

    let outputs = h.outputs().await;
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].kind, OutputKind::OpenHaunt);
    assert!(outputs[0].enabled);
    assert!(outputs[0].node_id.is_some(), "the output belongs to the station that drives");

    h.resolve("4d5e6f", MAINS_RELAY, None).await;
    h.devices.adopt("4d5e6f".into()).await.unwrap();
    assert_eq!(h.outputs().await.len(), 1, "a second node does not need a second output");
}

#[tokio::test]
async fn an_openhaunt_output_the_operator_switched_off_is_left_off() {
    let h = harness().await;
    let off = OutputConfig {
        id: Uuid::new_v4(),
        name: "nodes, but not tonight".into(),
        kind: OutputKind::OpenHaunt,
        target: None,
        universes: Vec::new(),
        enabled: false,
        node_id: None,
    };
    h.engine
        .set(create_path("outputs"), Lifecycle::Persisted, serde_json::to_value(&off).unwrap())
        .await
        .unwrap();

    h.resolve("1a2b3c", DIGITAL_IN, None).await;
    h.devices.adopt("1a2b3c".into()).await.unwrap();

    let outputs = h.outputs().await;
    assert_eq!(outputs.len(), 1, "an output that covers this station is not duplicated");
    assert!(!outputs[0].enabled, "and switching it off was a decision");
}

#[tokio::test]
async fn adopting_two_of_the_same_module_makes_one_fixture_type() {
    let h = harness().await;
    h.resolve("aaa", MAINS_RELAY, None).await;
    h.resolve("bbb", MAINS_RELAY, None).await;

    h.devices.adopt("aaa".into()).await.unwrap();
    h.devices.adopt("bbb".into()).await.unwrap();

    assert_eq!(h.fixtures().await.len(), 2);
    assert_eq!(h.fixture_types().await.len(), 1, "the type id is derived, not random");
}

#[tokio::test]
async fn adopting_twice_is_not_an_error_and_patches_once() {
    let h = harness().await;
    h.resolve("1a2b3c", DIGITAL_IN, None).await;

    let first = h.devices.adopt("1a2b3c".into()).await.unwrap();
    let second = h.devices.adopt("1a2b3c".into()).await.unwrap();

    assert_eq!(first, second);
    assert_eq!(h.fixtures().await.len(), 1);
}

#[tokio::test]
async fn a_gateway_is_adopted_onto_a_universe_nothing_else_is_using() {
    let h = harness().await;
    h.resolve("gate1", DMX_OUT, None).await;
    h.resolve("gate2", DMX_OUT, None).await;

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
async fn a_node_that_does_not_describe_its_ports_cannot_be_adopted() {
    // Firmware older than self-description, or a node that answered nothing. The
    // console has no table to fall back on and does not pretend otherwise.
    let (addr, _log) = a_node(0, serde_json::json!({})).await;
    let h = harness().await;
    h.resolve("silent", MAINS_RELAY, Some(addr)).await;

    let error = h.devices.adopt("silent".into()).await.unwrap_err();
    assert!(error.contains("does not describe its ports"), "{error}");
    assert!(h.fixtures().await.is_empty());
}

#[tokio::test]
async fn a_module_this_console_has_never_heard_of_is_adopted_from_its_own_words() {
    // Nothing here knows what a fogger is. The node does, and that is enough.
    let (addr, _log) = a_node(
        0,
        serde_json::json!({ "ports": [
            { "port": 0, "name": "Fog output", "access": "readwrite",
              "dataType": "number", "unit": "percent", "class": "fog-density" },
            { "port": 1, "name": "Tank level", "access": "readonly",
              "dataType": "number", "unit": "percent" },
        ]}),
    )
    .await;
    let h = harness().await;
    let mut txt = BTreeMap::new();
    txt.insert("modname".to_string(), "Fogger".to_string());
    h.resolve_with("fog1", 0x00ff, addr, txt).await;

    h.devices.adopt("fog1".into()).await.unwrap();

    let types = h.fixture_types().await;
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].name, "Fogger");
    assert_eq!(types[0].parameters[0].kind, ParameterKind::Named("Fog output".into()));
    assert_eq!(types[0].parameters[0].direction, ParameterDirection::Output);
    assert_eq!(types[0].parameters[1].kind, ParameterKind::Named("Tank level".into()));
    assert_eq!(types[0].parameters[1].direction, ParameterDirection::Input);
}

#[tokio::test]
async fn a_universe_is_allocated_to_the_node_that_says_it_forwards_one() {
    let h = harness().await;
    h.resolve("gate", DMX_OUT, None).await;
    h.resolve("relay", MAINS_RELAY, None).await;

    h.devices.adopt("gate".into()).await.unwrap();
    h.devices.adopt("relay".into()).await.unwrap();

    let fixtures = h.fixtures().await;
    let universe = |serial: &str| {
        fixtures.iter().find_map(|f| match &f.address {
            FixtureAddress::OpenHaunt { serial: s, universe } if s == serial => Some(*universe),
            _ => None,
        })
    };
    assert_eq!(universe("gate"), Some(Some(1)), "it described a universe to forward");
    assert_eq!(universe("relay"), Some(None), "it described only a relay");
}

#[tokio::test]
async fn adopting_a_device_nobody_has_seen_fails() {
    let h = harness().await;
    assert!(h.devices.adopt("ghost".into()).await.is_err());
}

#[tokio::test]
async fn a_follower_refuses_to_adopt() {
    let h = harness().await;
    h.resolve("1a2b3c", DIGITAL_IN, None).await;
    h.become_follower().await;

    let error = h.devices.adopt("1a2b3c".into()).await.unwrap_err();
    assert!(error.contains("leading the session"), "{error}");
    assert!(h.fixtures().await.is_empty(), "a follower must patch nothing");
}

// ── Forgetting ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn forgetting_a_device_unpatches_its_fixture_but_leaves_the_device_listed() {
    let h = harness().await;
    h.resolve("1a2b3c", DIGITAL_IN, None).await;
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
    h.resolve("1a2b3c", DIGITAL_IN, None).await;
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
    h.resolve("1a2b3c", DIGITAL_IN, None).await;

    assert_eq!(h.state().await.discovered["1a2b3c"].adopted_fixture_id, None);
}

// ── Talking to a node ─────────────────────────────────────────────────────────

#[tokio::test]
async fn identify_reaches_the_node() {
    let (addr, log) = a_node(0, firmware(OLED).description).await;
    let h = harness().await;
    h.resolve("1a2b3c", OLED, Some(addr)).await;

    h.devices.identify("1a2b3c".into()).await.unwrap();

    assert_eq!(log.lock().unwrap().identified, 1);
}

#[tokio::test]
async fn setting_an_output_posts_the_port_the_node_numbers_it_by() {
    let (addr, log) = a_node(firmware(MAINS_RELAY).flags, firmware(MAINS_RELAY).description).await;
    let h = harness().await;
    h.resolve("4d5e6f", MAINS_RELAY, Some(addr)).await;

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
    h.resolve("1a2b3c", MAINS_RELAY, None).await;

    let directory = h.directory.borrow_and_update().clone();
    let entry = directory.entries.get("1a2b3c").expect("the device is in the directory");
    assert_eq!(entry.ip, "127.0.0.1");
    assert_eq!(entry.module_type, MAINS_RELAY);
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

// ── The broker and the nodes on it ────────────────────────────────────────────
//
// A real rumqttc client standing in for a node, against the broker the manager
// started. Nothing here is mocked: the topics, the payloads, and the merge into
// live_values are the ones a real node would drive.

use rumqttc::{AsyncClient, MqttOptions, QoS};

/// Where the manager's broker ended up. It is a process-wide singleton, so this is
/// whatever the first harness in the run asked for.
async fn broker_port(h: &Harness) -> u16 {
    for _ in 0..200 {
        if let Some(addr) = h.state().await.broker_addr {
            if let Some((_, port)) = addr.rsplit_once(':') {
                return port.parse().expect("the broker address ends in a port");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the leader never started a broker");
}

/// A node's MQTT client: connected, and announced on its status topic.
async fn a_node_on_the_broker(h: &Harness, serial: &str) -> AsyncClient {
    let mut options = MqttOptions::new(format!("node-{serial}"), "127.0.0.1", broker_port(h).await);
    options.set_keep_alive(std::time::Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(options, 16);
    tokio::spawn(async move { while eventloop.poll().await.is_ok() {} });

    client
        .publish(format!("openhaunt/{serial}/status"), QoS::AtLeastOnce, true, "online")
        .await
        .unwrap();
    client
}

pub(super) async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

pub(super) async fn live_value(
    h: &Harness,
    fixture_id: Uuid,
    key: &str,
) -> Option<serde_json::Value> {
    h.fixtures()
        .await
        .into_iter()
        .find(|f| f.id == fixture_id)?
        .sensed_values
        .get(key)
        .map(|v| serde_json::to_value(v).unwrap())
}

#[tokio::test]
async fn a_console_with_nothing_adopted_runs_no_broker() {
    let h = harness().await;
    h.resolve("seen", DIGITAL_IN, None).await;

    let state = h.state().await;
    assert!(state.active, "it is still the node that would drive them");
    assert_eq!(state.broker_addr, None, "but it has nothing to run a broker for");
}

#[tokio::test]
async fn adopting_the_first_device_brings_the_broker_up() {
    let h = harness().await;
    h.resolve("first", DIGITAL_IN, None).await;
    h.devices.adopt("first".into()).await.unwrap();

    assert!(broker_port(&h).await > 0);
}

#[tokio::test]
async fn an_input_edge_lands_in_the_fixture_s_live_values() {
    let h = harness().await;
    h.resolve("edge1", DIGITAL_IN, None).await;
    let fixture_id = h.devices.adopt("edge1".into()).await.unwrap();

    let node = a_node_on_the_broker(&h, "edge1").await;
    node.publish(
        "openhaunt/edge1/input/3",
        QoS::AtLeastOnce,
        false,
        r#"{"state": true, "edge": "rising", "ts": 900}"#,
    )
    .await
    .unwrap();

    eventually("the contact to close in the show", || async {
        live_value(&h, fixture_id, "Contact:3").await
            == Some(serde_json::json!({ "type": "Bool", "value": true }))
    })
    .await;
}

#[tokio::test]
async fn a_sensor_reading_lands_on_the_parameter_its_port_is_bound_to() {
    let h = harness().await;
    h.resolve("env1", ENVIRONMENT, None).await;
    let fixture_id = h.devices.adopt("env1".into()).await.unwrap();

    let node = a_node_on_the_broker(&h, "env1").await;
    // Port 1 is Humidity on this module, and nothing on the wire says so.
    node.publish("openhaunt/env1/input/1", QoS::AtLeastOnce, false, r#"{"value": 42.0}"#)
        .await
        .unwrap();

    eventually("the humidity reading to arrive", || async {
        live_value(&h, fixture_id, "Humidity").await
            == Some(serde_json::json!({ "type": "Float", "value": 42.0 }))
    })
    .await;
}

#[tokio::test]
async fn an_input_from_a_device_nobody_adopted_changes_nothing() {
    let h = harness().await;
    // One adopted device, so there is a broker; the stray one is beside it.
    h.resolve("neighbour", MAINS_RELAY, None).await;
    h.devices.adopt("neighbour".into()).await.unwrap();
    h.resolve("stray", DIGITAL_IN, None).await;

    let node = a_node_on_the_broker(&h, "stray").await;
    node.publish("openhaunt/stray/input/0", QoS::AtLeastOnce, false, r#"{"state": true}"#)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let fixtures = h.fixtures().await;
    assert!(
        fixtures.iter().all(|f| f.sensed_values.is_empty()),
        "an unadopted device drives nothing",
    );
}

#[tokio::test]
async fn health_from_a_node_shows_up_against_it() {
    let h = harness().await;
    h.resolve("health1", DIGITAL_IN, None).await;
    h.devices.adopt("health1".into()).await.unwrap();

    let node = a_node_on_the_broker(&h, "health1").await;
    node.publish(
        "openhaunt/health1/health",
        QoS::AtLeastOnce,
        false,
        r#"{"uptime_s": 120, "temp_c": 38.0, "poe_class": 3, "errors": []}"#,
    )
    .await
    .unwrap();

    eventually("health to show up", || async {
        h.state().await.discovered["health1"].health.as_ref().map(|hh| hh.uptime_s) == Some(120)
    })
    .await;
    let state = h.state().await;
    assert_eq!(state.discovered["health1"].health.as_ref().unwrap().temperature_c, Some(38.0));
}

#[tokio::test]
async fn a_node_that_publishes_offline_is_marked_offline() {
    let h = harness().await;
    h.resolve("offline1", DIGITAL_IN, None).await;
    h.devices.adopt("offline1".into()).await.unwrap();

    let node = a_node_on_the_broker(&h, "offline1").await;
    node.publish("openhaunt/offline1/status", QoS::AtLeastOnce, true, "offline").await.unwrap();

    eventually("the node to be marked offline", || async {
        !h.state().await.discovered["offline1"].online
    })
    .await;
}

#[tokio::test]
async fn adopting_a_node_tells_it_where_the_broker_is() {
    let (addr, log) = a_node(firmware(MAINS_RELAY).flags, firmware(MAINS_RELAY).description).await;
    let h = harness().await;
    h.resolve("4d5e6f", MAINS_RELAY, Some(addr)).await;

    h.devices.adopt("4d5e6f".into()).await.unwrap();

    let advertised = h.state().await.broker_addr;
    assert!(advertised.is_some(), "the first adoption brings the broker up");
    let configs = log.lock().unwrap().configs.clone();
    assert_eq!(configs.len(), 1, "adoption is when a node learns where to publish");
    assert_eq!(configs[0]["mqtt"]["broker"], serde_json::json!(advertised));
    assert!(configs[0].get("dmx").is_none(), "a relay has no universe to forward");
}

#[tokio::test]
async fn adopting_a_gateway_also_tells_it_which_universe_to_listen_for() {
    let (addr, log) = a_node(firmware(DMX_OUT).flags, firmware(DMX_OUT).description).await;
    let h = harness().await;
    h.resolve("gate1", DMX_OUT, Some(addr)).await;

    h.devices.adopt("gate1".into()).await.unwrap();

    let configs = log.lock().unwrap().configs.clone();
    assert_eq!(configs[0]["dmx"], serde_json::json!({ "protocol": "sacn", "universe": 1 }));
}

#[tokio::test]
async fn a_follower_does_not_drive_devices() {
    let h = harness().await;
    h.resolve("follow1", DIGITAL_IN, None).await;
    h.devices.adopt("follow1".into()).await.unwrap();
    let _ = broker_port(&h).await;

    h.become_follower().await;

    eventually("the node to stop driving", || async { !h.state().await.active }).await;
}

#[tokio::test]
async fn a_node_promoted_back_to_leading_drives_again() {
    let h = harness().await;
    h.resolve("promo1", DIGITAL_IN, None).await;
    h.devices.adopt("promo1".into()).await.unwrap();
    let _ = broker_port(&h).await;
    h.become_follower().await;
    eventually("the node to stop driving", || async { !h.state().await.active }).await;

    h.engine
        .set(
            vec![PathSegment::Key("session".into())],
            Lifecycle::Local,
            serde_json::json!({
                "is_advertising": true, "is_follower": false,
                "session_id": Uuid::new_v4(), "discovered": [],
            }),
        )
        .await
        .unwrap();

    eventually("the node to drive again", || async { h.state().await.active }).await;
}

#[tokio::test]
async fn a_node_that_comes_back_is_told_where_the_broker_is_again() {
    // A rebooted node has forgotten its configuration, and announcing itself is the
    // only notice the console gets that it needs telling.
    let (addr, log) = a_node(firmware(MAINS_RELAY).flags, firmware(MAINS_RELAY).description).await;
    let h = harness().await;
    h.resolve("reboot1", MAINS_RELAY, Some(addr)).await;
    h.devices.adopt("reboot1".into()).await.unwrap();
    assert_eq!(log.lock().unwrap().configs.len(), 1);

    h.event(DeviceEvent::Removed { serial: "reboot1".into() }).await;
    h.resolve("reboot1", MAINS_RELAY, Some(addr)).await;

    assert_eq!(log.lock().unwrap().configs.len(), 2);
}

#[tokio::test]
async fn a_node_nobody_adopted_is_not_configured_when_it_appears() {
    let (addr, log) =
        a_node(firmware(MAINS_RELAY).flags, firmware(MAINS_RELAY).description).await;
    let h = harness().await;
    // Something else adopted, so there is a broker to be told about.
    h.resolve("other", DIGITAL_IN, None).await;
    h.devices.adopt("other".into()).await.unwrap();

    h.resolve("unadopted", MAINS_RELAY, Some(addr)).await;

    assert!(
        log.lock().unwrap().configs.is_empty(),
        "a discovered node is asked for nothing until it is adopted",
    );
}

#[tokio::test]
async fn a_console_restarted_mid_show_recognises_the_devices_it_had_adopted() {
    // Adoption lives in the fixture, which is persisted; the device list is not.
    // A fresh manager against the same show has to work it out from the fixtures.
    let (addr, log) = a_node(firmware(MAINS_RELAY).flags, firmware(MAINS_RELAY).description).await;
    let h = harness().await;
    h.resolve("survivor", MAINS_RELAY, Some(addr)).await;
    let fixture_id = h.devices.adopt("survivor".into()).await.unwrap();

    let after_restart = h.with_a_fresh_device_manager().await;
    after_restart.resolve("survivor", MAINS_RELAY, Some(addr)).await;

    let state = after_restart.state().await;
    assert_eq!(
        state.discovered["survivor"].adopted_fixture_id,
        Some(fixture_id),
        "the fixture is still patched, so the device is still adopted",
    );
    assert!(
        log.lock().unwrap().configs.len() >= 2,
        "and it has to be told where the broker is now, since it cannot know",
    );
}

#[tokio::test]
async fn a_follower_with_nothing_adopted_still_says_it_is_not_driving() {
    // Nothing to start or stop either side of the change, so the only thing that
    // moves is what this node says about itself — which still has to be said.
    let h = harness().await;
    h.resolve("watched", DIGITAL_IN, None).await;
    assert!(h.state().await.active);

    h.become_follower().await;

    eventually("the panel to stop offering to adopt", || async { !h.state().await.active }).await;
    assert!(
        h.state().await.discovered.contains_key("watched"),
        "a follower keeps browsing and keeps showing what it finds",
    );
}

// ── Effect capability ─────────────────────────────────────────────────────────

/// The capability is read out of the same `/info` body as everything else, and it
/// reaches the directory, which is where an output plugin looks — the plugin is
/// handed a patch and never the device list.
#[tokio::test]
async fn what_a_port_says_it_can_trace_reaches_the_directory() {
    let h = harness().await;
    let (addr, _log) = a_node(
        0,
        serde_json::json!({
            "ports": [
                { "port": 0, "name": "Strip colour", "access": "readwrite", "dataType": "color",
                  "class": "color",
                  "effects": { "shapes": ["sine", "square"], "steps": true, "transitions": true } },
                { "port": 1, "name": "Brightness", "access": "readwrite", "dataType": "number",
                  "class": "intensity" },
            ],
        }),
    )
    .await;

    h.resolve_at("strip1", MAINS_RELAY, addr).await;

    let entry = h.directory.borrow().entries.get("strip1").cloned().expect("in the directory");
    let caps = entry.effects.expect("the node said something");
    let colour = caps.port(0).expect("port 0 did");
    assert!(colour.has_shape("sine"));
    assert!(colour.steps);
    assert!(colour.transitions);
    assert!(caps.port(1).is_none(), "and port 1 did not, so it offers nothing");
}

/// The trap, end to end this time: adopting a node whose ports advertise effects
/// must land on the same fixture type as adopting one whose ports do not, or every
/// parameter patched against the old id is orphaned by a firmware update.
#[tokio::test]
async fn advertising_effects_does_not_change_the_fixture_type_a_node_adopts_as() {
    let ports = |effects: bool| {
        let mut port = serde_json::json!({
            "port": 0, "name": "Relay", "access": "readwrite", "dataType": "boolean",
            "class": "switch",
        });
        if effects {
            port["effects"] = serde_json::json!({ "shapes": ["square"], "steps": true });
        }
        serde_json::json!({ "ports": [port] })
    };

    let plain = harness().await;
    let (addr, _log) = a_node(0, ports(false)).await;
    plain.resolve_at("relay1", MAINS_RELAY, addr).await;
    let before = plain.devices.adopt("relay1".into()).await.unwrap();

    let advertising = harness().await;
    let (addr, _log) = a_node(0, ports(true)).await;
    advertising.resolve_at("relay1", MAINS_RELAY, addr).await;
    let after = advertising.devices.adopt("relay1".into()).await.unwrap();

    let type_of = |h: &Harness, id: Uuid| {
        let engine = h.engine.clone();
        async move {
            let fixture = engine
                .get(vec![
                    PathSegment::Key("fixtures".into()),
                    PathSegment::Id(id),
                    PathSegment::Key("fixture_type_id".into()),
                ])
                .await
                .unwrap();
            fixture
        }
    };

    assert_eq!(
        type_of(&plain, before).await,
        type_of(&advertising, after).await,
        "the unknown key must not reach the hash that names the fixture type",
    );
}

/// A node with no broker connection is still reachable over HTTP, and a shape has
/// to travel that way too or a node adopted but not yet connected would silently
/// keep tracing whatever it had.
#[tokio::test]
async fn a_shape_reaches_a_node_over_http_when_it_is_not_on_the_broker() {
    let h = harness().await;
    let (addr, log) = a_node(
        0,
        serde_json::json!({
            "ports": [
                { "port": 0, "name": "Relay", "access": "readwrite", "dataType": "boolean",
                  "class": "switch", "effects": { "shapes": ["square"], "steps": true } },
            ],
        }),
    )
    .await;
    h.resolve_at("relay1", MAINS_RELAY, addr).await;

    let descriptor = serde_json::json!({ "curve": { "shape": "square" }, "rate": 1.0 });
    h.devices.set_effect("relay1".into(), 0, Some(descriptor.clone()));
    eventually("the descriptor to arrive", || async {
        log.lock().unwrap().outputs.iter().any(|b| b["effects"]["0"] == descriptor)
    })
    .await;

    h.devices.set_effect("relay1".into(), 0, None);
    eventually("the clear to arrive", || async {
        log.lock()
            .unwrap()
            .outputs
            .iter()
            .any(|b| b["effects"]["0"] == serde_json::json!({ "clear": true }))
    })
    .await;
}

/// The clock is what lets a node place the start of a cycle, so it has to be there
/// before anything is asked to trace one — and retained, so a node that connects
/// between ticks gets an answer at once rather than rendering against a guess.
#[tokio::test]
async fn the_console_says_what_time_it_is_once_driving_starts() {
    let h = harness().await;
    h.resolve("relay1", MAINS_RELAY, None).await;
    h.devices.adopt("relay1".into()).await.unwrap();

    // A node that subscribes after the fact: a retained message is delivered on
    // subscribe, which is the whole reason for the flag.
    let mut options =
        MqttOptions::new("clock-watcher", "127.0.0.1", broker_port(&h).await);
    options.set_keep_alive(std::time::Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(options, 16);
    let seen: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    tokio::spawn(async move {
        while let Ok(event) = eventloop.poll().await {
            if let rumqttc::Event::Incoming(rumqttc::Packet::Publish(p)) = event {
                if let Ok(body) = serde_json::from_slice::<serde_json::Value>(&p.payload) {
                    recorder.lock().unwrap().push(body);
                }
            }
        }
    });
    client.subscribe("openhaunt/clock", QoS::AtLeastOnce).await.unwrap();

    eventually("the console to say what time it is", || async {
        seen.lock().unwrap().iter().any(|body| {
            body["t"].as_u64().is_some_and(|t| t > 1_600_000_000_000) && body["seq"].is_number()
        })
    })
    .await;
}
