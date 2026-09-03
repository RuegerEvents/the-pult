//! The pieces every demo is built out of.
//!
//! Four shows that all patch fixtures, hang trusses, write cues and start
//! sequences. This is the vocabulary; the four files beside it are the shows.
//!
//! # Nothing here writes a rotation down
//!
//! A fixture's own axis is **−Y**, straight down, so zero rotation is a light
//! hanging and looking at the floor. The rotation that aims one somewhere else is
//! *not* two of the three XYZ angles — XYZ tips before it turns, so a light aimed
//! sideways loses its turn entirely — and `Transform::facing` is the one place in
//! this console that does that arithmetic. Every demo says which way a light points
//! as a direction and lets it work out the angles.
//!
//! That is not a tidiness rule. The first version of these demos wrote `{90, 0, 0}`
//! meaning "hanging", which is a quarter turn away from hanging, and every head in
//! three of the four shows pointed at the back wall.

use anyhow::Result;
use pult_schema::types::{
    catalogue,
    cue::{Cue, FollowMode, ParameterCapture},
    effect::Easing,
    fixture::{
        Fixture, FixtureAddress, FixtureType, ParameterDefinition, ParameterKind, ParameterValue,
        Vec3,
    },
    flow::{Flow, FlowEdge, FlowNode, FlowNodeKind},
    scene::{SceneObject, SceneObjectKind, Transform},
    sequence::Sequence,
    dmx_mode::DmxBreak,
};
use uuid::Uuid;

use super::{id, Seeder};

// ── Where things point ────────────────────────────────────────────────────────

/// The directions a demo aims a light in, named for what they mean.
///
/// Nobody reads `{143.13, 0, 180}` and sees a light pointing at the audience's feet.
pub mod facing {
    use super::Vec3;

    /// Straight down: what a head on a truss does before anybody aims it.
    pub const DOWN: Vec3 = Vec3 { x: 0.0, y: -1.0, z: 0.0 };
    /// Downstage and down — the front-of-house angle, onto the acting area.
    pub const DOWNSTAGE: Vec3 = Vec3 { x: 0.0, y: -0.8, z: 0.6 };
    /// Upstage and down, for backlight looking the other way.
    pub const UPSTAGE: Vec3 = Vec3 { x: 0.0, y: -0.8, z: -0.6 };
    /// Across the stage from the audience's left, and down: a boom.
    pub const FROM_LEFT: Vec3 = Vec3 { x: 0.9, y: -0.44, z: 0.0 };
    /// And from the other side.
    pub const FROM_RIGHT: Vec3 = Vec3 { x: -0.9, y: -0.44, z: 0.0 };
    /// Steeply down and a little upstage: a cyc batten washing a cloth.
    pub const AT_THE_CLOTH: Vec3 = Vec3 { x: 0.0, y: -0.9, z: -0.44 };
}

/// At a point, turned to nothing.
pub fn at(x: f32, y: f32, z: f32) -> Transform {
    Transform::at(Vec3 { x, y, z })
}

/// At a point, aimed along a direction. Never a hand-written rotation: see the
/// module note.
pub fn aimed(x: f32, y: f32, z: f32, direction: Vec3) -> Transform {
    Transform::facing(Vec3 { x, y, z }, direction)
}

// ── Patching ──────────────────────────────────────────────────────────────────

/// One DMX address in a universe.
pub fn dmx(universe: u16, address: u16) -> FixtureAddress {
    FixtureAddress::Dmx {
        mode: "Default".to_string(),
        breaks: vec![DmxBreak { universe, address }],
    }
}

/// A fixture type with an implicit mode: a byte per output parameter in the order
/// they are listed, and three for a colour.
pub fn a_type(name: &str, parameters: Vec<ParameterDefinition>) -> FixtureType {
    FixtureType {
        id: id(),
        name: name.to_string(),
        manufacturer: "Generic".to_string(),
        channel_count: parameters
            .iter()
            .map(|p| if matches!(p.kind, ParameterKind::ColorRgb) { 3 } else { 1 })
            .sum::<u16>(),
        parameters,
        ..FixtureType::default()
    }
}

pub fn intensity() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::Intensity, ParameterValue::Float(0.0))
}

pub fn colour() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::ColorRgb, ParameterValue::rgb(1.0, 1.0, 1.0))
}

/// Pan and tilt rest in the middle, which is a head pointing along the rest
/// direction its transform gives it.
pub fn pan() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::Pan, ParameterValue::Float(0.5))
}

pub fn tilt() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::Tilt, ParameterValue::Float(0.5))
}

/// A strobe channel carries a *rate*: the console sends the byte and the fixture
/// does the flashing.
pub fn strobe_rate() -> ParameterDefinition {
    ParameterDefinition::new(ParameterKind::Strobe, ParameterValue::Float(0.0))
}

pub fn a_fixture(
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

/// A patch head that walks a universe and rolls into the next when one is full.
///
/// Which is what a real patch does, and what makes the Patch panel's universe view
/// worth looking at in the larger demos.
pub struct Addresses {
    universe: u16,
    next: u16,
}

impl Addresses {
    pub fn from(universe: u16) -> Self {
        Addresses { universe, next: 1 }
    }

    pub fn take(&mut self, channels: u16) -> FixtureAddress {
        if self.next + channels > 513 {
            self.universe += 1;
            self.next = 1;
        }
        let address = dmx(self.universe, self.next);
        self.next += channels;
        address
    }
}

// ── The room ──────────────────────────────────────────────────────────────────

/// Hang a run of truss, and answer the handle that moves it and its lights together.
///
/// The run is a `Group` — which is what `SceneObjectKind::Group` is for — with the
/// sections parented to it and, by the caller, the fixtures too. So dragging the run
/// moves everything on it, which is the whole reason a fixture's position is
/// relative to what it hangs off.
///
/// `along` is how many metres of it, made of three-metre lengths; the run is centred
/// on `centre` and runs along X.
pub async fn truss_run(
    into: &Seeder,
    name: &str,
    layer: Option<Uuid>,
    centre: Vec3,
    metres: f32,
) -> Result<Uuid> {
    let section = catalogue::piece("f34-3m").expect("the catalogue has a three-metre length");
    let sections = (metres / section.size.x).round().max(1.0) as usize;
    let span = sections as f32 * section.size.x;

    let run = SceneObject {
        id: id(),
        name: name.to_string(),
        kind: SceneObjectKind::Group,
        transform: Transform::at(centre),
        parent: None,
        layer,
        class: None,
        geometry: Vec::new(),
        symbol: None,
        // The handle itself draws nothing: it is a place to take hold of, and the
        // sections under it are what there is to see.
        catalogue: None,
    };
    into.create("scene_objects", &run).await?;

    for n in 0..sections {
        let offset = -span / 2.0 + section.size.x * (n as f32 + 0.5);
        into.create(
            "scene_objects",
            &SceneObject {
                id: id(),
                name: format!("{name} {}", n + 1),
                kind: section.kind,
                transform: Transform::at(Vec3 { x: offset, y: 0.0, z: 0.0 }),
                parent: Some(run.id),
                layer,
                class: None,
                geometry: Vec::new(),
                symbol: None,
                catalogue: Some(section.id.to_string()),
            },
        )
        .await?;
    }
    Ok(run.id)
}

/// One piece out of the catalogue, placed. Decks, walls and flats.
pub async fn a_piece(
    into: &Seeder,
    name: &str,
    catalogue_id: &str,
    transform: Transform,
    layer: Option<Uuid>,
) -> Result<Uuid> {
    let piece = catalogue::piece(catalogue_id)
        .unwrap_or_else(|| panic!("{catalogue_id} is not in the catalogue"));
    let object = SceneObject {
        id: id(),
        name: name.to_string(),
        kind: piece.kind,
        transform,
        parent: None,
        layer,
        class: None,
        geometry: Vec::new(),
        symbol: None,
        catalogue: Some(piece.id.to_string()),
    };
    into.create("scene_objects", &object).await?;
    Ok(object.id)
}

// ── Cues and playback ─────────────────────────────────────────────────────────

pub fn capture(fixture: Uuid, kind: ParameterKind, value: ParameterValue) -> ParameterCapture {
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

pub fn level(fixture: Uuid, at: f32) -> ParameterCapture {
    capture(fixture, ParameterKind::Intensity, ParameterValue::Float(at))
}

pub fn hue(fixture: Uuid, r: f32, g: f32, b: f32) -> ParameterCapture {
    capture(fixture, ParameterKind::ColorRgb, ParameterValue::rgb(r, g, b))
}

pub fn a_cue(name: &str, number: f64, captures: Vec<ParameterCapture>) -> Cue {
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

/// Write a stack and the sequence that runs it.
///
/// `start` takes the first cue, which is what anchors `went_at` — an effect with no
/// anchor renders nothing, so a demo that wants something moving the moment it opens
/// has to go through the sequence's own Go rather than write `active_cue_index`.
pub async fn a_stack(into: &Seeder, name: &str, cues: Vec<Cue>, start: bool) -> Result<Uuid> {
    for cue in &cues {
        into.create("cues", cue).await?;
    }
    let sequence = Sequence {
        id: id(),
        name: name.to_string(),
        cue_ids: cues.iter().map(|cue| cue.id).collect(),
        active_cue_index: None,
        went_at: None,
    };
    into.create("sequences", &sequence).await?;
    if start {
        into.set(
            vec![
                pult_schema::path::PathSegment::Key("sequences".into()),
                pult_schema::path::PathSegment::Id(sequence.id),
                pult_schema::path::PathSegment::Key("goNext".into()),
            ],
            serde_json::json!({}),
            pult_schema::lifecycle::Lifecycle::Synced,
        )
        .await?;
    }
    Ok(sequence.id)
}

/// A colour that walks through the evening, for a cyc or a wash.
pub fn sky(step: usize) -> ParameterValue {
    const SKY: &[(f32, f32, f32)] = &[
        (0.10, 0.20, 0.60),
        (0.20, 0.30, 0.70),
        (0.40, 0.50, 0.90),
        (0.60, 0.40, 0.30),
        (0.20, 0.10, 0.30),
        (0.05, 0.05, 0.20),
        (0.90, 0.60, 0.30),
        (0.50, 0.70, 1.00),
        (1.00, 0.90, 0.80),
    ];
    let (r, g, b) = SKY[step % SKY.len()];
    ParameterValue::rgb(r, g, b)
}

// ── Flows ─────────────────────────────────────────────────────────────────────

/// One flow graph: its nodes at the coordinates given, wired as
/// `(from, port, to, port)`.
///
/// Nodes carry their own coordinates rather than being laid out from an index,
/// because an `And` has two feeders and they cannot share a row.
pub async fn draw(
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every direction the demos aim a light in points *down*.
    ///
    /// The regression this is here for: `{90, 0, 0}` written meaning "hanging" is a
    /// quarter turn away from hanging, and it aimed every head in three of the four
    /// shows at the back wall. A demo is the first thing anybody sees.
    #[test]
    fn nothing_a_demo_aims_is_pointed_at_the_ceiling() {
        for (name, direction) in [
            ("DOWN", facing::DOWN),
            ("DOWNSTAGE", facing::DOWNSTAGE),
            ("UPSTAGE", facing::UPSTAGE),
            ("FROM_LEFT", facing::FROM_LEFT),
            ("FROM_RIGHT", facing::FROM_RIGHT),
            ("AT_THE_CLOTH", facing::AT_THE_CLOTH),
        ] {
            assert!(direction.y < 0.0, "{name} points upwards");
            let aimed = aimed(0.0, 5.0, 0.0, direction);
            let back = aimed.facing_direction();
            assert!(back.y < 0.0, "{name} comes back out of the transform pointing up: {back:?}");
        }
    }

    /// And a direction survives the round trip through Euler angles.
    #[test]
    fn a_light_points_where_it_was_aimed() {
        for direction in [facing::DOWNSTAGE, facing::UPSTAGE, facing::FROM_LEFT] {
            let length = (direction.x.powi(2) + direction.y.powi(2) + direction.z.powi(2)).sqrt();
            let unit = Vec3 {
                x: direction.x / length,
                y: direction.y / length,
                z: direction.z / length,
            };
            let back = aimed(1.0, 2.0, 3.0, direction).facing_direction();
            for (was, is) in [(unit.x, back.x), (unit.y, back.y), (unit.z, back.z)] {
                assert!((was - is).abs() < 1e-3, "aimed at {unit:?}, points {back:?}");
            }
        }
    }

    /// Straight down is no rotation at all, which is the fact the whole module rests
    /// on and the one the first version of these demos got wrong.
    #[test]
    fn hanging_is_zero_rotation() {
        let hanging = aimed(0.0, 6.0, 0.0, facing::DOWN);
        assert_eq!(hanging.rotation, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
    }

    #[test]
    fn a_patch_head_rolls_into_the_next_universe_when_one_is_full() {
        let mut addresses = Addresses::from(1);
        // 85 six-channel heads fit in a universe; the 86th does not.
        for _ in 0..85 {
            addresses.take(6);
        }
        let FixtureAddress::Dmx { breaks, .. } = addresses.take(6) else { panic!("a DMX address") };
        assert_eq!((breaks[0].universe, breaks[0].address), (2, 1));
    }
}
