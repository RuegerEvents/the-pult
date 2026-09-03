//! Two hundred heads on trusses, in layers, six sequences deep.
//!
//! The show to open when the question is what the console does under a rig somebody
//! would actually hire. Three things it has that the smaller demos do not:
//!
//! - **The trusses are scene objects, and the heads hang off them.** A fixture's
//!   position is relative to whatever it hangs off, so dragging a truss moves its
//!   lights — which is the whole reason `Fixture::parent` exists, and is invisible
//!   in a rig where everything is placed in world space.
//! - **Layers**, so hiding half the rig is one click and the Layers panel has
//!   something to be about.
//! - **Six sequences, all running**, each holding a slice of the rig with an effect
//!   on it. Which is what makes the station tick: the engine has no clock of its own,
//!   so a show with nothing running is a station doing nothing at all.
//!
//! Deterministic, and needs no asset: the trusses draw as boxes, so opening this
//! costs no download and no network.

use anyhow::Result;
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        cue::ParameterCapture,
        effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread},
        fixture::{ParameterKind, ParameterValue, Vec3},
        scene::{Layer, SceneObject, SceneObjectKind, Transform},
        sequence::Sequence,
        speedmaster::SpeedMaster,
        Fixture,
    },
};

use super::{
    haunt::{a_cue, a_type, capture, colour, dmx, intensity, pan, tilt},
    id, now_ms, Seeder,
};

/// How many heads per truss, and how many trusses. Two hundred is the size task 29
/// measured a console working hard at, without being the two thousand that exists
/// only to be measured.
const TRUSSES: usize = 10;
const PER_TRUSS: usize = 20;

/// Straight down off a truss.
const HANGING: Vec3 = Vec3 { x: 90.0, y: 0.0, z: 0.0 };

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Festival").await?;

    let head = a_type("Wash 19×15W", vec![intensity(), colour(), pan(), tilt()]);
    into.create("fixture_types", &head).await?;

    // Two layers, so hiding half the rig is one click. Front and back rather than
    // odd and even: a layer is a thing an operator points at, not a partition.
    let mut layers = Vec::new();
    for (n, name) in ["Downstage", "Upstage"].into_iter().enumerate() {
        let layer = Layer {
            id: id(),
            name: name.to_string(),
            locked: false,
            sort_order: n as u32,
        };
        into.create("layers", &layer).await?;
        layers.push(layer.id);
    }

    // Ten trusses, five downstage and five up, each a box of its own so the rig view
    // has structure in it rather than two hundred floating heads.
    let mut trusses = Vec::new();
    for n in 0..TRUSSES {
        let downstage = n < TRUSSES / 2;
        let along = n % (TRUSSES / 2);
        let truss = SceneObject {
            id: id(),
            name: format!("Truss {}", n + 1),
            kind: SceneObjectKind::Truss,
            transform: Transform {
                position: Vec3 {
                    x: -16.0 + 8.0 * along as f32,
                    y: if downstage { 8.0 } else { 9.5 },
                    z: if downstage { 4.0 } else { -4.0 },
                },
                rotation: Vec3::default(),
                scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            },
            parent: None,
            layer: Some(layers[usize::from(!downstage)]),
            class: None,
            geometry: Vec::new(),
            symbol: None,
        };
        into.create("scene_objects", &truss).await?;
        trusses.push((truss.id, truss.layer));
    }

    // Twenty heads per truss, hung off it: a head's position is relative to whatever
    // it hangs off, so moving a truss moves its lights.
    //
    // Addressed straight through, rolling into the next universe when this one is
    // full — six channels each means about eighty-five to a universe, so this is
    // three of them and the Patch panel's universe view has something to show.
    let mut universe = 1u16;
    let mut next = 1u16;
    for (t, (truss, layer)) in trusses.iter().enumerate() {
        for n in 0..PER_TRUSS {
            if next + head.channel_count > 513 {
                universe += 1;
                next = 1;
            }
            let fixture = Fixture {
                id: id(),
                name: format!("Head {}", t * PER_TRUSS + n + 1),
                fixture_type_id: head.id,
                address: dmx(universe, next),
                // Relative to the truss: a metre apart along it, hanging.
                position: Some(Transform {
                    position: Vec3 { x: -3.0 + 0.32 * n as f32, y: -0.3, z: 0.0 },
                    rotation: HANGING,
                    scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                }),
                parent: Some(*truss),
                layer: *layer,
                ..Fixture::default()
            };
            into.create("fixtures", &fixture).await?;
            next += head.channel_count;
        }
    }

    let master = SpeedMaster {
        id: id(),
        name: "Show".to_string(),
        bpm: 124.0,
        multiplier: 1.0,
        running: true,
        t0: now_ms(),
    };
    into.create("speed_masters", &master).await?;

    let rig = into.fixtures().await;

    // Six sequences, each holding a slice of the rig with a shape on it. Slices
    // rather than the whole rig per sequence, because that is what a real stack does
    // and because six effects over two hundred heads each would be a rig where
    // everything is asserted six times.
    const SEQUENCES: usize = 6;
    for s in 0..SEQUENCES {
        let slice: Vec<&Fixture> =
            rig.iter().skip(s).step_by(SEQUENCES).collect();
        let effect = id();
        let mut cue = a_cue(&format!("Look {}", s + 1), 1.0, Vec::new());
        cue.fade_in_ms = 2_000;
        for (n, fixture) in slice.iter().enumerate() {
            cue.captures.push(capture(
                fixture.id,
                ParameterKind::Intensity,
                ParameterValue::Float(0.7),
            ));
            cue.captures.push(ParameterCapture {
                effect: Some(EffectSpec {
                    effect_id: effect,
                    curve: Curve::Shape(if s % 2 == 0 { Shape::Sine } else { Shape::Triangle }),
                    rate: Rate::Master {
                        id: master.id,
                        // Each sequence a different fraction of the one tempo, so
                        // they drift against each other without ever disagreeing
                        // about the beat.
                        multiplier: 0.25 * (s as f32 + 1.0),
                    },
                    low: ParameterValue::rgb(0.9, 0.2, 0.0),
                    high: ParameterValue::rgb(0.0, 0.4, 1.0),
                    width: 0.5,
                    direction: if s % 3 == 0 { Direction::Forward } else { Direction::Backward },
                    phase: n as f32 / slice.len().max(1) as f32,
                    spread: Spread::Linear,
                    t0: None,
                }),
                ..capture(fixture.id, ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0))
            });
        }
        into.create("cues", &cue).await?;

        let sequence = Sequence {
            id: id(),
            name: format!("Look {}", s + 1),
            cue_ids: vec![cue.id],
            active_cue_index: None,
            went_at: None,
        };
        into.create("sequences", &sequence).await?;
        // Through the sequence's own Go: taking a cue is what anchors `went_at`, and
        // an effect with no anchor renders nothing.
        into.set(
            vec![
                PathSegment::Key("sequences".into()),
                PathSegment::Id(sequence.id),
                PathSegment::Key("goNext".into()),
            ],
            serde_json::json!({}),
            Lifecycle::Synced,
        )
        .await?;
    }

    Ok(())
}
