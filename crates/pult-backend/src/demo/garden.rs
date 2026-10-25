//! Heinz Horror Garten: a haunt run by its sensors.
//!
//! The other four demos are shows an operator runs. This one runs itself: a group
//! walks into a maze area, a PIR on an OpenHaunt node sees them, a flow counts them
//! in and — if the area was empty — plays that area's scare, which is a timeline with
//! a light cue, a relay prop, a scream in the area's speaker zone, a bang out of the
//! node's own loudspeaker and, in two of the areas, a skeleton speaking a recorded
//! take. The traffic light at the entrance goes green when the second area is free.
//! A phone surface shows all of it and resets an area whose count has drifted.
//!
//! **Nothing in it is a maze.** Every rule above is flows, variables and timelines —
//! show data an operator can read in the Flows panel and change — which is the whole
//! point of having built it: the first gig this console runs is a haunt, and the test
//! of whether the console is general is whether a haunt needs any code. It does not.
//!
//! # The nodes
//!
//! Ten V0 prototypes, five of each revision, patched the way adopting them would patch
//! them: a fixture per head, ids and types derived from the serial and the description
//! the node publishes. So a real node (or `openhaunt-node-sim --module v0`) with one of
//! these serials coming up on the network finds its rows already here, and the strips
//! this show runs in `sacn` mode are put into that mode on the node, because the show
//! is the truth about a strip. [`V0_INFO`] is what a V0 says about itself, held equal to
//! the simulator's by a test.
//!
//! # The sound and the takes
//!
//! Generated here, deterministically, because a demo carries no files: wind, screams,
//! a bang, a laugh, and a skeleton's take recorded against the laugh. On a station with
//! no asset store the timelines are seeded silent and without the take.

use std::collections::HashSet;

use anyhow::Result;
use pult_schema::types::{
    cue::ParameterCapture,
    effect::{EffectSpec, Spread},
    effect_template::{default_effect_templates, EffectTemplate},
    fixture::{Fixture, FixtureAddress, ParameterKind, ParameterValue, Vec3},
    flow::{FlowNodeKind, IfRunning, TriggerAction, TriggerCondition, TriggerSource},
    group::Element,
    mount::Mount,
    openhaunt::{self, NodeDescription},
    parameter_key,
    pixels::{self, NodePixels, PixelStrip, StripMode},
    scene::{SceneObject, SceneObjectKind, Transform},
    surface::{ColorStop, Surface, SurfaceTile, TileSize, TileTarget},
    timeline::{
        AtEnd, AudioClip, AudioDestination, NodeAudioMode, Timeline, TimelineAction, TimelineEnd,
        TimelineEvent, TimelineSource, TimelineTrack,
    },
    variable::{Variable, VariableKind, VariableOp},
    zone::{SpeakerZone, ZoneNode},
    CueShot, ParameterDefinition,
};
use uuid::Uuid;

use super::{
    id,
    kit::{
        a_clamped_fixture, a_cue, a_fixture, a_piece, a_stack, a_type, at, boom, capture, color,
        dmx, draw, facing, hue, intensity, level, pan, posed, production, truss_run, under,
        weighing,
    },
    Seeder,
};

/// Each maze area's color, the same on its zone and wherever else the area is drawn.
const AREA_COLORS: [&str; 4] = ["#ef4444", "#f97316", "#eab308", "#06b6d4"];

/// What a V0 prototype publishes at `/api/v1/info`, both strips in `effect` mode: the
/// module firmware 0.3 calls `0x0F01`. Only the parts the console reads.
pub const V0_INFO: &str = r#"{
  "module": { "type": "0x0f01", "name": "Node V0" },
  "heads": [
    { "head": 1, "name": "Strip 1", "kind": "pixel-strip" },
    { "head": 2, "name": "Strip 2", "kind": "pixel-strip" }
  ],
  "ports": [
    { "port": 0, "name": "Relay 1", "access": "readwrite", "dataType": "boolean", "default": 0.0, "class": "switch" },
    { "port": 1, "name": "Relay 2", "access": "readwrite", "dataType": "boolean", "default": 0.0, "class": "switch" },
    { "port": 2, "name": "Relay 3", "access": "readwrite", "dataType": "boolean", "default": 0.0, "class": "switch" },
    { "port": 3, "name": "Relay 4", "access": "readwrite", "dataType": "boolean", "default": 0.0, "class": "switch" },
    { "port": 4, "name": "Sensor 1", "access": "readonly", "dataType": "boolean", "class": "contact" },
    { "port": 5, "name": "Sensor 2", "access": "readonly", "dataType": "boolean", "class": "contact" },
    { "port": 6, "name": "Color", "access": "readwrite", "dataType": "color", "class": "color", "head": 1 },
    { "port": 7, "name": "Brightness", "access": "readwrite", "dataType": "number", "unit": "percent", "minimum": 0.0, "maximum": 1.0, "default": 0.0, "class": "intensity", "head": 1 },
    { "port": 8, "name": "Color 2", "access": "readwrite", "dataType": "color", "head": 1 },
    { "port": 9, "name": "Effect", "access": "readwrite", "dataType": "string", "class": "pixel-effect", "head": 1 },
    { "port": 10, "name": "Speed", "access": "readwrite", "dataType": "number", "unit": "hertz", "minimum": 0.0, "maximum": 10.0, "default": 1.0, "class": "speed", "head": 1 },
    { "port": 11, "name": "Size", "access": "readwrite", "dataType": "number", "unit": "percent", "minimum": 0.0, "maximum": 1.0, "default": 0.25, "class": "size", "head": 1 },
    { "port": 12, "name": "Reverse", "access": "readwrite", "dataType": "boolean", "default": 0.0, "class": "reverse", "head": 1 },
    { "port": 13, "name": "Mirror", "access": "readwrite", "dataType": "boolean", "default": 0.0, "class": "mirror", "head": 1 },
    { "port": 14, "name": "Segments", "access": "readwrite", "dataType": "number", "unit": "unitless", "minimum": 1.0, "maximum": 16.0, "default": 1.0, "class": "segments", "head": 1 },
    { "port": 15, "name": "Color", "access": "readwrite", "dataType": "color", "class": "color", "head": 2 },
    { "port": 16, "name": "Brightness", "access": "readwrite", "dataType": "number", "unit": "percent", "minimum": 0.0, "maximum": 1.0, "default": 0.0, "class": "intensity", "head": 2 },
    { "port": 17, "name": "Color 2", "access": "readwrite", "dataType": "color", "head": 2 },
    { "port": 18, "name": "Effect", "access": "readwrite", "dataType": "string", "class": "pixel-effect", "head": 2 },
    { "port": 19, "name": "Speed", "access": "readwrite", "dataType": "number", "unit": "hertz", "minimum": 0.0, "maximum": 10.0, "default": 1.0, "class": "speed", "head": 2 },
    { "port": 20, "name": "Size", "access": "readwrite", "dataType": "number", "unit": "percent", "minimum": 0.0, "maximum": 1.0, "default": 0.25, "class": "size", "head": 2 },
    { "port": 21, "name": "Reverse", "access": "readwrite", "dataType": "boolean", "default": 0.0, "class": "reverse", "head": 2 },
    { "port": 22, "name": "Mirror", "access": "readwrite", "dataType": "boolean", "default": 0.0, "class": "mirror", "head": 2 },
    { "port": 23, "name": "Segments", "access": "readwrite", "dataType": "number", "unit": "unitless", "minimum": 1.0, "maximum": 16.0, "default": 1.0, "class": "segments", "head": 2 }
  ],
  "pixels": {
    "effects": ["solid", "gradient", "chase", "comet", "wave", "breathe", "fire", "candle", "sparkle", "lightning"],
    "orders": ["RGB", "RBG", "GRB", "GBR", "BRG", "BGR", "RGBW", "GRBW"],
    "max_count": 1024,
    "multicast_universes": 4,
    "strips": [
      { "output": 0, "head": 1, "mode": "effect", "count": 60, "order": "GRB", "universe": 0, "address": 1, "hold_ms": 10000 },
      { "output": 1, "head": 2, "mode": "effect", "count": 60, "order": "GRB", "universe": 0, "address": 1, "hold_ms": 10000 }
    ]
  }
}"#;

/// The module type a V0 reports, which every type id it produces is derived from.
pub const V0_MODULE: u16 = 0x0f01;

/// What a V0 says, and the vocabulary of its strips' `Effect` port.
pub fn v0_description() -> (NodeDescription, NodePixels) {
    let info: serde_json::Value = serde_json::from_str(V0_INFO).expect("V0_INFO is JSON");
    let description: NodeDescription =
        serde_json::from_value(info.clone()).expect("V0_INFO is a node description");
    let pixels = NodePixels::from_info(&info).expect("V0_INFO has pixels");
    (description, pixels)
}

/// The serials, in the order the nodes are listed below. The prefix is the revision:
/// five V0.1 and five V0.2, the gig's boards.
pub const SERIALS: [&str; 10] = [
    "v02a01", "v02a02", "v01a03", "v01a04", "v02a05", "v01a06", "v01a07", "v02a08", "v01a09",
    "v02a10",
];

/// One node, and what this show uses it for.
struct NodeRole {
    serial: &'static str,
    name: &'static str,
    at: Vec3,
    /// What each strip is patched as: `None` leaves it as the node came, which is a
    /// node effect.
    strips: [Option<StripPatch>; 2],
}

#[derive(Clone, Copy)]
struct StripPatch {
    mode: StripMode,
    count: u16,
    universe: u16,
    /// Where the strip starts, and the pitch it runs along world X at.
    start: Vec3,
    pitch: f32,
}

fn effect_strip(start: Vec3) -> Option<StripPatch> {
    Some(StripPatch { mode: StripMode::Effect, count: 60, universe: 0, start, pitch: 1.0 / 60.0 })
}

fn sacn_strip(universe: u16, start: Vec3) -> Option<StripPatch> {
    Some(StripPatch { mode: StripMode::Sacn, count: 60, universe, start, pitch: 0.1 })
}

/// One node's fixtures once patched.
struct Node {
    serial: &'static str,
    /// Head 0: the relays and the two sensors.
    node: Uuid,
    /// The strips' head fixtures, and for an sACN strip its pixels along it.
    strips: [Uuid; 2],
    cells: [Vec<Element>; 2],
}

// ── The look of the place ────────────────────────────────────────────────────

/// Where the maze runs: along X, between walls at these two depths.
const MAZE_FRONT: f32 = 0.0;
const MAZE_BACK: f32 = -4.0;
/// Each area is six meters of path, the first starting here.
const MAZE_START: f32 = -12.0;
const AREA_LENGTH: f32 = 6.0;

fn area_center(area: usize) -> f32 {
    MAZE_START + AREA_LENGTH * (area as f32 + 0.5)
}

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Heinz Horror Garten").await?;

    // ── Types ────────────────────────────────────────────────────────────────

    let par = weighing(
        a_type("RGB par", vec![intensity(), color()]),
        2.5,
        60.0,
        (0.22, 0.25, 0.22),
    );
    into.create("fixture_types", &par).await?;
    // A bought animatronic: a jaw, a head that turns, two arms. Generic, because the
    // real ones arrive with a channel list on a sheet of paper and no GDTF.
    let skeleton = weighing(
        a_type(
            "Animatronic skeleton",
            vec![
                named("Jaw", 0.0),
                pan(),
                named("Left arm", 0.0),
                named("Right arm", 0.0),
            ],
        ),
        14.0,
        90.0,
        (0.5, 1.8, 0.35),
    );
    into.create("fixture_types", &skeleton).await?;

    // ── The room ─────────────────────────────────────────────────────────────

    // The entrance gate: two truss towers and a bar across them, with the gate pars
    // on it looking out at the queue.
    let gate_at = Vec3 { x: -14.0, y: 0.0, z: 12.0 };
    boom(into, "Gate tower left", None, Vec3 { x: gate_at.x - 3.0, y: 1.5, z: gate_at.z }, 3.0)
        .await?;
    boom(into, "Gate tower right", None, Vec3 { x: gate_at.x + 3.0, y: 1.5, z: gate_at.z }, 3.0)
        .await?;
    let header =
        truss_run(into, "Gate header", None, Vec3 { x: gate_at.x, y: 3.3, z: gate_at.z }, 6.0)
            .await?;

    // Eight IBC containers in a row across the garden, each a 1.2 m tank lit from
    // inside. A deck stretched to a tank's height is the nearest piece the catalog
    // has to a plastic cube in a steel cage.
    let mut ibcs = Vec::new();
    for n in 0..8 {
        let x = -10.5 + 3.0 * n as f32;
        let ibc = SceneObject {
            id: id(),
            name: format!("IBC {}", n + 1),
            kind: SceneObjectKind::Group,
            transform: at(x, 0.0, 8.0),
            parent: None,
            layer: None,
            class: None,
            geometry: Vec::new(),
            symbol: None,
            catalog: None,
            properties: serde_json::Value::Null,
            locked: false,
            weight_kg: None,
        };
        into.create("scene_objects", &ibc).await?;
        let tank = SceneObject {
            id: id(),
            name: format!("IBC {} tank", n + 1),
            parent: Some(ibc.id),
            transform: Transform {
                position: Vec3 { x: 0.0, y: 1.16, z: 0.0 },
                scale: Vec3 { x: 1.2, y: 5.8, z: 1.0 },
                ..Transform::default()
            },
            ..catalog_piece("deck-1x1")
        };
        into.create("scene_objects", &tank).await?;
        ibcs.push(ibc.id);
    }

    // The maze: a path along X between two walls, four areas of six meters.
    for area in 0..4 {
        for side in [MAZE_FRONT, MAZE_BACK] {
            for panel in 0..3 {
                let x = MAZE_START + AREA_LENGTH * area as f32 + 1.0 + 2.0 * panel as f32;
                // The way in to area 1 is a gap in the front wall, and so is the way out
                // of area 4.
                let doorway =
                    side == MAZE_FRONT && ((area == 0 && panel == 0) || (area == 3 && panel == 2));
                if doorway {
                    continue;
                }
                a_piece(
                    into,
                    &format!("Area {} wall", area + 1),
                    "wall-2x1",
                    Transform {
                        position: Vec3 { x, y: 0.0, z: side },
                        scale: Vec3 { x: 1.0, y: 2.2, z: 1.0 },
                        ..Transform::default()
                    },
                    None,
                )
                .await?;
            }
        }
    }

    // ── The nodes ────────────────────────────────────────────────────────────

    let wall_top = 2.25;
    let roles = [
        NodeRole {
            serial: SERIALS[0],
            name: "Area 1",
            at: Vec3 { x: area_center(0), y: 0.3, z: MAZE_BACK + 0.4 },
            strips: [effect_strip(Vec3 { x: area_center(0) - 3.0, y: wall_top, z: MAZE_BACK + 0.05 }), None],
        },
        NodeRole {
            serial: SERIALS[1],
            name: "Area 2",
            at: Vec3 { x: area_center(1), y: 0.3, z: MAZE_BACK + 0.4 },
            strips: [sacn_strip(10, Vec3 { x: area_center(1) - 3.0, y: wall_top, z: MAZE_BACK + 0.05 }), None],
        },
        NodeRole {
            serial: SERIALS[2],
            name: "Area 3",
            at: Vec3 { x: area_center(2), y: 0.3, z: MAZE_BACK + 0.4 },
            strips: [effect_strip(Vec3 { x: area_center(2) - 3.0, y: wall_top, z: MAZE_BACK + 0.05 }), None],
        },
        NodeRole {
            serial: SERIALS[3],
            name: "Area 4",
            at: Vec3 { x: area_center(3), y: 0.3, z: MAZE_BACK + 0.4 },
            strips: [None, None],
        },
        NodeRole {
            serial: SERIALS[4],
            name: "Entrance",
            at: Vec3 { x: MAZE_START - 0.6, y: 0.3, z: MAZE_FRONT + 0.6 },
            strips: [sacn_strip(11, Vec3 { x: MAZE_START, y: wall_top, z: MAZE_FRONT + 0.05 }), None],
        },
        NodeRole { serial: SERIALS[5], name: "Graveyard", at: Vec3 { x: -8.0, y: 0.3, z: 4.0 }, strips: [None, None] },
        NodeRole { serial: SERIALS[6], name: "Crypt", at: Vec3 { x: 8.0, y: 0.3, z: 4.0 }, strips: [None, None] },
        NodeRole {
            serial: SERIALS[7],
            name: "Pumpkin patch",
            at: Vec3 { x: 0.0, y: 0.3, z: 5.0 },
            strips: [effect_strip(Vec3 { x: -1.0, y: 0.1, z: 5.5 }), None],
        },
        NodeRole { serial: SERIALS[8], name: "Hanging tree", at: Vec3 { x: -4.0, y: 0.3, z: 11.0 }, strips: [None, None] },
        NodeRole {
            serial: SERIALS[9],
            name: "Exit",
            at: Vec3 { x: MAZE_START + 4.0 * AREA_LENGTH + 0.6, y: 0.3, z: MAZE_FRONT + 0.6 },
            strips: [None, None],
        },
    ];

    let (description, node_pixels) = v0_description();
    let mut made_types: HashSet<Uuid> = HashSet::new();
    let mut nodes = Vec::new();
    for role in &roles {
        nodes.push(patch_node(into, role, &description, &node_pixels, &mut made_types).await?);
    }
    let node = |name: &str| -> &Node {
        let index = roles.iter().position(|r| r.name == name).expect("a node by that name");
        &nodes[index]
    };

    // ── The DMX side ─────────────────────────────────────────────────────────

    // Universe 1, out of an Art-Net node in the shed: the gate, the tanks, the two
    // skeletons. The strips the console draws are universes 10 and 11.
    let mut next = 1u16;
    let mut address = |channels: u16| {
        let a = dmx(1, next);
        next += channels;
        a
    };

    let mut gate = Vec::new();
    for (n, along) in [-2.25f32, -0.75, 0.75, 2.25].into_iter().enumerate() {
        let mount = Mount::along(along);
        let fixture = a_clamped_fixture(
            &format!("Gate par {}", n + 1),
            par.id,
            address(par.channel_count),
            header,
            under(mount, facing::DOWNSTAGE),
            mount,
        );
        into.create("fixtures", &fixture).await?;
        gate.push(fixture.id);
    }

    // Standing on the bottom of each tank, looking up into it.
    let mut tanks = Vec::new();
    for (n, ibc) in ibcs.iter().enumerate() {
        let fixture = Fixture {
            parent: Some(*ibc),
            ..a_fixture(
                &format!("IBC {} par", n + 1),
                par.id,
                address(par.channel_count),
                Transform::facing(Vec3 { x: 0.0, y: 0.35, z: 0.0 }, facing_up()),
            )
        };
        into.create("fixtures", &fixture).await?;
        tanks.push(fixture.id);
    }

    let mut skeletons = Vec::new();
    for (n, area) in [(1, 1usize), (2, 3usize)] {
        let fixture = a_fixture(
            &format!("Skeleton {n}"),
            skeleton.id,
            address(skeleton.channel_count),
            at(area_center(area) + 1.5, 0.0, MAZE_BACK + 0.6),
        );
        into.create("fixtures", &fixture).await?;
        skeletons.push(fixture.id);
    }

    // ── What the flows remember ──────────────────────────────────────────────

    let inside = [variable("Area 1 inside", VariableKind::Number, 0.0), variable("Area 2 inside", VariableKind::Number, 0.0)];
    for v in &inside {
        into.create("variables", v).await?;
    }
    let open = variable("Maze open", VariableKind::Flag, 1.0);
    into.create("variables", &open).await?;

    // ── The look ─────────────────────────────────────────────────────────────

    let templates = default_effect_templates();
    let template = |name: &str| -> &EffectTemplate {
        templates.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("no {name} template"))
    };
    let kinds_of = |fixture: Uuid| -> Vec<ParameterKind> {
        if gate.contains(&fixture) || tanks.contains(&fixture) {
            vec![ParameterKind::Intensity, ParameterKind::ColorRgb]
        } else {
            Vec::new()
        }
    };

    let orange = ParameterValue::rgb(1.0, 0.35, 0.0);
    let purple = ParameterValue::rgb(0.5, 0.0, 1.0);
    let landscape = ["Graveyard", "Crypt", "Pumpkin patch", "Hanging tree", "Exit"];

    let mut halloween = a_cue("Halloween", 1.0, Vec::new());
    halloween.fade_in_ms = 4_000;
    // The tanks: three on fire, three by candle, two flickering, alternately orange
    // and purple underneath.
    for (n, tank) in tanks.iter().enumerate() {
        halloween.captures.push(capture(*tank, ParameterKind::ColorRgb, if n % 2 == 0 { orange.clone() } else { purple.clone() }));
    }
    let fire: Vec<Uuid> = tanks.iter().copied().step_by(3).collect();
    let candle: Vec<Uuid> = tanks.iter().copied().skip(1).step_by(3).collect();
    let flicker: Vec<Uuid> = tanks.iter().copied().skip(2).step_by(3).collect();
    halloween.captures.extend(from_template(template("Fire"), &whole(&fire), &kinds_of));
    halloween.captures.extend(from_template(template("Candle"), &whole(&candle), &kinds_of));
    halloween.captures.extend(from_template(template("Flicker"), &whole(&flicker), &kinds_of));
    // The gate breathes purple.
    for par in &gate {
        halloween.captures.push(hue(*par, 0.5, 0.0, 1.0));
    }
    halloween.captures.extend(from_template(template("Breathing"), &whole(&gate), &|_| vec![ParameterKind::Intensity]));
    // The lanterns on the landscape nodes' relays, and the pumpkins on fire.
    for name in landscape {
        for relay in 0..2 {
            halloween.captures.push(capture(node(name).node, ParameterKind::Switch(relay), ParameterValue::Bool(true)));
        }
    }
    halloween.captures.extend(node_effect(node("Pumpkin patch").strips[0], "fire", (1.0, 0.3, 0.0), 0.8, 1.5));

    let mut late = a_cue("Late night", 2.0, Vec::new());
    late.fade_in_ms = 10_000;
    for tank in &tanks {
        late.captures.push(capture(*tank, ParameterKind::ColorRgb, ParameterValue::rgb(0.6, 0.0, 0.0)));
    }
    late.captures.extend(from_template(template("Flicker"), &whole(&tanks), &|_| vec![ParameterKind::Intensity]));
    for par in &gate {
        late.captures.push(level(*par, 0.3));
    }

    let mut out = a_cue("Out", 3.0, Vec::new());
    out.fade_in_ms = 3_000;
    for fixture in tanks.iter().chain(&gate) {
        out.captures.push(level(*fixture, 0.0));
    }
    for name in landscape {
        for relay in 0..2 {
            out.captures.push(capture(node(name).node, ParameterKind::Switch(relay), ParameterValue::Bool(false)));
        }
    }
    out.captures.push(capture(node("Pumpkin patch").strips[0], ParameterKind::Intensity, ParameterValue::Float(0.0)));

    let beauty = vec![CueShot::at(halloween.id, 6_000)];
    let garden = a_stack(into, "Garden", vec![halloween, late, out], true).await?;

    // ── The areas: a stack each, resting, and a scare to Go into ─────────────

    let mut scares = Vec::new();
    let props = ["Coffin lid", "Pneumatic arm", "Spider drop", "Fog burst"];
    for area in 0..4 {
        let this = node(&format!("Area {}", area + 1));
        let mut rest = a_cue("Rest", 1.0, Vec::new());
        rest.fade_in_ms = 2_000;
        let mut scare = a_cue("Scare", 2.0, Vec::new());
        scare.fade_in_ms = 0;
        for relay in 0..2 {
            rest.captures.push(capture(this.node, ParameterKind::Switch(relay), ParameterValue::Bool(false)));
            scare.captures.push(capture(this.node, ParameterKind::Switch(relay), ParameterValue::Bool(true)));
        }
        match area {
            // A strip the node draws: graveyard green until somebody walks in, and
            // lightning when they do.
            0 | 2 => {
                rest.captures.extend(node_effect(this.strips[0], "candle", (0.1, 0.8, 0.2), 0.5, 0.8));
                scare.captures.extend(node_effect(this.strips[0], "lightning", (0.9, 0.9, 1.0), 1.0, 6.0));
            }
            // A strip the console draws: a dim red chase that turns into a run of
            // white the moment the area wakes.
            1 => {
                let cells = &this.cells[0];
                // The pool's chase, run in color: a pixel has no dimmer of its own.
                let mut chase = template("Chase").clone();
                for layer in &mut chase.layers {
                    layer.kind = ParameterKind::ColorRgb;
                    layer.low = ParameterValue::rgb(0.0, 0.0, 0.0);
                    layer.high = ParameterValue::rgb(0.6, 0.0, 0.0);
                }
                rest.captures.extend(from_template(&chase, cells, &|_| vec![ParameterKind::ColorRgb]));
                for cell in cells {
                    let kind = ParameterKind::ColorRgb.in_cell(cell.cell.unwrap_or(1));
                    scare.captures.push(capture(cell.fixture, kind, ParameterValue::rgb(1.0, 1.0, 1.0)));
                }
            }
            _ => {}
        }
        // The skeletons rest still, so the take is what moves them.
        if let Some(skeleton) = match area {
            1 => Some(skeletons[0]),
            3 => Some(skeletons[1]),
            _ => None,
        } {
            rest.captures.push(capture(skeleton, named_kind("Jaw"), ParameterValue::Float(0.0)));
            rest.captures.push(capture(skeleton, ParameterKind::Pan, ParameterValue::Float(0.5)));
        }
        let (rest_id, scare_id) = (rest.id, scare.id);
        let stack = a_stack(into, &format!("Area {}", area + 1), vec![rest, scare], true).await?;
        scares.push((stack, rest_id, scare_id, props[area]));
    }

    // ── The scares, as timelines ─────────────────────────────────────────────

    let rate = 22_050;
    let wind = into.put("audio/wav", &wav(&ambience(rate, 16.0), rate)).await;
    let scream = into.put("audio/wav", &wav(&a_scream(rate, 2.6), rate)).await;
    let bang = into.put("audio/wav", &wav(&a_bang(rate, 1.2), rate)).await;
    let laugh = into.put("audio/wav", &wav(&a_laugh(rate, 4.0), rate)).await;

    // ── Where it comes out ───────────────────────────────────────────────────
    //
    // Speaker zones, named for the garden rather than the sound card, mapped for the
    // station seeding the show: Dante channels 1–2 for the row of tanks, one channel
    // per maze area after that, and the entrance on 7. A spare Mac maps the same zones
    // to its own card. The two areas with a skeleton have its node's loudspeaker in the
    // zone too, so its laugh comes from its own mouth as well as from the trees.
    let station = into.station();
    let zone = |name: &str, color: &str, channels: Vec<u16>, nodes: Vec<ZoneNode>| {
        let mut map = std::collections::BTreeMap::new();
        if let Some(station) = station {
            map.insert(station, channels.clone());
        }
        SpeakerZone {
            id: id(),
            name: name.into(),
            color: Some(color.into()),
            width: channels.len() as u16,
            channels: map,
            nodes,
            notes: String::new(),
        }
    };
    let mut zones = vec![zone("IBC row", "#8b5cf6", vec![1, 2], Vec::new())];
    for area in 0..4 {
        let speaks = area == 1 || area == 3;
        let nodes = if speaks {
            let this = node(&format!("Area {}", area + 1));
            vec![ZoneNode { serial: this.serial.to_string(), mode: NodeAudioMode::Clip }]
        } else {
            Vec::new()
        };
        zones.push(zone(&format!("Area {}", area + 1), AREA_COLORS[area], vec![3 + area as u16], nodes));
    }
    zones.push(zone("Entrance", "#22c55e", vec![7], Vec::new()));
    for each in &zones {
        into.create("speaker_zones", each).await?;
    }
    let zone_of = |name: &str| AudioDestination::Zone {
        zone: zones.iter().find(|z| z.name == name).map(|z| z.id).unwrap_or_default(),
    };

    let ambient = a_timeline("Ambience", Some(TimelineEnd { at_ms: 16_000, then: AtEnd::Loop }));
    let ambient = Timeline {
        clips: wind.iter().map(|sha| a_clip("Garden wind", sha, 0, zone_of("IBC row"))).collect(),
        ..ambient
    };
    into.create("timelines", &ambient).await?;

    let mut scare_timelines = Vec::new();
    for (area, (stack, rest, scare, prop)) in scares.iter().enumerate() {
        let speaks = area == 1 || area == 3;
        let length = if speaks { 8_000 } else { 6_000 };
        let zone = zone_of(&format!("Area {}", area + 1));
        let this = node(&format!("Area {}", area + 1));
        let on_the_node = AudioDestination::Node { serial: this.serial.to_string(), mode: NodeAudioMode::Clip };
        let mut clips = Vec::new();
        if let Some(sha) = &bang {
            clips.push(a_clip(&format!("{prop} bang"), sha, 0, on_the_node));
        }
        let voice = if speaks { &laugh } else { &scream };
        if let Some(sha) = voice {
            clips.push(a_clip(if speaks { "Skeleton laugh" } else { "Scream" }, sha, 300, zone));
        }
        let mut tracks = Vec::new();
        if speaks {
            let skeleton = if area == 1 { skeletons[0] } else { skeletons[1] };
            let recording = skeleton_take(skeleton, 300);
            let take = pult_render::track::encode(&recording);
            if let Some(sha) = into.put(crate::infra::assets::TRACK_MIME, &take).await {
                // A row per parameter, the way a recorded take is stored.
                let whole = TimelineTrack {
                    id: id(),
                    name: format!("Skeleton {} take", if area == 1 { 1 } else { 2 }),
                    asset: sha,
                    input_id: None,
                    offset_ms: 0,
                    enabled: true,
                    fixture_id: None,
                    key: None,
                    from_ms: None,
                    to_ms: None,
                };
                let keys: Vec<(Uuid, String)> =
                    recording.keys.iter().map(|k| (k.fixture_id, k.key.clone())).collect();
                tracks.extend(pult_schema::types::timeline::split_row(&whole, &keys));
            }
        }
        let timeline = Timeline {
            clips,
            tracks,
            events: vec![
                TimelineEvent { id: id(), at_ms: 0, action: TimelineAction::GoToCue { sequence_id: *stack, cue_id: *scare } },
                TimelineEvent {
                    id: id(),
                    at_ms: length as u32 - 1_000,
                    action: TimelineAction::GoToCue { sequence_id: *stack, cue_id: *rest },
                },
            ],
            ..a_timeline(&format!("Area {} scare", area + 1), Some(TimelineEnd { at_ms: length, then: AtEnd::Stop }))
        };
        into.create("timelines", &timeline).await?;
        scare_timelines.push(timeline.id);
    }

    // ── The flows ────────────────────────────────────────────────────────────

    let pir = |fixture: Uuid, port: u8| {
        FlowNodeKind::Source(TriggerSource::Parameter { fixture_id: fixture, parameter: ParameterKind::Contact(port) })
    };
    let read = |variable: Uuid| FlowNodeKind::Source(TriggerSource::Variable { variable_id: variable });
    let act = |action: TriggerAction| FlowNodeKind::Action(action);
    let play = |timeline: Uuid| act(TriggerAction::PlayTimeline { timeline_id: timeline, if_running: IfRunning::Ignore });
    let count = |variable: Uuid, by: f64| act(TriggerAction::SetVariable { variable_id: variable, op: VariableOp::Add(by) });

    // Areas 1 and 2: a PIR at the way in and one at the way out. The first group into
    // an empty area sets the scare off; whoever follows them in while they are still
    // there does not, and neither does anybody while the maze is shut.
    for area in 0..2 {
        let this = node(&format!("Area {}", area + 1));
        draw(
            into,
            &format!("Area {}", area + 1),
            &[
                (pir(this.node, 4), 40.0, 40.0),
                (FlowNodeKind::Condition(TriggerCondition::RisingEdge), 300.0, 40.0),
                (read(inside[area].id), 40.0, 240.0),
                (FlowNodeKind::Not, 300.0, 240.0),
                (read(open.id), 40.0, 400.0),
                (FlowNodeKind::And, 560.0, 320.0),
                (FlowNodeKind::Gate, 820.0, 180.0),
                (play(scare_timelines[area]), 1080.0, 180.0),
                (count(inside[area].id, 1.0), 820.0, -120.0),
                (pir(this.node, 5), 40.0, 600.0),
                (FlowNodeKind::Condition(TriggerCondition::RisingEdge), 300.0, 600.0),
                (count(inside[area].id, -1.0), 560.0, 600.0),
            ],
            &[
                (0, 0, 1, 0),
                (2, 0, 3, 0),
                (3, 0, 5, 0),
                (4, 0, 5, 1),
                (1, 0, 6, 0),
                (5, 0, 6, 1),
                (6, 0, 7, 0),
                (1, 0, 8, 0),
                (9, 0, 10, 0),
                (10, 0, 11, 0),
            ],
        )
        .await?;
    }

    // Areas 3 and 4: one PIR, which runs the action. A scare already running ignores
    // the next group, so the lockout is the timeline's own length.
    for area in 2..4 {
        let this = node(&format!("Area {}", area + 1));
        draw(
            into,
            &format!("Area {}", area + 1),
            &[
                (pir(this.node, 4), 40.0, 40.0),
                (FlowNodeKind::Condition(TriggerCondition::RisingEdge), 300.0, 40.0),
                (read(open.id), 40.0, 240.0),
                (FlowNodeKind::Gate, 560.0, 120.0),
                (play(scare_timelines[area]), 820.0, 120.0),
            ],
            &[(0, 0, 1, 0), (1, 0, 3, 0), (2, 0, 3, 1), (3, 0, 4, 0)],
        )
        .await?;
    }

    // The traffic light: red as soon as anybody is in area 2 or the maze is shut;
    // green when area 2 empties with the maze open, or the maze opens with area 2
    // empty. Green is where the lamp rests, so a show that has just opened says "go".
    let entrance = node("Entrance").node;
    let lamp = |relay: u8, lit: bool| {
        act(TriggerAction::SetParameter {
            fixture_id: entrance,
            parameter: ParameterKind::Switch(relay),
            value: ParameterValue::Bool(lit),
        })
    };
    draw(
        into,
        "Traffic light",
        &[
            (read(inside[1].id), 40.0, 160.0),
            (FlowNodeKind::Condition(TriggerCondition::RisingEdge), 300.0, 0.0),
            (FlowNodeKind::Condition(TriggerCondition::FallingEdge), 300.0, 200.0),
            (read(open.id), 40.0, 560.0),
            (FlowNodeKind::Condition(TriggerCondition::RisingEdge), 300.0, 620.0),
            (FlowNodeKind::Condition(TriggerCondition::FallingEdge), 300.0, 420.0),
            (FlowNodeKind::Not, 300.0, 820.0),
            (FlowNodeKind::Gate, 580.0, 260.0),
            (FlowNodeKind::Gate, 580.0, 700.0),
            (lamp(0, true), 860.0, 0.0),
            (lamp(1, false), 860.0, 260.0),
            (lamp(1, true), 860.0, 520.0),
            (lamp(0, false), 860.0, 780.0),
        ],
        &[
            (0, 0, 1, 0),
            (0, 0, 2, 0),
            (3, 0, 4, 0),
            (3, 0, 5, 0),
            (0, 0, 6, 0),
            (2, 0, 7, 0),
            (3, 0, 7, 1),
            (4, 0, 8, 0),
            (6, 0, 8, 1),
            (1, 0, 9, 0),
            (1, 0, 10, 0),
            (5, 0, 9, 0),
            (5, 0, 10, 0),
            (7, 0, 11, 0),
            (7, 0, 12, 0),
            (8, 0, 11, 0),
            (8, 0, 12, 0),
        ],
    )
    .await?;

    // ── The phone ────────────────────────────────────────────────────────────

    let green = "#22c55e";
    let red = "#ef4444";
    let occupancy = vec![
        ColorStop { at: 0.0, color: green.into() },
        ColorStop { at: 1.0, color: red.into() },
    ];
    let mut tiles = vec![
        tile("Entrance", None, TileSize::Full, TileTarget::Heading),
        tile("Red", Some(red), TileSize::Half, TileTarget::Parameter { fixture_id: entrance, parameter: ParameterKind::Switch(0) }),
        tile("Green", Some(green), TileSize::Half, TileTarget::Parameter { fixture_id: entrance, parameter: ParameterKind::Switch(1) }),
    ];
    for (area, v) in inside.iter().enumerate() {
        tiles.push(tile(&format!("Area {}", area + 1), None, TileSize::Full, TileTarget::Heading));
        tiles.push(tile("Inside", None, TileSize::Half, TileTarget::VariableReadout { variable_id: v.id, stops: occupancy.clone() }));
        tiles.push(tile("Reset", None, TileSize::Half, TileTarget::VariableSetter { variable_id: v.id, op: VariableOp::Reset }));
    }
    tiles.push(tile("Areas 3 and 4", None, TileSize::Full, TileTarget::Heading));
    for area in 2..4 {
        tiles.push(tile(
            &format!("Scare {}", area + 1),
            None,
            TileSize::Half,
            TileTarget::Timeline { timeline_id: scare_timelines[area], from_start: true },
        ));
    }
    tiles.extend([
        tile("Maze", None, TileSize::Full, TileTarget::Heading),
        tile(
            "Open",
            None,
            TileSize::Half,
            TileTarget::VariableReadout {
                variable_id: open.id,
                stops: vec![ColorStop { at: 0.0, color: red.into() }, ColorStop { at: 1.0, color: green.into() }],
            },
        ),
        tile("Open / shut", None, TileSize::Half, TileTarget::VariableSetter { variable_id: open.id, op: VariableOp::Toggle }),
        tile("Ambience", None, TileSize::Full, TileTarget::Timeline { timeline_id: ambient.id, from_start: false }),
        tile("Garden Go", None, TileSize::Half, TileTarget::SequenceGo { sequence_id: garden }),
        tile("All off", Some(red), TileSize::Half, TileTarget::SequenceOff { sequence_id: garden }),
    ]);
    into.create("surfaces", &Surface { id: id(), name: "Maze".into(), tiles, color: None }).await?;

    into.describe_the_production(production(
        "Heinz Horror Garten",
        "Heinz's garden",
        "Behind the barn, past the pumpkins",
        "30.10.2026 - 01.11.2026",
        "Rüger Events",
        "hallo@rueger.events",
    ))
    .await?;
    posed(into, beauty).await?;
    Ok(())
}

// ── Patching a node the way adopting it would ────────────────────────────────

async fn patch_node(
    into: &Seeder,
    role: &NodeRole,
    description: &NodeDescription,
    node_pixels: &NodePixels,
    made: &mut HashSet<Uuid>,
) -> Result<Node> {
    // A strip patched in `sacn` mode owns no ports, so the node describes itself
    // without them: the description this show will see once it has put the strip in
    // that mode, and the types adoption would then make.
    let mut description = description.clone();
    let mut pixels = node_pixels.clone();
    for (output, patch) in role.strips.iter().enumerate() {
        let Some(patch) = patch else { continue };
        let head = output as u8 + 1;
        if let Some(strip) = pixels.strips.iter_mut().find(|s| s.head == head) {
            strip.mode = patch.mode;
            strip.count = patch.count;
            strip.universe = patch.universe;
        }
        if patch.mode == StripMode::Sacn {
            description.ports.retain(|p| p.head() != head);
        }
    }

    let mut strips = [Uuid::nil(); 2];
    let mut cells: [Vec<Element>; 2] = [Vec::new(), Vec::new()];
    let mut node = Uuid::nil();
    for head in description.head_numbers() {
        let fixture_type = openhaunt::head_type_from(V0_MODULE, "Node V0", &description, head, &pixels.effects);
        if made.insert(fixture_type.id) {
            into.create("fixture_types", &fixture_type).await?;
        }
        let strip = pixels.strip_for_head(head).map(PixelStrip::from_node).map(|mut s| {
            if let Some(patch) = role.strips[usize::from(head.saturating_sub(1))] {
                s.pitch_m = patch.pitch;
            }
            s
        });
        let patch = (head > 0).then(|| role.strips[usize::from(head - 1)]).flatten();
        let place = match patch {
            Some(p) => p.start,
            None => Vec3 { x: role.at.x + 0.2 * head as f32, y: role.at.y, z: role.at.z },
        };
        let fixture = Fixture {
            id: if head == 0 { id() } else { openhaunt::head_fixture_id(role.serial, head) },
            name: match head {
                0 => format!("{} node", role.name),
                n => format!("{} strip {n}", role.name),
            },
            fixture_type_id: fixture_type.id,
            address: FixtureAddress::OpenHaunt {
                serial: role.serial.to_string(),
                universe: None,
                head: (head != 0).then_some(head),
                strip: strip.clone(),
            },
            position: Some(at(place.x, place.y, place.z)),
            ..Fixture::default()
        };
        // Green is where the traffic light rests: an open maze with nobody in area 2
        // says "go" before anybody has walked through it.
        let fixture = if role.name == "Entrance" && head == 0 {
            let mut lit = fixture;
            lit.home_values.insert(parameter_key(&ParameterKind::Switch(1)), ParameterValue::Bool(true));
            lit
        } else {
            fixture
        };
        if head == 0 {
            node = fixture.id;
            into.create("fixtures", &fixture).await?;
            continue;
        }
        strips[usize::from(head - 1)] = fixture.id;
        match strip.filter(|s| s.mode == StripMode::Sacn) {
            // A strip the console draws is one fixture of the strip's own type, a cell
            // per pixel, placed where the strip starts.
            Some(strip) => {
                let strip_type = pixels::strip_type(&strip);
                if made.insert(strip_type.id) {
                    into.create("fixture_types", &strip_type).await?;
                }
                into.create("fixtures", &Fixture { fixture_type_id: strip_type.id, ..fixture.clone() }).await?;
                cells[usize::from(head - 1)] =
                    (1..=strip.count).map(|cell| Element { fixture: fixture.id, cell: Some(cell) }).collect();
            }
            None => into.create("fixtures", &fixture).await?,
        }
    }
    Ok(Node { serial: role.serial, node, strips, cells })
}

// ── Small builders ───────────────────────────────────────────────────────────

fn named_kind(name: &str) -> ParameterKind {
    ParameterKind::Named(name.to_string())
}

fn named(name: &str, rest: f32) -> ParameterDefinition {
    ParameterDefinition::new(named_kind(name), ParameterValue::Float(rest))
}

/// A light standing on the floor and looking straight up.
fn facing_up() -> Vec3 {
    Vec3 { x: 0.0, y: 1.0, z: 0.0 }
}

fn catalog_piece(catalog_id: &str) -> SceneObject {
    let piece = pult_schema::types::piece(catalog_id).expect("in the catalog");
    SceneObject {
        id: id(),
        name: piece.title.to_string(),
        kind: piece.kind,
        transform: Transform::default(),
        parent: None,
        layer: None,
        class: None,
        geometry: Vec::new(),
        symbol: None,
        catalog: Some(piece.id.to_string()),
        properties: serde_json::Value::Null,
        locked: false,
        weight_kg: None,
    }
}

fn variable(name: &str, kind: VariableKind, initial: f64) -> Variable {
    Variable {
        id: id(),
        name: name.into(),
        kind,
        initial,
        min: Some(0.0),
        max: None,
        value: None,
    }
}

fn tile(label: &str, color: Option<&str>, size: TileSize, target: TileTarget) -> SurfaceTile {
    SurfaceTile { id: id(), label: label.into(), color: color.map(str::to_string), size, target }
}

fn a_timeline(name: &str, end: Option<TimelineEnd>) -> Timeline {
    Timeline {
        id: id(),
        name: name.into(),
        clips: Vec::new(),
        detected: None,
        source: TimelineSource::Internal,
        grid: Vec::new(),
        markers: Vec::new(),
        events: Vec::new(),
        tracks: Vec::new(),
        flashes: None,
        pending: None,
        speed_master: None,
        node_id: None,
        end,
        running: false,
        anchor_ms: 0,
        position_at_anchor_ms: 0,
        rate: 1.0,
        recording: None,
        armed_by: None,
    }
}

fn a_clip(name: &str, sha: &str, start_ms: u64, destination: AudioDestination) -> AudioClip {
    AudioClip {
        id: id(),
        name: name.into(),
        asset: sha.into(),
        peaks: None,
        start_ms,
        offset_ms: 0,
        length_ms: None,
        gain_db: 0.0,
        fade_in_ms: 20,
        fade_out_ms: 200,
        looped: false,
        destination,
    }
}

/// A strip the node draws, told what to draw: the node's own effect by name, its
/// color, brightness and speed. Speed is a fader over the port's own 0–10 Hz.
fn node_effect(strip: Uuid, effect: &str, (r, g, b): (f32, f32, f32), brightness: f32, hz: f32) -> Vec<ParameterCapture> {
    vec![
        capture(strip, named_kind("Effect"), ParameterValue::Text(effect.into())),
        capture(strip, ParameterKind::ColorRgb, ParameterValue::rgb(r, g, b)),
        capture(strip, ParameterKind::Intensity, ParameterValue::Float(brightness)),
        capture(strip, named_kind("Speed"), ParameterValue::Float(hz / 10.0)),
    ]
}

/// A pool template, applied to some fixtures the way the Effects panel applies one:
/// one effect id per layer across the selection, each fixture's phase from the
/// template's spread, and the template named so the panel can say where it came from.
/// A list of fixtures as the elements of a selection: each one whole.
fn whole(fixtures: &[Uuid]) -> Vec<Element> {
    fixtures.iter().map(|f| Element { fixture: *f, cell: None }).collect()
}

fn from_template(
    template: &EffectTemplate,
    elements: &[Element],
    kinds_of: &dyn Fn(Uuid) -> Vec<ParameterKind>,
) -> Vec<ParameterCapture> {
    let mut captures = Vec::new();
    for layer in &template.layers {
        let effect_id = id();
        let chosen: Vec<&Element> =
            elements.iter().filter(|e| kinds_of(e.fixture).contains(&layer.kind)).collect();
        let offsets = phases(layer.spread, chosen.len(), layer.spread_cycles);
        for (element, phase) in chosen.iter().zip(offsets) {
            let (fixture, kind) = match element.cell {
                Some(cell) => (&element.fixture, layer.kind.in_cell(cell)),
                None => (&element.fixture, layer.kind.clone()),
            };
            captures.push(ParameterCapture {
                effect: Some(EffectSpec {
                    effect_id,
                    curve: layer.curve.clone(),
                    rate: layer.rate,
                    low: layer.low.clone(),
                    high: layer.high.clone(),
                    width: layer.width,
                    direction: layer.direction,
                    phase,
                    spread: layer.spread,
                    t0: None,
                    template: Some(template.id),
                }),
                ..capture(*fixture, kind, layer.low.clone())
            });
        }
    }
    captures
}

/// The phase each fixture of a selection gets — `frontend/src/lib/effects.ts`'s
/// `phases`, for the spreads the templates use.
fn phases(spread: Spread, n: usize, cycles: f32) -> Vec<f32> {
    let base: Vec<f32> = match (n, spread) {
        (0, _) => Vec::new(),
        (1, _) => vec![0.0],
        (_, Spread::Linear) => (0..n).map(|i| i as f32 / n as f32).collect(),
        (_, Spread::Reversed) => (0..n).map(|i| (n - 1 - i) as f32 / n as f32).collect(),
        (_, Spread::Random { seed }) => {
            let mut next = mulberry32(seed);
            (0..n).map(|_| next()).collect()
        }
        _ => vec![0.0; n],
    };
    if cycles == 1.0 {
        base
    } else {
        base.into_iter().map(|p| (p * cycles).rem_euclid(1.0)).collect()
    }
}

fn mulberry32(seed: u32) -> impl FnMut() -> f32 {
    let mut a = seed;
    move || {
        a = a.wrapping_add(0x6d2b_79f5);
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        ((t ^ (t >> 14)) as f64 / 4_294_967_296.0) as f32
    }
}

// ── The sound, made here ─────────────────────────────────────────────────────

fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    pult_audio::wav::encode(samples, 1, rate, pult_audio::wav::Encoding::Pcm16).expect("mono PCM16")
}

/// A noise source that is the same every time, so the files — and their shas — are.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Wind through the hedges and a drone under it. Sixteen seconds, which is what the
/// ambience loops at; the wind's slow swell is a whole number of cycles of that, so
/// the seam is where the swell is at its quietest.
fn ambience(rate: u32, seconds: f32) -> Vec<f32> {
    let mut noise = Noise(0x4848_4701);
    let n = (rate as f32 * seconds) as usize;
    let mut low = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            // A one-pole low-pass over white noise is wind, more or less.
            low += 0.02 * (noise.next() - low);
            let swell = 0.5 - 0.5 * (std::f32::consts::TAU * t / seconds * 2.0).cos();
            let drone = 0.08 * (std::f32::consts::TAU * 55.0 * t).sin();
            (1.6 * low * (0.3 + 0.7 * swell) + drone).clamp(-1.0, 1.0)
        })
        .collect()
}

fn a_scream(rate: u32, seconds: f32) -> Vec<f32> {
    let mut noise = Noise(0x4848_4702);
    let n = (rate as f32 * seconds) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let pitch = 900.0 - 350.0 * (t / seconds) + 40.0 * (std::f32::consts::TAU * 7.0 * t).sin();
            phase += pitch / rate as f32;
            let envelope = (t / 0.03).min(1.0) * (1.0 - t / seconds).max(0.0).powf(0.6);
            let voice = (std::f32::consts::TAU * phase).sin() + 0.4 * (std::f32::consts::TAU * phase * 2.0).sin();
            (envelope * (0.55 * voice + 0.25 * noise.next())).clamp(-1.0, 1.0)
        })
        .collect()
}

fn a_bang(rate: u32, seconds: f32) -> Vec<f32> {
    let mut noise = Noise(0x4848_4703);
    let n = (rate as f32 * seconds) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let body = (std::f32::consts::TAU * (48.0 + 30.0 * (-t * 8.0).exp()) * t).sin() * (-t * 5.0).exp();
            let crack = noise.next() * (-t * 60.0).exp();
            (0.9 * body + 0.6 * crack).clamp(-1.0, 1.0)
        })
        .collect()
}

/// "Ha" five times a second from half a second in, which is what the skeleton's jaw
/// is recorded against.
fn a_laugh(rate: u32, seconds: f32) -> Vec<f32> {
    let n = (rate as f32 * seconds) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let in_laugh = t - LAUGH_STARTS;
            let syllable = (in_laugh * LAUGH_HZ).rem_euclid(1.0);
            let gate = if (0.0..LAUGH_LENGTH).contains(&in_laugh) && syllable < 0.55 {
                (syllable / 0.05).min(1.0) * (1.0 - syllable / 0.55)
            } else {
                0.0
            };
            let pitch = 180.0 - 30.0 * (in_laugh / LAUGH_LENGTH).clamp(0.0, 1.0);
            phase += pitch / rate as f32;
            // A sawtooth is a voice's buzz before the mouth shapes it.
            let saw = 2.0 * phase.rem_euclid(1.0) - 1.0;
            (0.7 * gate * saw).clamp(-1.0, 1.0)
        })
        .collect()
}

const LAUGH_STARTS: f32 = 0.5;
const LAUGH_HZ: f32 = 5.0;
const LAUGH_LENGTH: f32 = 3.0;

/// A take of one skeleton speaking the laugh: the jaw opens on each "ha", the head
/// turns slowly through it, and the arms come up and go down again. Change points
/// only, the way a recording is written; `offset_ms` is where on the timeline the
/// laugh clip starts.
pub fn skeleton_take(skeleton: Uuid, offset_ms: u32) -> pult_render::track::Track {
    use pult_render::track::{Track, TrackKey, TrackPoint};
    let point = |ms: u32, v: f32| TrackPoint { release: false, ms: offset_ms + ms, value: ParameterValue::Float(v) };
    let syllables = (LAUGH_LENGTH * LAUGH_HZ) as u32;
    let start = (LAUGH_STARTS * 1000.0) as u32;
    let step = (1000.0 / LAUGH_HZ) as u32;
    let mut jaw = vec![point(0, 0.0)];
    for n in 0..syllables {
        jaw.push(point(start + n * step, 0.85));
        jaw.push(point(start + n * step + step / 2, 0.15));
    }
    jaw.push(point(start + syllables * step, 0.0));
    let head: Vec<TrackPoint> = (0..=24)
        .map(|n| point(n * 250, 0.5 + 0.2 * (n as f32 * 0.4).sin()))
        .chain(std::iter::once(point(6_500, 0.5)))
        .collect();
    let key = |kind: &ParameterKind| parameter_key(kind);
    Track {
        keys: vec![
            TrackKey { fixture_id: skeleton, key: key(&named_kind("Jaw")), points: jaw },
            TrackKey { fixture_id: skeleton, key: key(&ParameterKind::Pan), points: head },
            TrackKey {
                fixture_id: skeleton,
                key: key(&named_kind("Left arm")),
                points: vec![point(0, 0.0), point(800, 0.9), point(5_000, 0.0)],
            },
            TrackKey {
                fixture_id: skeleton,
                key: key(&named_kind("Right arm")),
                points: vec![point(0, 0.0), point(1_200, 0.9), point(5_500, 0.0)],
            },
        ],
    }
}
