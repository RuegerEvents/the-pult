use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::effect::{RunningEffect, RunningFade};
use super::programmer::ProgrammerValue;
use crate::PultSchema;

/// A point in the rig, in metres, from whatever origin the show uses.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Where a fixture is, and for a moving one, where it points.
///
/// The spec asks for positions to be either positional (XYZ) or axial (a position
/// and a direction vector). Nothing forces a position to be accurate: a rig can be
/// laid out roughly and corrected later, or updated from tracking data.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FixturePosition {
    /// Just where it hangs.
    Point(Vec3),
    /// Where it hangs and the direction it faces at rest.
    Axial { position: Vec3, direction: Vec3 },
}

impl FixturePosition {
    pub fn position(&self) -> Vec3 {
        match self {
            FixturePosition::Point(p) => *p,
            FixturePosition::Axial { position, .. } => *position,
        }
    }
}

/// How output reaches a fixture.
///
/// Not every fixture is on a DMX line. An OpenHaunt node is addressed by the serial
/// of the node it lives on, and only a DMX gateway module also carries a universe —
/// for a relay or a sensor there is no universe to speak of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FixtureAddress {
    Dmx { universe: u16, address: u16 },
    OpenHaunt { serial: String, universe: Option<u16> },
}

impl Default for FixtureAddress {
    fn default() -> Self {
        FixtureAddress::Dmx { universe: 1, address: 1 }
    }
}

impl FixtureAddress {
    /// Universe and start address, for the fixtures that have them.
    pub fn dmx(&self) -> Option<(u16, u16)> {
        match self {
            FixtureAddress::Dmx { universe, address } => Some((*universe, *address)),
            // A gateway module carries a universe but its own address is the node,
            // not a slot in that universe: it owns all 512 channels.
            FixtureAddress::OpenHaunt { .. } => None,
        }
    }

    /// The node serial, for fixtures that live on one.
    pub fn serial(&self) -> Option<&str> {
        match self {
            FixtureAddress::OpenHaunt { serial, .. } => Some(serial),
            FixtureAddress::Dmx { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterKind {
    Intensity,
    ColorRgb,
    Pan,
    Tilt,
    GoboIndex,
    Raw(u8),
    /// A switched output: a relay, a dry contact the console closes.
    Switch(u8),
    /// A switch or button the console reads.
    Contact(u8),
    Temperature,
    Humidity,
    AirQuality,
    /// A line of text, for a display module.
    Text,
    /// Something a device declared that this console has no name for.
    ///
    /// A node describes its own ports, and it is allowed to describe one the
    /// console has never heard of. The name it gave is the whole identity: it is
    /// what the operator sees and what the parameter key is built from.
    Named(String),
}

// What a parameter can be, and the arithmetic over it, live in `pult-render`: the
// browser evaluates values now and cannot depend on this crate. Re-exported under the
// path it has always had.
pub use pult_render::value::ParameterValue;

/// Which way a parameter flows.
///
/// Everything the console has driven so far is an output. A sensor node reverses
/// that: the device writes the value and the show reads it, which is what lets a
/// contact closure be an ordinary fixture parameter rather than a separate concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterDirection {
    #[default]
    Output,
    Input,
}

/// Where a parameter sits on the thing that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ParameterBinding {
    /// An offset from the fixture's DMX start address, 1-based.
    Dmx { channel: u8 },
    /// A port on an I/O module, 0-based, as the module numbers its own terminals.
    Port { index: u8 },
}

impl ParameterBinding {
    pub fn dmx_channel(&self) -> Option<u8> {
        match self {
            ParameterBinding::Dmx { channel } => Some(*channel),
            ParameterBinding::Port { .. } => None,
        }
    }

    pub fn port(&self) -> Option<u8> {
        match self {
            ParameterBinding::Port { index } => Some(*index),
            ParameterBinding::Dmx { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ParameterDefinition {
    pub kind: ParameterKind,
    #[serde(default)]
    pub direction: ParameterDirection,
    pub binding: ParameterBinding,
    pub default_value: ParameterValue,
}

/// `fixture_types.parameters` is one JSON column, so a showfile written before
/// bindings existed holds `dmx_channel` where `binding` now goes. Reading it back
/// through this shape is the whole migration for that field — there is no column to
/// alter, and a show that has never been reopened stays readable.
///
/// Written out by hand rather than through `#[serde(from = ...)]`, because that
/// attribute is one more thing for ts-rs to fail to parse and warn about.
#[derive(Deserialize)]
struct ParameterDefinitionWire {
    kind: ParameterKind,
    #[serde(default)]
    direction: ParameterDirection,
    binding: Option<ParameterBinding>,
    dmx_channel: Option<u8>,
    default_value: ParameterValue,
}

impl<'de> Deserialize<'de> for ParameterDefinition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ParameterDefinitionWire::deserialize(deserializer)?;
        let binding = wire
            .binding
            .or_else(|| wire.dmx_channel.map(|channel| ParameterBinding::Dmx { channel }))
            .unwrap_or(ParameterBinding::Dmx { channel: 1 });
        Ok(ParameterDefinition {
            kind: wire.kind,
            direction: wire.direction,
            binding,
            default_value: wire.default_value,
        })
    }
}

/// Template describing what parameters a fixture type has.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "fixture_types")]
pub struct FixtureType {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub manufacturer: String,
    #[pult(lifecycle = PERSISTED)]
    pub channel_count: u16,
    #[pult(lifecycle = PERSISTED)]
    pub parameters: Vec<ParameterDefinition>,
}

/// A patched fixture instance — a specific unit in the rig.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "fixtures")]
pub struct Fixture {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub fixture_type_id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub address: FixtureAddress,
    /// Where this fixture is in the rig. None until it has been placed.
    #[pult(lifecycle = PERSISTED)]
    pub position: Option<FixturePosition>,
    /// What this fixture's devices have *told* the console, keyed by parameter key:
    /// a contact closure, a temperature, a humidity — anything a device reports.
    ///
    /// The one kind of value the console still stores, and the reason it does is that
    /// it cannot work it out. What a fixture is being *driven* to is a function of the
    /// fades and effects on it, the programmer over them and its home value beneath,
    /// evaluated for whatever moment a consumer asks about; what it is *sensing*
    /// arrived off a wire and is not a function of anything the console holds.
    ///
    /// SYNCED, and for a reason that survived the removal of its driven half: nothing
    /// else on the network can work this value out, because the wire it came off is
    /// attached to this station.
    ///
    /// Defaulted on the wire so a showfile or a peer from before the split — where one
    /// map carried both halves under the name `live_values` — opens with this field
    /// simply absent. The old name is *ignored* rather than aliased onto this one: what
    /// it mostly carried was driven values, and those are now a function of what is
    /// driving them. Taking them for readings would be filing the console's own output
    /// as something a device reported.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub sensed_values: HashMap<String, ParameterValue>,
    /// What shape is driving each of this fixture's parameters, keyed by parameter
    /// key.
    ///
    /// Together with `live_fades` this is *what the fixture is doing*, and the whole of
    /// it: nothing stores the values these come to. A shape and its anchor are enough
    /// for anybody holding the row to work out what the parameter is at any moment they
    /// care about, which is what lets an output connector run at its protocol's rate, a
    /// node trace the shape itself, and a browser draw it at its own refresh.
    ///
    /// LOCAL rather than SYNCED. Every station works these out for itself from
    /// replicated cue state, so broadcasting them would be sending each console a
    /// slower copy of what it has already computed.
    ///
    /// Defaulted on the wire: a LOCAL field is worked out by the station that holds
    /// it, so a client creating a fixture has nothing to say about it and should not
    /// have to name it. Without this, `fixtures/__create` refuses a body that is
    /// otherwise complete because it omitted a field only the engine can fill.
    #[serde(default)]
    #[pult(lifecycle = LOCAL)]
    pub live_effects: HashMap<String, RunningEffect>,
    /// The fades on each of this fixture's parameters, keyed by parameter key. LOCAL
    /// and defaulted for the same reasons as above.
    ///
    /// **Including the ones that have arrived**, which is what makes this the record of
    /// where the rig is rather than a list of what is in flight. A landed fade is a
    /// constant function of time, and since nothing stores the number it landed on, it
    /// is the only thing that remembers it. One entry per parameter anything has ever
    /// driven, replaced when a cue, a release or an action takes the key.
    #[serde(default)]
    #[pult(lifecycle = LOCAL)]
    pub live_fades: HashMap<String, RunningFade>,
    /// What this fixture's parameters rest at when nothing is driving them, keyed by
    /// parameter key. Empty on nearly every fixture, and then the answer is
    /// whatever its type declares.
    ///
    /// On the fixture rather than on the type because a house light that comes up
    /// when nothing is controlling it is a fact about *this* rig, and because a type
    /// is derived: the node describes its ports again and the console rebuilds it,
    /// which would take an override with it. Defaulted on the wire so a show written
    /// before this existed opens with every fixture saying nothing.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub home_values: HashMap<String, ParameterValue>,
}

/// The map key for a parameter, in `home_values`, `sensed_values`, `live_fades` and
/// `live_effects` alike.
///
/// Here rather than in the backend because three places derive it — the engine, the
/// browser, and the command-line plugin — and a fourth spelling of it would be a
/// fixture whose values quietly land under a key nothing reads.
pub fn parameter_key(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Intensity => "Intensity".into(),
        ParameterKind::ColorRgb => "ColorRgb".into(),
        ParameterKind::Pan => "Pan".into(),
        ParameterKind::Tilt => "Tilt".into(),
        ParameterKind::GoboIndex => "GoboIndex".into(),
        ParameterKind::Raw(channel) => format!("Raw:{channel}"),
        ParameterKind::Switch(n) => format!("Switch:{n}"),
        ParameterKind::Contact(n) => format!("Contact:{n}"),
        ParameterKind::Temperature => "Temperature".into(),
        ParameterKind::Humidity => "Humidity".into(),
        ParameterKind::AirQuality => "AirQuality".into(),
        ParameterKind::Text => "Text".into(),
        ParameterKind::Named(name) => format!("Named:{name}"),
    }
}

/// What a parameter rests at when nothing is driving it.
///
/// The fixture's own override where it has one, and what its type declares
/// otherwise. `None` where the type has no such parameter, which is the only honest
/// answer: a fixture that cannot pan has nowhere for a pan to rest.
///
/// One resolution, in the schema, because the engine resolving a relative write, the
/// engine sending a selection home and playback letting go of a key all have to
/// agree about it — and because `default_value` is what a *device* said, which is
/// the answer only until somebody overrides it.
pub fn home_value(
    fixture: &Fixture,
    fixture_type: &FixtureType,
    kind: &ParameterKind,
) -> Option<ParameterValue> {
    home_value_by_key(fixture, Some(fixture_type), &parameter_key(kind))
}

/// The same question, asked with the key instead of the kind.
///
/// Everything downstream of the engine already holds the key — a running fade, a held
/// programmer entry — and going back to a kind to come forward to the same string
/// again would be a second place for the two to disagree. So this is the resolution
/// and [`home_value`] is the spelling of it that starts from a kind.
///
/// The type is optional because an override does not need it: where a fixture is
/// patched to a type this station has not received yet, its own answer is still its
/// own answer, and a house light should not go dark waiting for a row to replicate.
pub fn home_value_by_key(
    fixture: &Fixture,
    fixture_type: Option<&FixtureType>,
    key: &str,
) -> Option<ParameterValue> {
    home_value_ref_by_key(fixture, fixture_type, key).cloned()
}

/// The same answer, borrowed rather than cloned.
///
/// What [`driving`] needs: a parameter's home value is looked up once per parameter
/// per output frame, and cloning a colour at that rate to hand it straight to an
/// evaluator that only reads it is the sort of cost that becomes the whole frame on a
/// rig of thousands.
pub fn home_value_ref_by_key<'a>(
    fixture: &'a Fixture,
    fixture_type: Option<&'a FixtureType>,
    key: &str,
) -> Option<&'a ParameterValue> {
    if let Some(overridden) = fixture.home_values.get(key) {
        return Some(overridden);
    }
    fixture_type?
        .parameters
        .iter()
        .find(|p| parameter_key(&p.kind) == key)
        .map(|p| &p.default_value)
}

// ── What is driving a parameter ──────────────────────────────────────

/// The programmer's entries, indexed by the parameter each one holds.
///
/// Built once by whoever is about to evaluate a rig. The alternative — scanning the
/// entries per parameter — is what makes a frame quadratic in the size of a look, and
/// a busy programmer is thousands of entries.
#[derive(Default)]
pub struct HeldByProgrammer<'a>(HashMap<(Uuid, String), &'a ProgrammerValue>);

impl<'a> HeldByProgrammer<'a> {
    pub fn of(entries: &'a [ProgrammerValue]) -> Self {
        Self(
            entries
                .iter()
                .map(|entry| ((entry.fixture_id, parameter_key(&entry.parameter_kind)), entry))
                .collect(),
        )
    }

    pub fn get(&self, fixture_id: Uuid, key: &str) -> Option<&'a ProgrammerValue> {
        self.0.get(&(fixture_id, key.to_string())).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Everything acting on one parameter, gathered from the show.
///
/// The station publishes the winner of playback's two layers into `live_effects` and
/// `live_fades`, so those two are read straight off the fixture. The programmer is
/// read from the SYNCED collection instead, because it is show data rather than
/// something a station worked out — and only where it holds a plain value: an entry
/// carrying a shape has already been resolved against its speed master and published
/// as the running effect, which is what keeps rate-following out of the evaluator.
pub fn driving<'a>(
    fixture: &'a Fixture,
    fixture_type: Option<&'a FixtureType>,
    held: Option<&'a ProgrammerValue>,
    key: &str,
) -> pult_render::Driving<'a> {
    pult_render::Driving {
        programmer: held.filter(|entry| entry.effect.is_none()).map(|entry| &entry.value),
        effect: fixture.live_effects.get(key),
        fade: fixture.live_fades.get(key),
        home: home_value_ref_by_key(fixture, fixture_type, key),
    }
}

/// What one parameter of one fixture is putting out at `now_ms`.
///
/// The whole of "what is this light doing" in one call, for the callers that want an
/// answer rather than the layers behind it.
pub fn value_at(
    fixture: &Fixture,
    fixture_type: Option<&FixtureType>,
    held: Option<&ProgrammerValue>,
    key: &str,
    now_ms: u64,
) -> Option<ParameterValue> {
    pult_render::value_at(&driving(fixture, fixture_type, held, key), now_ms)
}

/// The parameters of a type an operator can set, in the order it lists them.
///
/// Inputs are left out: a contact closure is a parameter a device writes and the
/// show reads, and there is nothing to send home.
pub fn output_parameters(fixture_type: &FixtureType) -> impl Iterator<Item = &ParameterDefinition> {
    fixture_type.parameters.iter().filter(|p| p.direction == ParameterDirection::Output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parameter_written_before_bindings_existed_still_loads() {
        let legacy = serde_json::json!({
            "kind": "Intensity",
            "dmx_channel": 4,
            "default_value": { "type": "Float", "value": 0.0 }
        });

        let parsed: ParameterDefinition = serde_json::from_value(legacy).unwrap();

        assert_eq!(parsed.binding, ParameterBinding::Dmx { channel: 4 });
        assert_eq!(parsed.direction, ParameterDirection::Output);
    }

    #[test]
    fn a_parameter_round_trips_through_its_current_shape() {
        let definition = ParameterDefinition {
            kind: ParameterKind::Contact(3),
            direction: ParameterDirection::Input,
            binding: ParameterBinding::Port { index: 3 },
            default_value: ParameterValue::Bool(false),
        };

        let json = serde_json::to_value(&definition).unwrap();
        let back: ParameterDefinition = serde_json::from_value(json).unwrap();

        assert_eq!(back, definition);
    }

    fn a_type(parameters: Vec<ParameterDefinition>) -> FixtureType {
        FixtureType {
            id: Uuid::nil(),
            name: "Par".into(),
            manufacturer: "Nobody".into(),
            channel_count: 1,
            parameters,
        }
    }

    fn a_fixture(home_values: HashMap<String, ParameterValue>) -> Fixture {
        Fixture {
            id: Uuid::nil(),
            name: "House left".into(),
            fixture_type_id: Uuid::nil(),
            address: FixtureAddress::default(),
            position: None,
            sensed_values: HashMap::new(),
            live_effects: HashMap::new(),
            live_fades: HashMap::new(),
            home_values,
        }
    }

    fn an_intensity(default_value: ParameterValue) -> ParameterDefinition {
        ParameterDefinition {
            kind: ParameterKind::Intensity,
            direction: ParameterDirection::Output,
            binding: ParameterBinding::Dmx { channel: 1 },
            default_value,
        }
    }

    /// `home_values` is a column that did not exist, so a fixture written before it
    /// has to read back as one with nothing to say rather than as a parse failure.
    #[test]
    fn a_fixture_written_before_home_values_existed_still_loads() {
        let legacy = serde_json::json!({
            "id": Uuid::nil(),
            "name": "House left",
            "fixture_type_id": Uuid::nil(),
            "address": { "Dmx": { "universe": 1, "address": 1 } },
            "position": null,
            "live_values": {},
        });

        let parsed: Fixture = serde_json::from_value(legacy).unwrap();

        assert!(parsed.home_values.is_empty(), "nothing to say, rather than nothing at all");
    }

    /// A row from before values stopped being stored, opened here.
    ///
    /// `live_values` is *ignored* rather than read into `sensed_values`. Most of what
    /// it carried was the console's own output, which is now a function of what is
    /// driving the parameter; filing that as something a device reported would be a
    /// station claiming to have been told what it had in fact decided.
    #[test]
    fn a_fixture_written_before_values_stopped_being_stored_still_loads() {
        let legacy = serde_json::json!({
            "id": Uuid::nil(),
            "name": "House left",
            "fixture_type_id": Uuid::nil(),
            "address": { "Dmx": { "universe": 1, "address": 1 } },
            "position": null,
            "live_values": {
                "Intensity": { "type": "Float", "value": 0.8 },
                "Contact:0": { "type": "Bool", "value": true },
            },
            "home_values": { "Intensity": { "type": "Float", "value": 1.0 } },
        });

        let parsed: Fixture = serde_json::from_value(legacy).expect("an older row still reads");

        assert!(parsed.sensed_values.is_empty(), "the old map is ignored, not adopted");
        assert!(parsed.live_fades.is_empty());
        assert!(parsed.live_effects.is_empty());
        // And nothing else was lost getting there — the fields that outlived the
        // change arrive as they were written.
        assert_eq!(parsed.name, "House left");
        assert_eq!(parsed.home_values.get("Intensity"), Some(&ParameterValue::Float(1.0)));
    }

    /// And the other direction: a station on an older build receiving a row from this
    /// one, which simply does not carry the field it is looking for.
    #[test]
    fn a_row_from_here_omits_the_field_an_older_station_looks_for() {
        let fixture = a_fixture(HashMap::new());
        let written = serde_json::to_value(&fixture).expect("a fixture serialises");

        assert!(written.get("live_values").is_none(), "there is no such field any more");
        assert!(written.get("sensed_values").is_some(), "and what replaced half of it is there");
        // An older build defaults what it cannot find, which is the whole of what it
        // has to do: `live_values` was `#[serde(default)]`-adjacent there too.
        assert_eq!(written["sensed_values"], serde_json::json!({}));
    }

    #[test]
    fn a_fixture_with_no_override_rests_where_its_type_says() {
        let fixture_type = a_type(vec![an_intensity(ParameterValue::Float(0.0))]);
        let fixture = a_fixture(HashMap::new());

        assert_eq!(
            home_value(&fixture, &fixture_type, &ParameterKind::Intensity),
            Some(ParameterValue::Float(0.0))
        );
    }

    /// The case that forces the override to exist: a house light is on when nothing
    /// is controlling it, and its type — derived from what the node said about its
    /// ports — has no way to know that.
    #[test]
    fn a_fixture_with_an_override_rests_there_instead() {
        let fixture_type = a_type(vec![an_intensity(ParameterValue::Float(0.0))]);
        let fixture = a_fixture(HashMap::from([(
            "Intensity".to_string(),
            ParameterValue::Float(1.0),
        )]));

        assert_eq!(
            home_value(&fixture, &fixture_type, &ParameterKind::Intensity),
            Some(ParameterValue::Float(1.0))
        );
    }

    #[test]
    fn a_parameter_the_type_does_not_have_rests_nowhere() {
        let fixture_type = a_type(vec![an_intensity(ParameterValue::Float(0.0))]);
        let fixture = a_fixture(HashMap::new());

        assert_eq!(
            home_value(&fixture, &fixture_type, &ParameterKind::Pan),
            None,
            "a fixture that cannot pan has nowhere for a pan to rest"
        );
    }

    #[test]
    fn an_input_is_not_something_to_send_home() {
        let fixture_type = a_type(vec![
            an_intensity(ParameterValue::Float(0.0)),
            ParameterDefinition {
                kind: ParameterKind::Contact(0),
                direction: ParameterDirection::Input,
                binding: ParameterBinding::Port { index: 0 },
                default_value: ParameterValue::Bool(false),
            },
        ]);

        let kinds: Vec<&ParameterKind> =
            output_parameters(&fixture_type).map(|p| &p.kind).collect();

        assert_eq!(kinds, vec![&ParameterKind::Intensity]);
    }

    #[test]
    fn a_parameter_key_says_which_one_it_is() {
        assert_eq!(parameter_key(&ParameterKind::Intensity), "Intensity");
        assert_ne!(
            parameter_key(&ParameterKind::Raw(5)),
            parameter_key(&ParameterKind::Raw(6)),
            "two raw channels are two keys"
        );
        assert_eq!(parameter_key(&ParameterKind::Named("Fog".into())), "Named:Fog");
    }

    #[test]
    fn an_address_answers_only_for_the_kind_it_is() {
        let dmx = FixtureAddress::Dmx { universe: 2, address: 17 };
        assert_eq!(dmx.dmx(), Some((2, 17)));
        assert_eq!(dmx.serial(), None);

        let node = FixtureAddress::OpenHaunt { serial: "1a2b3c".into(), universe: Some(5) };
        assert_eq!(node.dmx(), None, "a node fixture has no slot in a universe");
        assert_eq!(node.serial(), Some("1a2b3c"));
    }

    #[test]
    fn a_nudge_moves_what_arithmetic_means_something_for() {
        assert_eq!(
            ParameterValue::Float(0.5).nudged(0.1).unwrap(),
            ParameterValue::Float(0.6)
        );
        assert_eq!(ParameterValue::Int(3).nudged(2.0).unwrap(), ParameterValue::Int(5));
        assert_eq!(
            ParameterValue::Color { r: 0.2, g: 0.5, b: 0.9 }.nudged(0.1).unwrap(),
            ParameterValue::Color { r: 0.3, g: 0.6, b: 1.0 },
            "every channel moves, so a nudge means brighter rather than redder"
        );
    }

    /// Past the top is the top. An operator holding a fader at full and asking for
    /// more should get full, not a value the output has to clamp behind their back.
    #[test]
    fn a_nudge_comes_to_rest_inside_the_range() {
        assert_eq!(ParameterValue::Float(0.95).nudged(0.2).unwrap(), ParameterValue::Float(1.0));
        assert_eq!(ParameterValue::Float(0.05).nudged(-0.2).unwrap(), ParameterValue::Float(0.0));

        // Approximately, because f32 subtraction is: what is being asserted is that
        // the channel already at the bottom stops there while the others carry on.
        let ParameterValue::Color { r, g, b } =
            ParameterValue::Color { r: 0.9, g: 0.1, b: 0.5 }.nudged(-0.3).unwrap()
        else {
            panic!("a nudged colour is a colour")
        };
        assert!((r - 0.6).abs() < 1e-5, "{r}");
        assert_eq!(g, 0.0, "the channel that would have gone under stops at the bottom");
        assert!((b - 0.2).abs() < 1e-5, "{b}");
    }

    #[test]
    fn a_nudge_says_so_where_there_is_no_halfway() {
        let switch = ParameterValue::Bool(true).nudged(0.1).expect_err("a relay has no halfway");
        assert!(switch.contains("on or off"), "{switch}");
        let text =
            ParameterValue::Text("HELLO".into()).nudged(1.0).expect_err("text cannot be nudged");
        assert!(text.contains("text"), "{text}");
    }
}
