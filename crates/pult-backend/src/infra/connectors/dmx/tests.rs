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
        ..FixtureType::default()
    }
}

fn a_fixture(fixture_type: &FixtureType, universe: u16, address: u16) -> Fixture {
    Fixture {
        id: Uuid::new_v4(),
        name: "Spot".into(),
        fixture_type_id: fixture_type.id,
        address: FixtureAddress::dmx(universe, address),
        position: None,
        sensed_values: HashMap::new(),
        live_effects: Default::default(),
        live_fades: Default::default(),
        home_values: Default::default(),
        ..Fixture::default()
    }
}

fn patch(fixtures: Vec<Fixture>, types: Vec<FixtureType>) -> Patch {
    Patch::new(fixtures, types, vec![])
}

fn dimmer() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0))
}

#[test]
fn a_fixture_lands_on_its_patched_address() {
    let ft = a_type(vec![dimmer()]);
    let mut fixture = a_fixture(&ft, 1, 10);
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);

    assert_eq!(universes.len(), 1);
    assert_eq!(universes[0].number, 1);
    assert_eq!(universes[0].channels[9], 255, "address 10 is index 9");
    assert_eq!(universes[0].channels[8], 0, "the channel below must be untouched");
    assert_eq!(universes[0].channels[10], 0);
}

#[test]
fn a_parameter_offset_is_added_to_the_fixture_address() {
    // Offset 3 with nothing at 2: a gap is a fact about a mode, which is why the
    // mode is written out rather than left to the implicit one's parameter order.
    let ft = a_modal_type(
        vec![
            dimmer(),
            ParameterDefinition::new(ParameterKind::Pan, ParameterValue::Float(0.0)),
        ],
        vec![DmxMode {
            name: "Default".into(),
            breaks: vec![3],
            channels: vec![channel("Intensity", 0, vec![1]), channel("Pan", 0, vec![3])],
        }],
    );
    let mut fixture = a_fixture(&ft, 1, 100);
    holding(&mut fixture, "Pan", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);

    // Address 100, parameter channel 3 → DMX 102 → index 101.
    assert_eq!(universes[0].channels[101], 255);
}

#[test]
fn a_parameter_with_no_live_value_falls_back_to_its_default() {
    let ft = a_type(vec![ParameterDefinition::new(
        ParameterKind::Intensity,
        ParameterValue::Float(0.5))],
    );
    let fixture = a_fixture(&ft, 1, 1);

    let universes = render(&patch(vec![fixture], vec![ft]), 0);

    assert_eq!(universes[0].channels[0], 128);
}

#[test]
fn colour_takes_three_consecutive_channels() {
    let ft = a_type(vec![ParameterDefinition::new(
        ParameterKind::ColorRgb,
        ParameterValue::rgb(0.0, 0.0, 0.0))],
    );
    let mut fixture = a_fixture(&ft, 1, 5);
    holding(&mut fixture, "ColorRgb", ParameterValue::rgb(1.0, 0.5, 0.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);

    assert_eq!(universes[0].channels[4], 255);
    assert_eq!(universes[0].channels[5], 128);
    assert_eq!(universes[0].channels[6], 0);
}

#[test]
fn a_boolean_is_full_or_nothing() {
    let ft = a_type(vec![ParameterDefinition::new(
        ParameterKind::Raw(1),
        ParameterValue::Bool(false))],
    );
    let mut fixture = a_fixture(&ft, 1, 1);
    holding(&mut fixture, "Raw:1", ParameterValue::Bool(true));

    assert_eq!(render(&patch(vec![fixture.clone()], vec![ft.clone()]), 0)[0].channels[0], 255);

    holding(&mut fixture, "Raw:1", ParameterValue::Bool(false));
    assert_eq!(render(&patch(vec![fixture], vec![ft]), 0)[0].channels[0], 0);
}

#[test]
fn an_out_of_range_level_clamps_instead_of_wrapping() {
    let ft = a_type(vec![dimmer()]);
    let mut fixture = a_fixture(&ft, 1, 1);

    holding(&mut fixture, "Intensity", ParameterValue::Float(4.0));
    assert_eq!(render(&patch(vec![fixture.clone()], vec![ft.clone()]), 0)[0].channels[0], 255);

    holding(&mut fixture, "Intensity", ParameterValue::Float(-1.0));
    assert_eq!(
        render(&patch(vec![fixture], vec![ft]), 0)[0].channels[0],
        0,
        "a bad value must dim a light, not flash it to full",
    );
}

#[test]
fn a_parameter_past_the_end_of_the_universe_is_dropped() {
    let ft = a_type(vec![dimmer()]);
    let mut fixture = a_fixture(&ft, 1, 512);
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture.clone()], vec![ft.clone()]), 0);
    assert_eq!(universes[0].channels[511], 255, "512 is the last valid address");

    fixture.address = FixtureAddress::dmx(1, 513);
    let universes = render(&patch(vec![fixture], vec![ft]), 0);
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
    holding(&mut one, "Intensity", ParameterValue::Float(1.0));
    holding(&mut two, "Intensity", ParameterValue::Float(0.25));

    let universes = render(&patch(vec![two, one], vec![ft]), 0);

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
    holding(&mut one, "Intensity", ParameterValue::Float(1.0));
    holding(&mut two, "Intensity", ParameterValue::Float(0.0));

    let universes = render(&patch(vec![one, two], vec![ft]), 0);

    assert_eq!(universes.len(), 1);
    assert_eq!(universes[0].channels[0], 255);
    assert_eq!(universes[0].channels[1], 0);
}

#[test]
fn a_fixture_patched_to_a_missing_type_is_skipped() {
    let ft = a_type(vec![dimmer()]);
    let mut orphan = a_fixture(&ft, 1, 1);
    orphan.fixture_type_id = Uuid::new_v4();
    holding(&mut orphan, "Intensity", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![orphan], vec![ft]), 0);

    assert!(universes.is_empty(), "nothing sensible can be sent for an unknown type");
}

#[test]
fn a_fixture_on_a_node_has_no_place_in_a_universe() {
    let ft = a_type(vec![dimmer()]);
    let mut fixture = a_fixture(&ft, 1, 1);
    fixture.address = FixtureAddress::OpenHaunt { serial: "1a2b3c".into(), universe: Some(1) };
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);

    assert!(universes.is_empty(), "a node fixture is not addressed by DMX slot");
}

#[test]
fn a_parameter_bound_to_a_port_takes_no_channel() {
    let ft = a_type(vec![ParameterDefinition {
        binding: Some(ParameterBinding::Port { index: 0 }),
        ..ParameterDefinition::new(ParameterKind::Switch(0), ParameterValue::Bool(false))
    }]);
    let mut fixture = a_fixture(&ft, 1, 1);
    holding(&mut fixture, "Switch:0", ParameterValue::Bool(true));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);

    assert!(
        universes[0].channels.iter().all(|c| *c == 0),
        "a relay port is not a DMX channel, whatever the fixture is addressed to",
    );
}

#[test]
fn an_input_parameter_is_never_written_to_the_wire() {
    // Deliberately given a channel of its own by the mode: direction alone has to be
    // enough to keep a reading the device produced from being sent back out.
    let ft = a_modal_type(
        vec![ParameterDefinition {
            direction: ParameterDirection::Input,
            ..ParameterDefinition::new(ParameterKind::Contact(0), ParameterValue::Bool(false))
        }],
        vec![DmxMode {
            name: "Default".into(),
            breaks: vec![1],
            channels: vec![channel("Contact:0", 0, vec![1])],
        }],
    );
    let mut fixture = a_fixture(&ft, 1, 1);
    holding(&mut fixture, "Contact:0", ParameterValue::Bool(true));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);

    assert!(universes[0].channels.iter().all(|c| *c == 0));
}

#[test]
fn text_leaves_the_channel_it_sits_on_alone() {
    let ft = a_type(vec![
        dimmer(),
        ParameterDefinition::new(ParameterKind::Text, ParameterValue::Text(String::new())),
    ]);
    let mut fixture = a_fixture(&ft, 1, 1);
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));
    holding(&mut fixture, "Text", ParameterValue::Text("BOO".into()));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);

    assert_eq!(universes[0].channels[0], 255);
    assert_eq!(universes[0].channels[1], 0, "there is no byte that means 'BOO'");
}

// ── Modes ─────────────────────────────────────────────────────────────────────
//
// Everything above describes a type with no modes, laid out by the implicit one. What
// follows is the other half: a type that names its own, which is what a GDTF import
// produces and what a fixture with a 16-bit pan, a second break or a fourth emitter
// needs in order to be addressable at all.

use pult_schema::types::dmx_mode::{ChannelFunctionRange, DmxBreak, DmxChannelLayout, DmxMode};
use pult_schema::types::fixture::{rgb_emitters, Emitter, Vec3};

/// A type with modes rather than bindings: the shape an imported fixture takes.
fn a_modal_type(parameters: Vec<ParameterDefinition>, modes: Vec<DmxMode>) -> FixtureType {
    FixtureType {
        id: Uuid::new_v4(),
        name: "Modal".into(),
        manufacturer: "Acme".into(),
        channel_count: modes.first().map(DmxMode::channel_count).unwrap_or(0),
        parameters,
        dmx_modes: modes,
        ..FixtureType::default()
    }
}

fn channel(key: &str, break_index: u8, offsets: Vec<u16>) -> DmxChannelLayout {
    DmxChannelLayout {
        parameter_key: key.into(),
        break_index,
        offsets,
        default: 0,
        functions: Vec::new(),
        emitter: None,
    }
}

/// A fixture in a named mode, at one address per break.
fn a_modal_fixture(
    fixture_type: &FixtureType,
    mode: &str,
    breaks: Vec<(u16, u16)>,
) -> Fixture {
    Fixture {
        address: FixtureAddress::Dmx {
            mode: mode.into(),
            breaks: breaks
                .into_iter()
                .map(|(universe, address)| DmxBreak { universe, address })
                .collect(),
        },
        ..a_fixture(fixture_type, 1, 1)
    }
}

#[test]
fn a_sixteen_bit_channel_puts_the_coarse_byte_first_and_the_fine_one_where_the_mode_says() {
    // Offsets 1 and 9: a real head commonly writes every coarse byte, then every fine
    // one. A writer that took the first offset and put the rest after it would land
    // the fine byte on somebody else's colour.
    let ft = a_modal_type(
        vec![ParameterDefinition::new(ParameterKind::Pan, ParameterValue::Float(0.5))],
        vec![DmxMode {
            name: "16-bit".into(),
            breaks: vec![9],
            channels: vec![channel("Pan", 0, vec![1, 9])],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "16-bit", vec![(1, 1)]);
    holding(&mut fixture, "Pan", ParameterValue::Float(0.5));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    let channels = &universes[0].channels;

    // Half of 65535 is 32768 at two bytes — 0x8000 — and not 128 with the fine byte
    // left at zero, which is what an 8-bit value widened by shifting would give.
    assert_eq!(channels[0], 0x80, "the coarse byte");
    assert_eq!(channels[8], 0x00, "the fine byte, nine slots along");
    assert!(channels[1..8].iter().all(|c| *c == 0), "nothing in between was touched");
}

#[test]
fn a_sixteen_bit_channel_resolves_finer_than_an_eight_bit_one() {
    let ft = a_modal_type(
        vec![ParameterDefinition::new(ParameterKind::Pan, ParameterValue::Float(0.0))],
        vec![DmxMode {
            name: "16-bit".into(),
            breaks: vec![2],
            channels: vec![channel("Pan", 0, vec![1, 2])],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "16-bit", vec![(1, 1)]);
    // A value an 8-bit channel cannot tell from the one beside it.
    holding(&mut fixture, "Pan", ParameterValue::Float(0.5 + 1.0 / 512.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    assert_eq!(universes[0].channels[0], 0x80);
    assert_eq!(universes[0].channels[1], 0x80, "the fine byte carries what the coarse cannot");
}

#[test]
fn a_mode_with_two_breaks_writes_into_two_universes() {
    let ft = a_modal_type(
        vec![
            ParameterDefinition::new(ParameterKind::Pan, ParameterValue::Float(0.0)),
            ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0)),
        ],
        vec![DmxMode {
            name: "Split".into(),
            breaks: vec![1, 1],
            channels: vec![channel("Pan", 0, vec![1]), channel("Intensity", 1, vec![1])],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "Split", vec![(1, 5), (7, 100)]);
    holding(&mut fixture, "Pan", ParameterValue::Float(1.0));
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    assert_eq!(universes.len(), 2, "two breaks, two universes");
    assert_eq!(universes[0].number, 1);
    assert_eq!(universes[0].channels[4], 255, "pan at universe 1 channel 5");
    assert_eq!(universes[1].number, 7);
    assert_eq!(universes[1].channels[99], 255, "the dimmer break, somewhere else entirely");
}

// ── What an output carries ────────────────────────────────────────────────────
//
// `OutputConfig::universes` documented itself as a filter for a year and no
// connector read it, so an output restricted to universe 1 transmitted all seven.
// These are the tests that the field means what it says, and that it means it at the
// point where it is worth anything — before the rig is evaluated rather than after.

#[test]
fn an_output_renders_only_the_universes_it_carries() {
    let ft = a_type(vec![dimmer()]);
    let mut here = a_fixture(&ft, 1, 10);
    let mut elsewhere = a_fixture(&ft, 5, 10);
    holding(&mut here, "Intensity", ParameterValue::Float(1.0));
    holding(&mut elsewhere, "Intensity", ParameterValue::Float(1.0));
    let patch = patch(vec![here, elsewhere], vec![ft]);

    let all = render(&patch, 0);
    assert_eq!(all.iter().map(|u| u.number).collect::<Vec<_>>(), vec![1, 5]);

    let restricted = render_carried(&patch, 0, &[1]);
    assert_eq!(restricted.len(), 1, "the other universe is not built at all");
    assert_eq!(restricted[0].number, 1);
    assert_eq!(restricted[0].channels[9], 255, "and what it does carry is unchanged");
}

#[test]
fn a_universe_nothing_carries_is_absent_rather_than_blank() {
    let ft = a_type(vec![dimmer()]);
    let fixture = a_fixture(&ft, 4, 1);

    let rendered = render_carried(&patch(vec![fixture], vec![ft]), 0, &[1, 2]);

    assert!(
        rendered.is_empty(),
        "a universe of zeroes would put a blackout on a wire that had been carrying \
         somebody else's rig"
    );
}

#[test]
fn one_break_of_a_fixture_can_be_carried_and_the_other_not() {
    // Which is the whole reason the filter is per universe rather than per fixture: a
    // head with a separate dimmer break sits in two spans that need not be on one
    // node, and each half goes out on the output that carries it.
    let ft = a_modal_type(
        vec![
            ParameterDefinition::new(ParameterKind::Pan, ParameterValue::Float(0.0)),
            ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0)),
        ],
        vec![DmxMode {
            name: "Split".into(),
            breaks: vec![1, 1],
            channels: vec![channel("Pan", 0, vec![1]), channel("Intensity", 1, vec![1])],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "Split", vec![(1, 5), (7, 100)]);
    holding(&mut fixture, "Pan", ParameterValue::Float(1.0));
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));

    let rendered = render_carried(&patch(vec![fixture], vec![ft]), 0, &[7]);

    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].number, 7);
    assert_eq!(rendered[0].channels[99], 255, "the dimmer break, on the output that has it");
}

#[test]
fn an_empty_list_still_means_every_universe() {
    let ft = a_type(vec![dimmer()]);
    let fixtures = vec![a_fixture(&ft, 1, 1), a_fixture(&ft, 9, 1)];
    let patch = patch(fixtures, vec![ft]);

    assert_eq!(render_carried(&patch, 0, &[]).len(), 2);
    assert_eq!(render_carried(&patch, 0, &[]).len(), render(&patch, 0).len());
}

#[test]
fn a_break_the_fixture_has_no_address_in_is_dropped_rather_than_guessed() {
    let ft = a_modal_type(
        vec![ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0))],
        vec![DmxMode {
            name: "Split".into(),
            breaks: vec![0, 1],
            channels: vec![channel("Intensity", 1, vec![1])],
        }],
    );
    // Patched with one address, in a mode that wants two.
    let mut fixture = a_modal_fixture(&ft, "Split", vec![(1, 1)]);
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    assert!(
        universes[0].channels.iter().all(|c| *c == 0),
        "a break with no address is nowhere, not channel 1",
    );
}

#[test]
fn a_mode_that_lacks_a_parameter_a_cue_drives_simply_does_not_send_it() {
    let ft = a_modal_type(
        vec![
            ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0)),
            ParameterDefinition::new(ParameterKind::Zoom, ParameterValue::Float(0.0)),
        ],
        vec![DmxMode {
            name: "Basic".into(),
            breaks: vec![1],
            channels: vec![channel("Intensity", 0, vec![1])],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "Basic", vec![(1, 1)]);
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));
    holding(&mut fixture, "Zoom", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    assert_eq!(universes[0].channels[0], 255);
    assert!(
        universes[0].channels[1..].iter().all(|c| *c == 0),
        "the zoom this mode does not have has nowhere to go, and does not go anywhere",
    );
}

#[test]
fn a_mode_the_type_does_not_have_falls_back_to_the_first_one() {
    let ft = a_modal_type(
        vec![ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0))],
        vec![DmxMode {
            name: "Standard".into(),
            breaks: vec![1],
            channels: vec![channel("Intensity", 0, vec![1])],
        }],
    );
    // A show patched against a revision of the file that had this mode.
    let mut fixture = a_modal_fixture(&ft, "Extended", vec![(1, 1)]);
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    assert_eq!(
        universes[0].channels[0], 255,
        "going to the wrong mode beats going dark because a file was revised",
    );
}

#[test]
fn an_rgbw_head_lights_its_white_die_on_a_colour_from_a_cue_that_knows_nothing_about_it() {
    let emitters = {
        let mut list = rgb_emitters();
        list.push(Emitter {
            name: "White".into(),
            rgb: Some(Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
            subtractive: false,
        });
        list
    };
    let colour = ParameterDefinition {
        emitters: emitters.clone(),
        ..ParameterDefinition::new(ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0))
    };
    let ft = a_modal_type(
        vec![colour],
        vec![DmxMode {
            name: "RGBW".into(),
            breaks: vec![4],
            channels: emitters
                .iter()
                .enumerate()
                .map(|(index, emitter)| DmxChannelLayout {
                    emitter: Some(emitter.name.clone()),
                    ..channel("ColorRgb", 0, vec![index as u16 + 1])
                })
                .collect(),
        }],
    );

    // A cue written against a plain RGB console: full red.
    let mut fixture = a_modal_fixture(&ft, "RGBW", vec![(1, 1)]);
    holding(&mut fixture, "ColorRgb", ParameterValue::rgb(1.0, 0.0, 0.0));
    let universes = render(&patch(vec![fixture.clone()], vec![ft.clone()]), 0);
    assert_eq!(&universes[0].channels[..4], &[255, 0, 0, 0], "red is not white");

    // And white: the neutral part of the colour is what the white die is for.
    holding(&mut fixture, "ColorRgb", ParameterValue::rgb(1.0, 1.0, 1.0));
    let universes = render(&patch(vec![fixture.clone()], vec![ft.clone()]), 0);
    assert_eq!(&universes[0].channels[..4], &[255, 255, 255, 255]);

    // Unless somebody said otherwise about that one die.
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert("White".to_string(), 0.0);
    holding(
        &mut fixture,
        "ColorRgb",
        ParameterValue::Color { r: 1.0, g: 1.0, b: 1.0, overrides },
    );
    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    assert_eq!(
        &universes[0].channels[..4],
        &[255, 255, 255, 0],
        "an override wins over anything derived",
    );
}

#[test]
fn an_integer_lands_in_the_named_range_the_mode_gives_it() {
    let ft = a_modal_type(
        vec![ParameterDefinition::new(ParameterKind::Gobo(1), ParameterValue::Int(0))],
        vec![DmxMode {
            name: "Standard".into(),
            breaks: vec![1],
            channels: vec![DmxChannelLayout {
                functions: vec![
                    ChannelFunctionRange {
                        name: "Open".into(),
                        dmx_from: 0,
                        dmx_to: 9,
                        ..ChannelFunctionRange::default()
                    },
                    ChannelFunctionRange {
                        name: "Breakup".into(),
                        dmx_from: 10,
                        dmx_to: 19,
                        ..ChannelFunctionRange::default()
                    },
                    ChannelFunctionRange {
                        name: "Dots".into(),
                        dmx_from: 20,
                        dmx_to: 255,
                        ..ChannelFunctionRange::default()
                    },
                ],
                ..channel("Gobo:1", 0, vec![1])
            }],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "Standard", vec![(1, 1)]);
    holding(&mut fixture, "Gobo:1", ParameterValue::Int(2));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    assert_eq!(
        universes[0].channels[0], 20,
        "the third slot is wherever the file put it, not at 2 out of 255",
    );
}

#[test]
fn a_virtual_channel_occupies_nothing() {
    let ft = a_modal_type(
        vec![
            ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0)),
            ParameterDefinition::new(ParameterKind::Zoom, ParameterValue::Float(0.0)),
        ],
        vec![DmxMode {
            name: "Standard".into(),
            breaks: vec![1],
            channels: vec![
                channel("Intensity", 0, vec![1]),
                // `Offset="None"` in the file: something the console can show and
                // cannot send.
                channel("Zoom", 0, Vec::new()),
            ],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "Standard", vec![(1, 1)]);
    holding(&mut fixture, "Intensity", ParameterValue::Float(1.0));
    holding(&mut fixture, "Zoom", ParameterValue::Float(1.0));

    let universes = render(&patch(vec![fixture], vec![ft]), 0);
    assert_eq!(universes[0].channels[0], 255);
    assert!(universes[0].channels[1..].iter().all(|c| *c == 0));
}

// ── What a viewer reads off the dedup cache ───────────────────────────────────

fn a_universe(number: u16, channels: &[(usize, u8)]) -> Universe {
    let mut universe = Universe::new(number);
    for (at, value) in channels {
        universe.channels[*at] = *value;
    }
    universe
}

#[test]
fn a_viewer_reads_the_images_the_dedup_was_already_keeping() {
    let mut cache = UniverseCache::default();
    let now = std::time::Instant::now();
    cache.needs_send(&a_universe(1, &[(0, 255), (5, 128)]), now, REFRESH_AFTER);
    cache.needs_send(&a_universe(4, &[]), now, REFRESH_AFTER);

    let traffic = cache.observe(Some("4"), now);
    assert_eq!(
        traffic.universes.iter().map(|u| u.universe).collect::<Vec<_>>(),
        vec![1, 4],
        "every universe this connector carries, in order, whichever is being looked at"
    );
    assert_eq!(traffic.universes[0].live_channels, 2);
    assert_eq!(traffic.universes[1].live_channels, 0);

    let focused = traffic.focused.expect("the universe that was asked for");
    assert_eq!(focused.universe, 4);
    assert_eq!(focused.channels.len(), UNIVERSE_SIZE);

    let first = cache.observe(Some("1"), now).focused.unwrap();
    assert_eq!(first.channels[0], 255);
    assert_eq!(first.channels[5], 128);
}

#[test]
fn asking_for_nothing_shows_the_first_universe_rather_than_a_blank_sheet() {
    let mut cache = UniverseCache::default();
    let now = std::time::Instant::now();
    cache.needs_send(&a_universe(7, &[(0, 1)]), now, REFRESH_AFTER);

    assert_eq!(cache.observe(None, now).focused.unwrap().universe, 7);
    assert_eq!(
        cache.observe(Some("99"), now).focused.unwrap().universe,
        7,
        "and a universe this output does not carry falls back rather than going blank"
    );
}

#[test]
fn a_keep_alive_is_not_a_change() {
    let mut cache = UniverseCache::default();
    let began = std::time::Instant::now();
    let settled = a_universe(1, &[(0, 200)]);
    cache.needs_send(&settled, began, REFRESH_AFTER);

    // A second later, the same bytes go out again because the keep-alive is due.
    let later = began + std::time::Duration::from_secs(1);
    assert!(cache.needs_send(&settled, later, REFRESH_AFTER), "the refresh is due");

    let traffic = cache.observe(None, later);
    assert_eq!(traffic.universes[0].sent_ms_ago, 0, "it has just been sent");
    assert!(
        traffic.universes[0].changed_ms_ago >= 1000,
        "and has not changed in a second — a sheet that read the send as movement would \
         report every idle universe as busy"
    );
}

// ── Reading a wire back ───────────────────────────────────────────────────────

/// The gate on decoding, and it is a corpus rather than a list of expected numbers:
/// render a patch, read the universes back as values, drive the same patch from those
/// values, and require the bytes to be identical. A decoder that is wrong about a
/// width, an offset order, a named range or an emitter fails here, and the failure
/// names the fixture rather than a byte.
fn survives_a_round_trip(name: &str, fixtures: Vec<Fixture>, types: Vec<FixtureType>) {
    let patch = patch(fixtures.clone(), types.clone());
    let before = render(&patch, 0);

    let images: HashMap<u16, [u8; UNIVERSE_SIZE]> =
        before.iter().map(|universe| (universe.number, universe.channels)).collect();
    let read = patch.decode(&images);
    assert!(!read.is_empty(), "{name}: nothing was decoded at all");

    // Put what was read back on the fixtures as landed fades — the only way a
    // parameter holds a value here — and render again.
    let mut driven = fixtures;
    for (fixture_id, key, value) in read {
        let fixture = driven.iter_mut().find(|f| f.id == fixture_id).expect("a patched fixture");
        holding(fixture, &key, value);
    }
    let after = render(&patch::Patch::new(driven, types, vec![]), 0);

    assert_eq!(before.len(), after.len(), "{name}: a universe appeared or vanished");
    for (before, after) in before.iter().zip(after.iter()) {
        assert_eq!(before.number, after.number, "{name}: universes came back in a new order");
        let differing: Vec<usize> = (0..UNIVERSE_SIZE)
            .filter(|slot| before.channels[*slot] != after.channels[*slot])
            .collect();
        assert!(
            differing.is_empty(),
            "{name}: universe {} came back different at slots {:?} ({:?} against {:?})",
            before.number,
            &differing[..differing.len().min(8)],
            differing.iter().take(8).map(|s| before.channels[*s]).collect::<Vec<_>>(),
            differing.iter().take(8).map(|s| after.channels[*s]).collect::<Vec<_>>(),
        );
    }
}

/// A shim so the helper above can say `patch::Patch::new` without shadowing the
/// `patch` function every other test here calls.
mod patch {
    pub use super::super::Patch;
}

fn an_emitter(name: &str, rgb: [f32; 3]) -> Emitter {
    Emitter {
        name: name.into(),
        rgb: Some(Vec3 { x: rgb[0], y: rgb[1], z: rgb[2] }),
        subtractive: false,
    }
}

/// A colour fixture whose emitters occupy one byte each from the start address.
fn a_colour_head(emitters: Vec<Emitter>) -> FixtureType {
    let colour = ParameterDefinition {
        emitters: emitters.clone(),
        ..ParameterDefinition::new(ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0))
    };
    a_modal_type(
        vec![colour],
        vec![DmxMode {
            name: "Colour".into(),
            breaks: vec![emitters.len() as u16],
            channels: emitters
                .iter()
                .enumerate()
                .map(|(index, emitter)| DmxChannelLayout {
                    emitter: Some(emitter.name.clone()),
                    ..channel("ColorRgb", 0, vec![index as u16 + 1])
                })
                .collect(),
        }],
    )
}

#[test]
fn a_plain_rgb_head_reads_back_as_the_bytes_it_went_out_as() {
    let ft = a_colour_head(rgb_emitters());
    let mut fixture = a_modal_fixture(&ft, "Colour", vec![(1, 1)]);
    holding(&mut fixture, "ColorRgb", ParameterValue::rgb(1.0, 0.4, 0.0));
    survives_a_round_trip("rgb", vec![fixture], vec![ft]);
}

/// The case `unmix` exists for: a white die at something the mix would never derive.
/// Without the overrides this comes back as a colour whose white is `min(r,g,b)` and
/// the byte changes, which is a guest console's rig quietly repatched on the way
/// through this one.
#[test]
fn an_rgbw_head_keeps_a_white_the_mix_would_not_have_derived() {
    let mut emitters = rgb_emitters();
    emitters.push(an_emitter("White", [1.0, 1.0, 1.0]));
    let ft = a_colour_head(emitters);

    let mut fixture = a_modal_fixture(&ft, "Colour", vec![(1, 1)]);
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert("White".to_string(), 1.0);
    holding(
        &mut fixture,
        "ColorRgb",
        ParameterValue::Color { r: 0.2, g: 0.2, b: 0.2, overrides },
    );
    survives_a_round_trip("rgbw", vec![fixture], vec![ft]);
}

#[test]
fn a_sixteen_bit_intensity_reads_back_at_its_own_width() {
    let ft = a_modal_type(
        vec![dimmer()],
        vec![DmxMode {
            name: "16-bit".into(),
            breaks: vec![2],
            // Coarse then fine, nine slots apart: the arrangement a real head uses and
            // the one a reader that sorted its offsets would get backwards.
            channels: vec![channel("Intensity", 0, vec![1, 10])],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "16-bit", vec![(1, 20)]);
    holding(&mut fixture, "Intensity", ParameterValue::Float(0.371_234));
    survives_a_round_trip("16-bit intensity", vec![fixture], vec![ft]);
}

#[test]
fn a_gobo_wheel_reads_back_as_the_slot_rather_than_as_the_byte() {
    let ranges = |from: u32, to: u32, name: &str| ChannelFunctionRange {
        name: name.into(),
        attribute: String::new(),
        dmx_from: from,
        dmx_to: to,
        physical_from: 0.0,
        physical_to: 0.0,
    };
    let ft = a_modal_type(
        vec![ParameterDefinition::new(ParameterKind::Gobo(1), ParameterValue::Int(0))],
        vec![DmxMode {
            name: "Wheel".into(),
            breaks: vec![1],
            channels: vec![DmxChannelLayout {
                functions: vec![
                    ranges(0, 9, "Open"),
                    ranges(10, 19, "Breakup"),
                    ranges(20, 29, "Dots"),
                ],
                ..channel("Gobo:1", 0, vec![1])
            }],
        }],
    );
    let mut fixture = a_modal_fixture(&ft, "Wheel", vec![(1, 5)]);
    holding(&mut fixture, "Gobo:1", ParameterValue::Int(2));
    survives_a_round_trip("gobo wheel", vec![fixture.clone()], vec![ft.clone()]);

    // And the value itself, not only the bytes: what a grab writes into the programmer
    // has to be "the third gobo", or an operator's next `+1` lands somewhere else.
    holding(&mut fixture, "Gobo:1", ParameterValue::Int(2));
    let patch = patch(vec![fixture], vec![ft]);
    let images: HashMap<u16, [u8; UNIVERSE_SIZE]> =
        render(&patch, 0).iter().map(|u| (u.number, u.channels)).collect();
    assert_eq!(patch.decode(&images)[0].2, ParameterValue::Int(2));
}

/// A whole rig at once, which is what a grab actually decodes.
#[test]
fn a_universe_of_several_fixtures_comes_back_as_all_of_them() {
    let dim = a_type(vec![dimmer()]);
    let mut one = a_fixture(&dim, 1, 1);
    holding(&mut one, "Intensity", ParameterValue::Float(1.0));
    let mut two = a_fixture(&dim, 1, 2);
    holding(&mut two, "Intensity", ParameterValue::Float(0.2));

    let rgb = a_colour_head(rgb_emitters());
    let mut head = a_modal_fixture(&rgb, "Colour", vec![(1, 10)]);
    holding(&mut head, "ColorRgb", ParameterValue::rgb(0.0, 1.0, 0.5));

    survives_a_round_trip("a rig", vec![one, two, head], vec![dim, rgb]);
}

/// A universe nobody handed over says nothing, rather than reading as a rig at zero —
/// which for a grab would black out every fixture the input does not carry.
#[test]
fn a_fixture_in_a_universe_that_did_not_arrive_is_not_decoded() {
    let ft = a_type(vec![dimmer()]);
    let here = a_fixture(&ft, 1, 1);
    let elsewhere = a_fixture(&ft, 4, 1);
    let ids = (here.id, elsewhere.id);
    let patch = patch(vec![here, elsewhere], vec![ft]);

    let images: HashMap<u16, [u8; UNIVERSE_SIZE]> =
        [(1u16, [77u8; UNIVERSE_SIZE])].into_iter().collect();
    let read = patch.decode(&images);

    assert_eq!(read.len(), 1);
    assert_eq!(read[0].0, ids.0);
    assert!(read.iter().all(|(id, _, _)| *id != ids.1));
}
