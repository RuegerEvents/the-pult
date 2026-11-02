//! What a node and its ports are called, kept the same on the node and in the show.
//!
//! Both keep the names: the node so that its own page and anything else that asks it
//! says "Ausgang PIR" rather than "Sensor 1", the show so that a node that is off still
//! has them. Two copies drift, and when they do the station has to know which one moved.
//! [`Fixture::agreed_names`] is what the two last agreed on, and the rule is
//! [`verdict`]: a name that changed only in the show is passed on to the node, and a name
//! the node changed is asked about — the node was renamed on its own page, reset, or is a
//! different module now, and only the operator knows which.
//!
//! The labels are never the parameter's identity. `Named:<name>` is a key that cues and
//! flows hold, and it comes from what the *module* calls a port; a label is what the
//! *operator* calls it, kept in [`Fixture::parameter_labels`] beside it.

use std::collections::BTreeMap;

use pult_schema::{
    path::PathSegment,
    types::{
        devices::{DiscoveredDevice, NameChoice, NameConflict, NameSide},
        fixture::{AgreedNames, Fixture, FixtureAddress, FixtureType},
        openhaunt,
    },
};
use serde::Deserialize;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::{DeviceManager, create_path, replace_path};
use crate::model::playback::parameter_key;

/// One thing that has a name on both sides: a node, a head, or a port.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Named {
    pub fixture: Uuid,
    pub head: u8,
    /// None for the node's own name or a head's label.
    pub port: Option<u8>,
    /// The parameter key the port is in the show, for a port.
    pub key: Option<String>,
    /// What it is called when nobody has named it.
    pub what: String,
    pub show: String,
    pub node: String,
    pub agreed: Option<String>,
}

/// What to do about one name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    /// Both say the same.
    Same,
    /// The show changed it: tell the node.
    Push,
    /// The show never had one and the node does: take it.
    Take,
    /// The node changed it, or both did: ask.
    Ask,
}

/// The whole rule, on three strings.
pub(super) fn verdict(show: &str, node: &str, agreed: Option<&str>) -> Verdict {
    if show == node {
        return Verdict::Same;
    }
    match agreed {
        // The node still says what was agreed, so the show is what moved: renamed here
        // while the node was off, or put back by an undo.
        Some(agreed) if agreed == node => Verdict::Push,
        // The node moved, whether or not the show did too.
        Some(_) => Verdict::Ask,
        // Never agreed. A side with nothing to say has no opinion to ask about.
        None if node.is_empty() => Verdict::Push,
        None if show.is_empty() => Verdict::Take,
        None => Verdict::Ask,
    }
}

/// What a head's fixture is called when nobody has named the head: `<node> Strip 1`,
/// the shape adoption and `name_heads` give it.
fn unnamed_head(fixture_name: &str, head_name: &str) -> bool {
    fixture_name == head_name || fixture_name.ends_with(&format!(" {head_name}"))
}

/// What the module calls a head, which is what a head nobody named is called.
fn head_name(device: &DiscoveredDevice, head: u8) -> String {
    device
        .description
        .as_ref()
        .and_then(|d| d.head_name(head))
        .map(str::to_string)
        .unwrap_or_else(|| format!("Head {head}"))
}

/// Every name on one node, in the show and on the node. Empty for a node that has not
/// said what it is or what anything on it is called: there is nothing to compare yet.
pub(super) fn names_of(device: &DiscoveredDevice, fixtures: &[Fixture], types: &[FixtureType]) -> Vec<Named> {
    let (Some(description), Some(labels)) = (&device.description, &device.labels) else {
        return Vec::new();
    };
    let mut named = Vec::new();
    for fixture in fixtures.iter().filter(|f| f.address.serial() == Some(device.serial.as_str())) {
        let Some(head) = fixture.address.head() else { continue };
        let agreed = fixture.agreed_names.as_ref();
        let (what, show, node) = match head {
            0 => ("Name".to_string(), openhaunt::node_name(&fixture.name), device.name.clone()),
            h => {
                let what = head_name(device, h);
                let show = if unnamed_head(&fixture.name, &what) {
                    String::new()
                } else {
                    openhaunt::node_name(&fixture.name)
                };
                (what, show, labels.heads.get(&h).cloned().unwrap_or_default())
            }
        };
        named.push(Named {
            fixture: fixture.id,
            head,
            port: None,
            key: None,
            what,
            show,
            node,
            agreed: agreed.and_then(|a| a.name.clone()),
        });

        let Some(fixture_type) = types.iter().find(|t| t.id == fixture.fixture_type_id) else { continue };
        for parameter in &fixture_type.parameters {
            let Some(port) = parameter.binding.and_then(|b| b.port()) else { continue };
            let key = parameter_key(&parameter.kind);
            let what = description
                .ports
                .iter()
                .find(|p| p.port == port)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| key.clone());
            named.push(Named {
                fixture: fixture.id,
                head,
                port: Some(port),
                show: fixture.labels().get(&key).map(|l| openhaunt::node_name(l)).unwrap_or_default(),
                node: labels.ports.get(&port).cloned().unwrap_or_default(),
                agreed: agreed.and_then(|a| a.parameters.get(&key).cloned()),
                key: Some(key),
                what,
            });
        }
    }
    named
}

/// What a node is told to call itself and its ports. Only what changes.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Telling {
    pub name: Option<String>,
    pub ports: BTreeMap<u8, String>,
    pub heads: BTreeMap<u8, String>,
}

impl Telling {
    fn add(&mut self, item: &Named, value: &str) {
        match (item.head, item.port) {
            (0, None) => self.name = Some(value.to_string()),
            (head, None) => {
                self.heads.insert(head, value.to_string());
            }
            (_, Some(port)) => {
                self.ports.insert(port, value.to_string());
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.ports.is_empty() && self.heads.is_empty()
    }

    /// The `POST /api/v1/config` body. `labels` is left out where only the name moves,
    /// so a node from before labels is asked nothing it does not know.
    pub fn body(&self) -> serde_json::Value {
        let mut body = serde_json::json!({});
        if let Some(name) = &self.name {
            body["name"] = serde_json::json!(name);
        }
        if !self.ports.is_empty() || !self.heads.is_empty() {
            let numbered = |m: &BTreeMap<u8, String>| -> serde_json::Map<String, serde_json::Value> {
                m.iter().map(|(n, l)| (n.to_string(), serde_json::json!(l))).collect()
            };
            body["labels"] = serde_json::json!({ "ports": numbered(&self.ports), "heads": numbered(&self.heads) });
        }
        body
    }
}

/// The names a node is adopted with, as the adoption dialog sends them: the node's,
/// and the labels by head and port number. Anything left out stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct AdoptNames {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub heads: BTreeMap<u8, String>,
    #[serde(default)]
    pub ports: BTreeMap<u8, String>,
}

/// One parameter of the show that a replacement node does not have.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Missing {
    pub fixture: String,
    pub head: u8,
    pub key: String,
    /// What the operator calls it: its label, or what the node called it.
    pub label: String,
}

/// What changes when one node stands in for another. See [`DeviceManager::compare`].
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// The two are different modules.
    pub module_differs: bool,
    /// The old node's module name, for saying so.
    pub was: String,
    pub is: String,
    pub missing: Vec<Missing>,
}

impl DeviceManager {
    /// Bring a node and the show to the same names: tell the node what the show
    /// changed, take what only the node has, and list what the operator has to decide.
    ///
    /// The leader's alone, and only for a node that is up and has said what it is.
    pub(super) async fn sync_names(&mut self, serial: &str) {
        if self.is_follower().await {
            return;
        }
        // Twice at most: once to compare, and once more after telling the node, against
        // what it says it took.
        for _ in 0..2 {
            let Some(device) = self.state.discovered.get(serial).cloned() else { return };
            if !device.online || device.adopted_fixture_id.is_none() {
                return;
            }
            let fixtures = self.fixtures().await;
            let types: Vec<FixtureType> = self.read("fixture_types").await;
            let items = names_of(&device, &fixtures, &types);
            if items.is_empty() {
                return;
            }

            let mut telling = Telling::default();
            let mut conflicts = Vec::new();
            let mut agreed: BTreeMap<Uuid, AgreedNames> = BTreeMap::new();
            let mut taken: Vec<&Named> = Vec::new();
            for item in &items {
                let agree = |agreed: &mut BTreeMap<Uuid, AgreedNames>, value: &str| {
                    let stored = fixtures.iter().find(|f| f.id == item.fixture).and_then(|f| f.agreed_names.clone());
                    let entry = agreed.entry(item.fixture).or_insert_with(|| stored.unwrap_or_default());
                    match &item.key {
                        Some(key) => {
                            entry.parameters.insert(key.clone(), value.to_string());
                        }
                        None => entry.name = Some(value.to_string()),
                    }
                };
                match verdict(&item.show, &item.node, item.agreed.as_deref()) {
                    Verdict::Same => {
                        if item.agreed.as_deref() != Some(item.show.as_str()) {
                            agree(&mut agreed, &item.show);
                        }
                    }
                    Verdict::Push => telling.add(item, &item.show),
                    Verdict::Take => {
                        taken.push(item);
                        agree(&mut agreed, &item.node);
                    }
                    Verdict::Ask => conflicts.push(NameConflict {
                        head: item.head,
                        port: item.port,
                        what: item.what.clone(),
                        node: item.node.clone(),
                        show: item.show.clone(),
                    }),
                }
            }

            for item in taken {
                info!("[devices] {serial} calls {} \"{}\"; the show takes it", item.what, item.node);
                self.call_in_show(&device, &fixtures, item, &item.node).await;
            }
            for (fixture, names) in agreed {
                if let Err(e) = self.write(field(fixture, "agreed_names"), &Some(names)).await {
                    debug!("[devices] could not note the names agreed with {serial}: {e}");
                }
            }
            if let Some(device) = self.state.discovered.get_mut(serial) {
                if device.name_conflicts != conflicts {
                    if !conflicts.is_empty() {
                        warn!("[devices] {serial} and the show call {} thing(s) differently", conflicts.len());
                    }
                    device.name_conflicts = conflicts;
                    self.publish().await;
                }
            }

            if telling.is_empty() {
                return;
            }
            let body = telling.body();
            // Told once. Firmware from before labels takes the body and keeps none of
            // it, and the next comparison would tell it again for ever.
            if self.names_told.get(serial) == Some(&body) {
                return;
            }
            info!("[devices] telling {serial} what the show calls things: {body}");
            if let Err(e) = self.post(serial, "config", body.clone()).await {
                debug!("[devices] could not rename on {serial}: {e}");
                return;
            }
            self.names_told.insert(serial.to_string(), body);
            // Shown at once rather than when the node's TXT record next resolves.
            if let (Some(name), Some(device)) = (&telling.name, self.state.discovered.get_mut(serial)) {
                device.name = name.clone();
            }
            // What it kept, which is what it is compared with next.
            self.refresh_description(serial).await;
            self.publish().await;
        }
    }

    /// Put a name into the show: the node's own fixture, a head's, or a parameter's label.
    async fn call_in_show(&self, device: &DiscoveredDevice, fixtures: &[Fixture], item: &Named, value: &str) {
        let Some(fixture) = fixtures.iter().find(|f| f.id == item.fixture) else { return };
        let result = match (&item.key, item.head) {
            (Some(key), _) => {
                // Read again rather than from `fixtures`: the whole map is written, and a
                // caller naming several ports in a row holds a copy from before the first.
                // Writing each from that copy kept only the last (the four labels named on
                // f674a554's page, 2026-11-02: the show took one and then cleared three).
                let mut labels = self
                    .fixtures()
                    .await
                    .into_iter()
                    .find(|f| f.id == fixture.id)
                    .map(|f| f.labels())
                    .unwrap_or_default();
                if value.is_empty() {
                    labels.remove(key);
                } else {
                    labels.insert(key.clone(), value.to_string());
                }
                self.write(field(fixture.id, "parameter_labels"), &Some(labels)).await
            }
            (None, 0) if !value.is_empty() => self.write(field(fixture.id, "name"), &value).await,
            (None, 0) => Ok(()),
            (None, head) => {
                // A head nobody named is called after its node, as adoption calls it.
                let name = match value {
                    "" => {
                        let node = fixtures
                            .iter()
                            .find(|f| f.address.serial() == Some(device.serial.as_str()) && f.address.head() == Some(0))
                            .map_or(device.name.as_str(), |f| f.name.as_str());
                        format!("{node} {}", head_name(device, head))
                    }
                    named => named.to_string(),
                };
                self.write(field(fixture.id, "name"), &name).await
            }
        };
        if let Err(e) = result {
            debug!("[devices] could not rename {} on {}: {e}", item.what, device.serial);
        }
    }

    /// Settle what the operator picked, then compare again. Taking the node's writes it
    /// into the show; taking the show's notes the node's as agreed, which makes the
    /// show's a change to pass on.
    pub(super) async fn settle_names(&mut self, serial: &str, choices: &[NameChoice]) -> Result<(), String> {
        if self.is_follower().await {
            return Err("only the node leading the session renames devices".into());
        }
        let device = self.state.discovered.get(serial).cloned().ok_or_else(|| format!("no device with serial {serial}"))?;
        if !device.online {
            return Err(format!("{} is offline", device.name));
        }
        let fixtures = self.fixtures().await;
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        let items = names_of(&device, &fixtures, &types);
        let mut agreed: BTreeMap<Uuid, AgreedNames> = BTreeMap::new();
        for choice in choices {
            let Some(item) = items.iter().find(|i| i.head == choice.head && i.port == choice.port) else { continue };
            if choice.take == NameSide::Node {
                self.call_in_show(&device, &fixtures, item, &item.node).await;
            }
            let stored = fixtures.iter().find(|f| f.id == item.fixture).and_then(|f| f.agreed_names.clone());
            let entry = agreed.entry(item.fixture).or_insert_with(|| stored.unwrap_or_default());
            match &item.key {
                Some(key) => {
                    entry.parameters.insert(key.clone(), item.node.clone());
                }
                None => entry.name = Some(item.node.clone()),
            }
        }
        for (fixture, names) in agreed {
            self.write(field(fixture, "agreed_names"), &Some(names)).await?;
        }
        // Told again even if this exact body was told before: somebody asked.
        self.names_told.remove(serial);
        self.sync_names(serial).await;
        Ok(())
    }

    /// Note everything the node says now as agreed, so that whatever the show says
    /// differently is passed on to it. What adoption with names and replacing a node
    /// both mean: the operator has just said what things are called.
    async fn agree_with_node(&self, serial: &str) {
        let Some(device) = self.state.discovered.get(serial) else { return };
        let fixtures = self.fixtures().await;
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        let mut agreed: BTreeMap<Uuid, AgreedNames> = BTreeMap::new();
        for item in names_of(device, &fixtures, &types) {
            let entry = agreed.entry(item.fixture).or_default();
            match item.key {
                Some(key) => {
                    entry.parameters.insert(key, item.node);
                }
                None => entry.name = Some(item.node),
            }
        }
        for (fixture, names) in agreed {
            if let Err(e) = self.write(field(fixture, "agreed_names"), &Some(names)).await {
                debug!("[devices] could not note {serial}'s names: {e}");
            }
        }
    }

    /// Write the names the adoption dialog gave into the show, in node terms: the
    /// node's, each head's, and each port's by number.
    async fn name_in_show(&self, serial: &str, names: &AdoptNames) {
        let Some(device) = self.state.discovered.get(serial).cloned() else { return };
        let fixtures = self.fixtures().await;
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        for item in names_of(&device, &fixtures, &types) {
            let given = match (item.head, item.port) {
                (0, None) => names.name.as_deref(),
                (head, None) => names.heads.get(&head).map(String::as_str),
                (_, Some(port)) => names.ports.get(&port).map(String::as_str),
            };
            let Some(given) = given.map(openhaunt::node_name) else { continue };
            if given != item.show {
                self.call_in_show(&device, &fixtures, &item, &given).await;
            }
        }
    }

    /// Adopt a node, perhaps in place of one that is gone, and call things what the
    /// operator said.
    pub(super) async fn adopt_named(
        &mut self,
        serial: &str,
        replaces: Option<&str>,
        names: Option<AdoptNames>,
    ) -> Result<Uuid, String> {
        let id = match replaces {
            Some(old) => self.replace(old, serial).await?,
            None => self.adopt(serial).await?,
        };
        // `adopt` and `replace` both read the description; the labels came with it.
        if names.is_some() || replaces.is_some() {
            self.agree_with_node(serial).await;
        }
        if let Some(names) = &names {
            self.name_in_show(serial, names).await;
        }
        self.names_told.remove(serial);
        self.sync_names(serial).await;
        self.publish().await;
        Ok(id)
    }

    /// Put a new node where an old one was: every fixture patched to `old` is patched
    /// to `new` instead, keeping its id — and so every cue, flow, group and layout that
    /// names it. A head the new node has and the old one did not gets a fixture of its
    /// own, as adoption would give it.
    async fn replace(&mut self, old: &str, new: &str) -> Result<Uuid, String> {
        if self.is_follower().await {
            return Err("only the node leading the session adopts devices".into());
        }
        if old == new {
            return Err("a node cannot replace itself".into());
        }
        if self.state.discovered.get(old).is_some_and(|d| d.online) {
            return Err(format!("{old} is still online; only a node that is gone can be replaced"));
        }
        let device = self.state.discovered.get(new).cloned().ok_or_else(|| format!("no device with serial {new}"))?;
        if device.adopted_fixture_id.is_some() {
            return Err(format!("{} is adopted already", device.name));
        }
        let description = device
            .description
            .clone()
            .filter(|d| !d.is_empty())
            .ok_or_else(|| format!("{new} does not describe its ports; cannot adopt"))?;

        let fixtures = self.fixtures().await;
        let patched: Vec<&Fixture> = fixtures.iter().filter(|f| f.address.serial() == Some(old)).collect();
        if !patched.iter().any(|f| f.address.head() == Some(0)) {
            return Err(format!("nothing is patched to {old}"));
        }
        let mut own = None;
        let mut heads = std::collections::BTreeSet::new();
        for fixture in &patched {
            let mut moved = (*fixture).clone();
            let FixtureAddress::OpenHaunt { serial, universe, head, .. } = &mut moved.address else { continue };
            *serial = new.to_string();
            let h = head.unwrap_or(0);
            heads.insert(h);
            if h == 0 {
                *universe = match description.dmx {
                    Some(_) => match *universe {
                        Some(u) => Some(u),
                        None => Some(self.next_free_universe().await),
                    },
                    None => None,
                };
                own = Some(moved.id);
            }
            self.write(replace_path("fixtures", moved.id), &moved).await?;
        }
        for head in description.head_numbers().into_iter().filter(|h| !heads.contains(h)) {
            let fixture = self.head_fixture(new, &device, &description, head, None).await?;
            self.write(create_path("fixtures"), &fixture).await?;
        }
        // The old node is not coming back as itself.
        self.state.discovered.remove(old);
        self.names_told.remove(old);
        info!("[devices] {new} replaces {old}");

        self.reconcile_adoptions().await;
        self.retype(new).await;
        self.reconcile_strips(new).await;
        let was_driving = self.mqtt.is_some();
        self.reconsider_leadership().await;
        if was_driving {
            self.push_config(new).await;
        }
        own.ok_or_else(|| format!("{old} has no head 0"))
    }

    /// What replacing `old` with `new` would leave without a port: every parameter of
    /// the old node's fixtures the new node's description does not have. A strip is left
    /// out, because what a strip is follows its mode and is set again either way.
    pub(super) async fn compare(&self, new: &str, old: &str) -> Result<Comparison, String> {
        let device = self.state.discovered.get(new).ok_or_else(|| format!("no device with serial {new}"))?;
        let description = device.description.as_ref().ok_or_else(|| format!("{new} has not said what it is"))?;
        let effects = device.pixels.as_ref().map(|p| p.effects.clone()).unwrap_or_default();
        let fixtures = self.fixtures().await;
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        let mut comparison = Comparison {
            is: device.module_name.clone(),
            was: self.state.discovered.get(old).map(|d| d.module_name.clone()).unwrap_or_default(),
            ..Comparison::default()
        };
        for fixture in fixtures.iter().filter(|f| f.address.serial() == Some(old)) {
            let Some(head) = fixture.address.head() else { continue };
            if fixture.address.strip().is_some() {
                continue;
            }
            let Some(was) = types.iter().find(|t| t.id == fixture.fixture_type_id) else { continue };
            if head == 0 {
                comparison.was = was.name.clone();
            }
            let is = openhaunt::head_type_from(device.module_type, &device.module_name, description, head, &effects);
            let keys: std::collections::BTreeSet<String> = is.parameters.iter().map(|p| parameter_key(&p.kind)).collect();
            for parameter in &was.parameters {
                let key = parameter_key(&parameter.kind);
                if !keys.contains(&key) {
                    comparison.missing.push(Missing {
                        fixture: fixture.name.clone(),
                        head,
                        label: fixture.parameter_label(&parameter.kind).map(str::to_string).unwrap_or_else(|| key.clone()),
                        key,
                    });
                }
            }
        }
        comparison.module_differs = !comparison.was.is_empty() && comparison.was != comparison.is;
        Ok(comparison)
    }
}

/// `fixtures/<id>/<field>`.
fn field(fixture: Uuid, name: &str) -> pult_schema::path::Path {
    vec![PathSegment::Key("fixtures".into()), PathSegment::Id(fixture), PathSegment::Key(name.into())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pult_schema::types::openhaunt::NodeLabels;

    #[test]
    fn the_same_name_is_nothing_to_do() {
        assert_eq!(verdict("Tor", "Tor", None), Verdict::Same);
        assert_eq!(verdict("", "", Some("x")), Verdict::Same);
    }

    #[test]
    fn a_name_changed_only_in_the_show_is_passed_on() {
        assert_eq!(verdict("Relay Horn", "", Some("")), Verdict::Push);
        assert_eq!(verdict("", "Relay Horn", Some("Relay Horn")), Verdict::Push, "an undo");
    }

    #[test]
    fn a_name_the_node_changed_is_asked_about() {
        assert_eq!(verdict("Tor", "Gate", Some("Tor")), Verdict::Ask, "renamed on the node's page");
        assert_eq!(verdict("Tor", "", Some("Tor")), Verdict::Ask, "the node was reset");
        assert_eq!(verdict("Horn", "Gate", Some("Tor")), Verdict::Ask, "both moved");
    }

    #[test]
    fn with_nothing_agreed_only_two_opinions_are_asked_about() {
        assert_eq!(verdict("Tor", "", None), Verdict::Push);
        assert_eq!(verdict("", "Tor", None), Verdict::Take);
        assert_eq!(verdict("Tor", "Gate", None), Verdict::Ask);
    }

    #[test]
    fn a_body_names_only_what_changes() {
        let mut telling = Telling::default();
        assert!(telling.is_empty());
        telling.name = Some("Hof".into());
        assert_eq!(telling.body(), serde_json::json!({ "name": "Hof" }), "no labels for a node that may not know them");
        telling.ports.insert(4, "Ausgang PIR".into());
        telling.heads.insert(1, String::new());
        assert_eq!(
            telling.body(),
            serde_json::json!({ "name": "Hof", "labels": { "ports": { "4": "Ausgang PIR" }, "heads": { "1": "" } } })
        );
    }

    #[test]
    fn a_strip_still_called_after_its_node_is_unnamed() {
        assert!(unnamed_head("Hof Strip 1", "Strip 1"));
        assert!(unnamed_head("Strip 1", "Strip 1"));
        assert!(!unnamed_head("LED Streifen Tor", "Strip 1"));
    }

    #[test]
    fn labels_are_read_from_the_raw_body() {
        let info = serde_json::json!({
            "ports": [
                { "port": 4, "name": "Sensor 1", "label": "Ausgang PIR" },
                { "port": 5, "name": "Sensor 2", "label": "  " },
                { "port": 0, "name": "Relay 1" },
            ],
            "heads": [{ "head": 1, "name": "Strip 1", "label": "LED Streifen Tor" }],
        });
        let labels = NodeLabels::from_info(&info);
        assert_eq!(labels.ports, BTreeMap::from([(4, "Ausgang PIR".to_string())]));
        assert_eq!(labels.heads, BTreeMap::from([(1, "LED Streifen Tor".to_string())]));
    }
}
