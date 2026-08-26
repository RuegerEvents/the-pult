//! End to end, against a simulated node.
//!
//! The tests beside these drive the manager's own inputs. These drive nothing but
//! the node: it is resolved, adopted, and then told to do something, and every step
//! in between — the HTTP config push, the MQTT connection the node makes on its own,
//! the topic it publishes to — is the real one. `openhaunt-sim` shares no code with
//! the console, so agreeing here means the two ends genuinely agree.

use openhaunt_sim::{Input, ModuleKind, SimConfig, SimHandle};
use pult_schema::{path::PathSegment, types::openhaunt};

use super::tests::*;

/// Start a node and hand it to the manager as though mDNS had found it.
async fn a_simulated_node(h: &Harness, module: ModuleKind, serial: &str) -> SimHandle {
    let sim = openhaunt_sim::start(SimConfig::new(module, serial))
        .await
        .expect("the simulator binds an ephemeral port");
    // Advertising is off, so discovery is injected rather than waited for.
    h.resolve_at(serial, module.type_id(), sim.http_addr).await;
    sim
}

#[tokio::test]
async fn adopting_a_node_and_pressing_a_button_moves_a_live_value() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-in").await;
    let fixture_id = h.devices.adopt("e2e-in".into()).await.unwrap();

    // The console told it where to publish, which is the only configuration a node
    // ever receives.
    let mut config = sim.received_config.clone();
    eventually("the node to be configured", || {
        let config = config.clone();
        async move { config.borrow().is_some() }
    })
    .await;
    let broker = config.borrow_and_update().clone().expect("a config just arrived");
    assert_eq!(broker["mqtt"]["broker"], serde_json::json!(h.state().await.broker_addr));

    sim.inputs.send(Input::Contact { port: 2, state: true }).await.unwrap();

    eventually("the contact to close in the show", || async {
        live_value(&h, fixture_id, "Contact:2").await
            == Some(serde_json::json!({ "type": "Bool", "value": true }))
    })
    .await;
}

#[tokio::test]
async fn a_sensor_node_reports_readings_into_the_show() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::Environment, "e2e-env").await;
    let fixture_id = h.devices.adopt("e2e-env".into()).await.unwrap();

    sim.inputs.send(Input::Reading { port: 0, value: 19.5 }).await.unwrap();

    eventually("the temperature to arrive", || async {
        live_value(&h, fixture_id, "Temperature").await
            == Some(serde_json::json!({ "type": "Float", "value": 19.5 }))
    })
    .await;
}

#[tokio::test]
async fn driving_an_output_reaches_the_node_it_is_addressed_to() {
    let h = harness().await;
    let sim = a_simulated_node(&h, ModuleKind::MainsRelay, "e2e-relay").await;
    h.devices.adopt("e2e-relay".into()).await.unwrap();

    // Wait until the node has announced itself, so the console reaches it over the
    // broker rather than falling back to HTTP.
    let config = sim.received_config.clone();
    eventually("the node to be configured", || {
        let config = config.clone();
        async move { config.borrow().is_some() }
    })
    .await;

    let state = sim.state.clone();
    eventually("the relay to close", || {
        let state = state.clone();
        let devices = h.devices.clone();
        async move {
            devices.set_output("e2e-relay".into(), 0, serde_json::json!({ "state": true }));
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            state.borrow().get("0") == Some(&serde_json::json!({ "state": true }))
        }
    })
    .await;
}

#[tokio::test]
async fn a_node_answers_for_its_own_module_type() {
    // The mains warning comes from the node's descriptor, not from this console's
    // guess about what a module id means.
    let h = harness().await;
    let _sim = a_simulated_node(&h, ModuleKind::MainsRelay, "e2e-mains").await;

    let state = h.state().await;
    assert!(state.discovered["e2e-mains"].is_mains);
    assert_eq!(state.discovered["e2e-mains"].module_type, openhaunt::MODULE_TYPE_MAINS_RELAY);
}

#[tokio::test]
async fn forgetting_a_node_leaves_it_running_and_unpatched() {
    let h = harness().await;
    let _sim = a_simulated_node(&h, ModuleKind::DigitalIn, "e2e-forget").await;
    h.devices.adopt("e2e-forget".into()).await.unwrap();
    assert_eq!(h.fixtures().await.len(), 1);

    h.devices.forget("e2e-forget".into()).await.unwrap();

    assert!(h.fixtures().await.is_empty());
    let value = h.engine.get(vec![PathSegment::Key("devices".into())]).await.unwrap();
    assert!(value["discovered"]["e2e-forget"]["online"].as_bool().unwrap());
}
