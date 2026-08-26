use std::collections::HashMap;

use pult_schema::types::fixture::{
    Fixture, FixtureAddress, FixtureType, ParameterBinding, ParameterDefinition,
    ParameterDirection, ParameterKind, ParameterValue,
};
use uuid::Uuid;

use super::*;

fn a_type(parameters: Vec<ParameterDefinition>) -> FixtureType {
    FixtureType {
        id: Uuid::new_v4(),
        name: "Test".into(),
        manufacturer: "Acme".into(),
        channel_count: parameters.len() as u16,
        parameters,
    }
}

fn a_fixture(fixture_type: &FixtureType, universe: u16, address: u16) -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: fixture_type.id,
        address: FixtureAddress::Dmx { universe, address },
        position: None,
        live_values: HashMap::new(),
        active_preset: None,
    }
}

fn patch(fixtures: Vec<Fixture>, types: Vec<FixtureType>) -> Patch {
    Patch {
        fixtures,
        fixture_types: types.into_iter().map(|t| (t.id, t)).collect(),
    }
}

fn dimmer() -> ParameterDefinition {
    ParameterDefinition {
        kind: ParameterKind::Intensity,
        direction: ParameterDirection::Output,
        binding: ParameterBinding::Dmx { channel: 1 },
        default_value: ParameterValue::Float(0.0),
    }
}

#[test]
fn a_fixture_lands_on_its_patched_address() {
    let ft = a_type(vec![dimmer()]);
    let mut fixture = a_fixture(&ft, 1, 10);
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]));

    assert_eq!(universes.len(), 1);
    assert_eq!(universes[0].number, 1);
    assert_eq!(universes[0].channels[9], 255, "address 10 is index 9");
    assert_eq!(universes[0].channels[8], 0, "the channel below must be untouched");
    assert_eq!(universes[0].channels[10], 0);
}

#[test]
fn a_parameter_offset_is_added_to_the_fixture_address() {
    let ft = a_type(vec![
        dimmer(),
        ParameterDefinition {
            kind: ParameterKind::Pan,
            direction: ParameterDirection::Output,
            binding: ParameterBinding::Dmx { channel: 3 },
            default_value: ParameterValue::Float(0.0),
        },
    ]);
    let mut fixture = a_fixture(&ft, 1, 100);
    fixture.live_values.insert("Pan".into(), ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]));

    // Address 100, parameter channel 3 → DMX 102 → index 101.
    assert_eq!(universes[0].channels[101], 255);
}

#[test]
fn a_parameter_with_no_live_value_falls_back_to_its_default() {
    let ft = a_type(vec![ParameterDefinition {
        kind: ParameterKind::Intensity,
        direction: ParameterDirection::Output,
        binding: ParameterBinding::Dmx { channel: 1 },
        default_value: ParameterValue::Float(0.5),
    }]);
    let fixture = a_fixture(&ft, 1, 1);

    let universes = render(&patch(vec![fixture], vec![ft]));

    assert_eq!(universes[0].channels[0], 128);
}

#[test]
fn colour_takes_three_consecutive_channels() {
    let ft = a_type(vec![ParameterDefinition {
        kind: ParameterKind::ColorRgb,
        direction: ParameterDirection::Output,
        binding: ParameterBinding::Dmx { channel: 1 },
        default_value: ParameterValue::Color { r: 0.0, g: 0.0, b: 0.0 },
    }]);
    let mut fixture = a_fixture(&ft, 1, 5);
    fixture
        .live_values
        .insert("ColorRgb".into(), ParameterValue::Color { r: 1.0, g: 0.5, b: 0.0 });

    let universes = render(&patch(vec![fixture], vec![ft]));

    assert_eq!(universes[0].channels[4], 255);
    assert_eq!(universes[0].channels[5], 128);
    assert_eq!(universes[0].channels[6], 0);
}

#[test]
fn a_boolean_is_full_or_nothing() {
    let ft = a_type(vec![ParameterDefinition {
        kind: ParameterKind::Raw(1),
        direction: ParameterDirection::Output,
        binding: ParameterBinding::Dmx { channel: 1 },
        default_value: ParameterValue::Bool(false),
    }]);
    let mut fixture = a_fixture(&ft, 1, 1);
    fixture.live_values.insert("Raw:1".into(), ParameterValue::Bool(true));

    assert_eq!(render(&patch(vec![fixture.clone()], vec![ft.clone()]))[0].channels[0], 255);

    fixture.live_values.insert("Raw:1".into(), ParameterValue::Bool(false));
    assert_eq!(render(&patch(vec![fixture], vec![ft]))[0].channels[0], 0);
}

#[test]
fn an_out_of_range_level_clamps_instead_of_wrapping() {
    let ft = a_type(vec![dimmer()]);
    let mut fixture = a_fixture(&ft, 1, 1);

    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(4.0));
    assert_eq!(render(&patch(vec![fixture.clone()], vec![ft.clone()]))[0].channels[0], 255);

    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(-1.0));
    assert_eq!(
        render(&patch(vec![fixture], vec![ft]))[0].channels[0],
        0,
        "a bad value must dim a light, not flash it to full",
    );
}

#[test]
fn a_parameter_past_the_end_of_the_universe_is_dropped() {
    let ft = a_type(vec![dimmer()]);
    let mut fixture = a_fixture(&ft, 1, 512);
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture.clone()], vec![ft.clone()]));
    assert_eq!(universes[0].channels[511], 255, "512 is the last valid address");

    fixture.address = FixtureAddress::Dmx { universe: 1, address: 513 };
    let universes = render(&patch(vec![fixture], vec![ft]));
    assert!(
        universes[0].channels.iter().all(|c| *c == 0),
        "an address past the universe must not wrap into another fixture",
    );
}

#[test]
fn fixtures_are_grouped_into_the_universes_they_are_patched_to() {
    let ft = a_type(vec![dimmer()]);
    let mut one = a_fixture(&ft, 1, 1);
    let mut two = a_fixture(&ft, 7, 1);
    one.live_values.insert("Intensity".into(), ParameterValue::Float(1.0));
    two.live_values.insert("Intensity".into(), ParameterValue::Float(0.25));

    let universes = render(&patch(vec![two, one], vec![ft]));

    assert_eq!(universes.len(), 2);
    assert_eq!(universes[0].number, 1, "universes come out in order");
    assert_eq!(universes[0].channels[0], 255);
    assert_eq!(universes[1].number, 7);
    assert_eq!(universes[1].channels[0], 64);
}

#[test]
fn two_fixtures_sharing_a_universe_keep_their_own_channels() {
    let ft = a_type(vec![dimmer()]);
    let mut one = a_fixture(&ft, 1, 1);
    let mut two = a_fixture(&ft, 1, 2);
    one.live_values.insert("Intensity".into(), ParameterValue::Float(1.0));
    two.live_values.insert("Intensity".into(), ParameterValue::Float(0.0));

    let universes = render(&patch(vec![one, two], vec![ft]));

    assert_eq!(universes.len(), 1);
    assert_eq!(universes[0].channels[0], 255);
    assert_eq!(universes[0].channels[1], 0);
}

#[test]
fn a_fixture_patched_to_a_missing_type_is_skipped() {
    let ft = a_type(vec![dimmer()]);
    let mut orphan = a_fixture(&ft, 1, 1);
    orphan.fixture_type_id = Uuid::new_v4();
    orphan.live_values.insert("Intensity".into(), ParameterValue::Float(1.0));

    let universes = render(&patch(vec![orphan], vec![ft]));

    assert!(universes.is_empty(), "nothing sensible can be sent for an unknown type");
}

#[test]
fn a_fixture_on_a_node_has_no_place_in_a_universe() {
    let ft = a_type(vec![dimmer()]);
    let mut fixture = a_fixture(&ft, 1, 1);
    fixture.address = FixtureAddress::OpenHaunt { serial: "1a2b3c".into(), universe: Some(1) };
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]));

    assert!(universes.is_empty(), "a node fixture is not addressed by DMX slot");
}

#[test]
fn a_parameter_bound_to_a_port_takes_no_channel() {
    let ft = a_type(vec![ParameterDefinition {
        kind: ParameterKind::Switch(0),
        direction: ParameterDirection::Output,
        binding: ParameterBinding::Port { index: 0 },
        default_value: ParameterValue::Bool(false),
    }]);
    let mut fixture = a_fixture(&ft, 1, 1);
    fixture.live_values.insert("Switch:0".into(), ParameterValue::Bool(true));

    let universes = render(&patch(vec![fixture], vec![ft]));

    assert!(
        universes[0].channels.iter().all(|c| *c == 0),
        "a relay port is not a DMX channel, whatever the fixture is addressed to",
    );
}

#[test]
fn an_input_parameter_is_never_written_to_the_wire() {
    let ft = a_type(vec![ParameterDefinition {
        kind: ParameterKind::Contact(0),
        direction: ParameterDirection::Input,
        // Deliberately bound to a channel: direction alone has to be enough to
        // keep a reading the device produced from being sent back out.
        binding: ParameterBinding::Dmx { channel: 1 },
        default_value: ParameterValue::Bool(false),
    }]);
    let mut fixture = a_fixture(&ft, 1, 1);
    fixture.live_values.insert("Contact:0".into(), ParameterValue::Bool(true));

    let universes = render(&patch(vec![fixture], vec![ft]));

    assert!(universes[0].channels.iter().all(|c| *c == 0));
}

#[test]
fn text_leaves_the_channel_it_sits_on_alone() {
    let ft = a_type(vec![
        dimmer(),
        ParameterDefinition {
            kind: ParameterKind::Text,
            direction: ParameterDirection::Output,
            binding: ParameterBinding::Dmx { channel: 2 },
            default_value: ParameterValue::Text(String::new()),
        },
    ]);
    let mut fixture = a_fixture(&ft, 1, 1);
    fixture.live_values.insert("Intensity".into(), ParameterValue::Float(1.0));
    fixture.live_values.insert("Text".into(), ParameterValue::Text("BOO".into()));

    let universes = render(&patch(vec![fixture], vec![ft]));

    assert_eq!(universes[0].channels[0], 255);
    assert_eq!(universes[0].channels[1], 0, "there is no byte that means 'BOO'");
}
