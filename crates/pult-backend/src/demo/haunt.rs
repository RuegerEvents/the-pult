//! The small demo, in Rust: five fixtures, three cues, two flows.
//!
//! A port of what `scripts/demo-seed.mjs` used to seed over the WebSocket, moved in
//! here so that opening it is a card on the welcome screen rather than a terminal.
//! What it shows is deliberately the whole console in miniature — a patch, a rig
//! with positions in it, a cue stack, an effect on a speed master, and two flow
//! graphs, one of which does the thing a one-row-per-rule trigger never could.

use anyhow::Result;
use pult_schema::types::{
    cue::{Cue, FollowMode, ParameterCapture},
    effect::{Curve, Direction, Easing, EffectSpec, Rate, Shape, Spread},
    fixture::{
        Fixture, FixtureAddress, FixtureType, ParameterDefinition, ParameterKind, ParameterValue,
    },
    flow::{Flow, FlowEdge, FlowNode, FlowNodeKind, TriggerAction, TriggerCondition, TriggerSource},
    fixture::Vec3,
    scene::Transform,
    sequence::Sequence,
    speedmaster::SpeedMaster,
};
use pult_schema::types::dmx_mode::DmxBreak;
use uuid::Uuid;

use super::{id, now_ms, Seeder};

/// Facing (0, -0.8, 0.6): downstage and down, which is how these heads hang.
///
/// Written as the rotation it is rather than as a direction, and named for the
/// direction it means — nobody reads `{143.13, 0, 180}` and sees a light pointing at
/// the audience's feet.
pub(crate) const DOWNSTAGE_AND_DOWN: Vec3 = Vec3 { x: 143.1301, y: 0.0, z: 180.0 };

pub(crate) fn at(x: f32, y: f32, z: f32) -> Transform {
    Transform {
        position: Vec3 { x, y, z },
        rotation: Vec3::default(),
        scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
    }
}

pub(crate) fn aimed(x: f32, y: f32, z: f32, rotation: Vec3) -> Transform {
    Transform { rotation, ..at(x, y, z) }
}

/// One DMX address in universe `universe` at `address`.
pub(crate) fn dmx(universe: u16, address: u16) -> FixtureAddress {
    FixtureAddress::Dmx {
        mode: "Default".to_string(),
        breaks: vec![DmxBreak { universe, address }],
    }
}

pub(crate) fn a_type(name: &str, parameters: Vec<ParameterDefinition>) -> FixtureType {
    FixtureType {
        id: id(),
        name: name.to_string(),
        manufacturer: "Generic".to_string(),
        // One byte per output parameter and three for a colour — the implicit mode's
        // own arithmetic, which is what a type with no `dmx_modes` gets.
        channel_count: parameters
            .iter()
            .map(|p| if matches!(p.kind, ParameterKind::ColorRgb) { 3 } else { 1 })
            .sum::<u16>(),
        parameters,
        ..FixtureType::default()
    }
}

pub(crate) fn intensity() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0))
}

pub(crate) fn colour() -> ParameterDefinition {
    ParameterDefinition::new(
        ParameterKind::ColorRgb,
        ParameterValue::rgb(1.0, 1.0, 1.0),
    )
}

/// Pan and tilt rest in the middle, which is where a head points when nothing is
/// driving it: straight along the rest direction its transform gives it.
pub(crate) fn pan() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::Pan, ParameterValue::Float(0.5))
}

pub(crate) fn tilt() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::Tilt, ParameterValue::Float(0.5))
}

pub(crate) fn a_fixture(
    name: &str,
    type_id: Uuid,
    address: FixtureAddress,
    position: Transform,
) -> Fixture {
    Fixture {
        id: id(),
        name: name.to_string(),
        fixture_type_id: type_id,
        address,
        position: Some(position),
        ..Fixture::default()
    }
}

pub(crate) fn capture(fixture: Uuid, kind: ParameterKind, value: ParameterValue) -> ParameterCapture {
    ParameterCapture {
        fixture_id: fixture,
        parameter_kind: kind,
        value,
        fade_in_ms: 0,
        fade_out_ms: 0,
        delay_in_ms: 0,
        effect: None,
        easing: Easing::Linear,
    }
}

pub(crate) fn a_cue(name: &str, number: f64, captures: Vec<ParameterCapture>) -> Cue {
    Cue {
        id: id(),
        name: name.to_string(),
        number,
        captures,
        follow_mode: FollowMode::Manual,
        fade_in_ms: 2_000,
        fade_out_ms: 0,
        is_active: false,
    }
}

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Haunt").await?;

    // One ordinary DMX fixture type, so the Patch tab has something in it and an
    // Art-Net output has something to send.
    let dimmer = a_type("Dimmer", vec![intensity()]);
    into.create("fixture_types", &dimmer).await?;

    // And a moving head, so there is something to puppeteer. Nothing binds a
    // channel: where a parameter sits belongs to a mode, and a type that names none
    // has the implicit one — intensity at 1, the colour across 2 to 4, pan at 5,
    // tilt at 6.
    let spot = a_type("Spot", vec![intensity(), colour(), pan(), tilt()]);
    into.create("fixture_types", &spot).await?;

    // Hung where the names say, in metres: X to the right as seen from front of
    // house, Y up, Z downstage towards the audience. Placed rather than nowhere, so
    // the Stage tab opens with a rig in it rather than three chips in a tray.
    for (index, (name, x, y, z)) in [
        ("Front left", -3.0, 4.5, 2.0),
        ("Front right", 3.0, 4.5, 2.0),
        ("Backlight", 0.0, 5.0, -3.0),
    ]
    .into_iter()
    .enumerate()
    {
        let fixture =
            a_fixture(name, dimmer.id, dmx(1, 1 + index as u16), at(x, y, z));
        into.create("fixtures", &fixture).await?;
    }

    // Aimed rather than merely placed: a moving head needs a rest direction for pan
    // and tilt to be angles away from.
    for (index, (name, x)) in [("Head left", -2.5), ("Head right", 2.5)].into_iter().enumerate() {
        let fixture = a_fixture(
            name,
            spot.id,
            dmx(1, 11 + index as u16 * spot.channel_count),
            aimed(x, 5.0, -1.0, DOWNSTAGE_AND_DOWN),
        );
        into.create("fixtures", &fixture).await?;
    }

    let rig = into.fixtures().await;
    let movers: Vec<&Fixture> =
        rig.iter().filter(|fixture| fixture.name.starts_with("Head")).collect();

    // A tempo for effects to follow. 120 bpm halved is one cycle a second: slow
    // enough to watch, fast enough to be obviously moving.
    let master = SpeedMaster {
        id: id(),
        name: "Chases".to_string(),
        bpm: 120.0,
        multiplier: 0.5,
        running: true,
        t0: now_ms(),
    };
    into.create("speed_masters", &master).await?;

    // One id across both heads, so the effects panel gathers them back into a single
    // editable effect rather than two unrelated sines.
    let chase = id();
    let sine = |fixture: &Fixture, phase: f32| ParameterCapture {
        effect: Some(EffectSpec {
            effect_id: chase,
            curve: Curve::Shape(Shape::Sine),
            rate: Rate::Master { id: master.id, multiplier: 1.0 },
            low: ParameterValue::rgb(0.4, 0.0, 0.0),
            high: ParameterValue::rgb(0.0, 0.2, 1.0),
            width: 0.5,
            direction: Direction::Forward,
            phase,
            spread: Spread::Linear,
            // A stored capture never carries an anchor: the cue's `went_at` is it, so
            // two consoles replaying this cue start the same cycle rather than each
            // remembering its own.
            t0: None,
        }),
        ..capture(fixture.id, ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0))
    };

    let level = |at: f32| {
        rig.iter()
            .map(|fixture| {
                capture(fixture.id, ParameterKind::Intensity, ParameterValue::Float(at))
            })
            .collect::<Vec<_>>()
    };

    let mut house = a_cue("House", 1.0, level(0.2));
    house.fade_out_ms = 2_000;
    let mut scare = a_cue("Scare", 2.0, level(1.0));
    scare.fade_in_ms = 3_000;
    scare.fade_out_ms = 1_500;
    // Everything up, and the two heads cycling through colour against each other on
    // the speed master.
    let mut possession = a_cue("Possession", 3.0, level(0.8));
    possession.fade_in_ms = 1_000;
    possession.fade_out_ms = 1_000;
    for (index, mover) in movers.iter().enumerate() {
        possession.captures.push(sine(mover, index as f32 * 0.5));
    }

    let cues = [house, scare, possession];
    for cue in &cues {
        into.create("cues", cue).await?;
    }

    let sequence = Sequence {
        id: id(),
        name: "Haunt".to_string(),
        cue_ids: cues.iter().map(|cue| cue.id).collect(),
        active_cue_index: None,
        went_at: None,
    };
    into.create("sequences", &sequence).await?;

    // Two graphs for the Flows panel. The first is a chain anyone can set off by
    // hand; the second is the thing a one-row-per-rule trigger could never say.
    //
    // Nodes carry their own coordinates rather than being laid out from an index,
    // because an `And` has two feeders and they cannot share a row.
    draw(
        into,
        "Panic button",
        &[
            (FlowNodeKind::Button, 40.0, 60.0),
            (FlowNodeKind::Delay { ms: 1_500 }, 280.0, 60.0),
            (FlowNodeKind::Action(TriggerAction::GoNext { sequence_id: sequence.id }), 520.0, 60.0),
        ],
        &[(0, 0, 1, 0), (1, 0, 2, 0)],
    )
    .await?;

    let watching = |fixture: &Fixture| {
        FlowNodeKind::Source(TriggerSource::Parameter {
            fixture_id: fixture.id,
            parameter: ParameterKind::Intensity,
        })
    };
    draw(
        into,
        "Both fronts up",
        &[
            (watching(&rig[0]), 40.0, 40.0),
            (watching(&rig[1]), 40.0, 180.0),
            (FlowNodeKind::And, 300.0, 100.0),
            (FlowNodeKind::Condition(TriggerCondition::RisingEdge), 540.0, 100.0),
            (
                FlowNodeKind::Action(TriggerAction::GoToCue {
                    sequence_id: sequence.id,
                    cue_id: cues[1].id,
                }),
                780.0,
                100.0,
            ),
        ],
        // Both watches into the two inputs of the And, then edge-detect the gate: it
        // fires when the second one comes up, not when either does.
        &[(0, 0, 2, 0), (1, 0, 2, 1), (2, 0, 3, 0), (3, 0, 4, 0)],
    )
    .await?;

    Ok(())
}

/// One flow graph: its nodes at the coordinates given, wired as `(from, port, to, port)`.
pub(crate) async fn draw(
    into: &Seeder,
    name: &str,
    nodes: &[(FlowNodeKind, f32, f32)],
    wires: &[(usize, u8, usize, u8)],
) -> Result<()> {
    let flow = Flow { id: id(), name: name.to_string(), enabled: true };
    into.create("flows", &flow).await?;

    let mut ids = Vec::new();
    for (kind, x, y) in nodes {
        let node = FlowNode {
            id: id(),
            flow_id: flow.id,
            kind: kind.clone(),
            x: *x,
            y: *y,
            active: false,
            last_fired_at: None,
        };
        ids.push(node.id);
        into.create("flow_nodes", &node).await?;
    }
    for (from, from_port, to, to_port) in wires {
        into.create(
            "flow_edges",
            &FlowEdge {
                id: id(),
                flow_id: flow.id,
                from_node: ids[*from],
                from_port: *from_port,
                to_node: ids[*to],
                to_port: *to_port,
            },
        )
        .await?;
    }
    Ok(())
}
