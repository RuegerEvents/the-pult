//! Two hundred heads on trusses, in layers, six sequences deep.
//!
//! The show to open when the question is what the console does under a rig somebody
//! would actually hire. Three things it has that the smaller demos do not:
//!
//! - **The trusses are runs of real sections, and the heads hang off them.** A
//!   fixture's position is relative to whatever it hangs off, so dragging a truss
//!   moves its lights — which is the whole reason `Fixture::parent` exists, and is
//!   invisible in a rig where everything is placed in world space.
//! - **Layers**, so hiding half the rig is one click and the Layers panel has
//!   something to be about.
//! - **Six sequences, all running**, each holding a slice of the rig with an effect
//!   on it. Which is what makes the station tick: the engine has no clock of its own,
//!   so a show with nothing running is a station doing nothing at all.
//!
//! Deterministic, and it needs no asset: the trusses come out of the console's own
//! catalogue, so opening this costs no download and no network.

use anyhow::Result;
use pult_schema::types::{
    cue::ParameterCapture,
    effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread},
    fixture::{Fixture, ParameterKind, ParameterValue, Vec3},
    scene::{Layer, Transform},
    speedmaster::SpeedMaster,
};

use super::{
    id,
    kit::{
        a_cue, a_fixture, a_piece, a_stack, a_type, aimed, capture, colour, facing, intensity,
        level, pan, tilt, truss_run, Addresses,
    },
    now_ms, Seeder,
};

/// Eight truss runs of twenty-five heads: two hundred, which is the size task 29
/// measured a console working hard at without being the two thousand that exists
/// only to be measured.
const RUNS: usize = 8;
const PER_RUN: usize = 25;
/// Fifteen metres across, which is five three-metre lengths.
const SPAN: f32 = 15.0;

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Festival").await?;

    let head = a_type("Wash 19×15W", vec![intensity(), colour(), pan(), tilt()]);
    into.create("fixture_types", &head).await?;

    // Two layers, so hiding half the rig is one click. Front and back rather than
    // odd and even: a layer is a thing an operator points at, not a partition.
    let mut layers = Vec::new();
    for (n, name) in ["Downstage", "Upstage"].into_iter().enumerate() {
        let layer =
            Layer { id: id(), name: name.to_string(), locked: false, sort_order: n as u32 };
        into.create("layers", &layer).await?;
        layers.push(layer.id);
    }

    // Eight trusses spanning the stage at increasing depth and trim, so the rig
    // reads as a stack of them going back rather than as one flat wall.
    let mut runs = Vec::new();
    for n in 0..RUNS {
        let downstage = n < RUNS / 2;
        let layer = layers[usize::from(!downstage)];
        let run = truss_run(
            into,
            &format!("Truss {}", n + 1),
            Some(layer),
            Vec3 { x: 0.0, y: 8.0 + n as f32 * 0.4, z: 7.0 - n as f32 * 2.0 },
            SPAN,
        )
        .await?;
        runs.push((run, layer));
    }

    // A stage to light. Six decks across the front, and a back wall of flats.
    for n in 0..6 {
        a_piece(
            into,
            &format!("Deck {}", n + 1),
            "deck-2x1",
            Transform {
                position: Vec3 { x: -5.0 + 2.0 * n as f32, y: 1.0, z: 2.0 },
                ..Transform::default()
            },
            None,
        )
        .await?;
    }
    for n in 0..8 {
        a_piece(
            into,
            &format!("Backdrop {}", n + 1),
            "flat-1x24",
            Transform {
                position: Vec3 { x: -7.0 + 2.0 * n as f32, y: 0.0, z: -8.0 },
                scale: Vec3 { x: 2.0, y: 2.0, z: 1.0 },
                ..Transform::default()
            },
            None,
        )
        .await?;
    }

    // Twenty-five heads per truss, hung off it: a head's position is relative to
    // whatever it hangs off, so moving a truss moves its lights.
    //
    // Addressed straight through, rolling into the next universe when this one is
    // full — six channels each means about eighty-five to a universe, so this is
    // three of them and the Patch panel's universe view has something to show.
    let mut addresses = Addresses::from(1);
    let step = SPAN / PER_RUN as f32;
    for (t, (truss, layer)) in runs.iter().enumerate() {
        for n in 0..PER_RUN {
            let mut fixture = a_fixture(
                &format!("Head {}", t * PER_RUN + n + 1),
                head.id,
                addresses.take(head.channel_count),
                aimed(-SPAN / 2.0 + step * (n as f32 + 0.5), -0.3, 0.0, facing::DOWN),
            );
            fixture.parent = Some(*truss);
            fixture.layer = Some(*layer);
            into.create("fixtures", &fixture).await?;
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

    // Six sequences, each holding a slice of the rig, and each with a stack of four
    // rather than a single look — a festival operator is riding several at once and
    // pressing Go on all of them, which is the thing worth being able to try.
    //
    // Slices rather than the whole rig per sequence, because six effects over two
    // hundred heads each would be a rig where everything is asserted six times.
    const SEQUENCES: usize = 6;
    for s in 0..SEQUENCES {
        let slice: Vec<&Fixture> = rig.iter().skip(s).step_by(SEQUENCES).collect();
        let shaped = |kind: ParameterKind,
                      shape,
                      multiplier: f32,
                      low: ParameterValue,
                      high: ParameterValue,
                      direction| {
            let effect = id();
            let mut captures: Vec<ParameterCapture> = Vec::new();
            for (n, fixture) in slice.iter().enumerate() {
                captures.push(level(fixture.id, 0.7));
                captures.push(ParameterCapture {
                    effect: Some(EffectSpec {
                        effect_id: effect,
                        curve: Curve::Shape(shape),
                        // Each sequence a different fraction of the one tempo, so
                        // they drift against each other without ever disagreeing
                        // about the beat.
                        rate: Rate::Master { id: master.id, multiplier },
                        low: low.clone(),
                        high: high.clone(),
                        width: 0.5,
                        direction,
                        phase: n as f32 / slice.len().max(1) as f32,
                        spread: Spread::Linear,
                        t0: None,
                    }),
                    ..capture(fixture.id, kind.clone(), low.clone())
                });
            }
            captures
        };

        let rate = 0.25 * (s as f32 + 1.0);
        let mut cues = vec![
            a_cue(
                &format!("Colour {}", s + 1),
                1.0,
                shaped(
                    ParameterKind::ColorRgb,
                    if s % 2 == 0 { Shape::Sine } else { Shape::Triangle },
                    rate,
                    ParameterValue::rgb(0.9, 0.2, 0.0),
                    ParameterValue::rgb(0.0, 0.4, 1.0),
                    if s % 3 == 0 { Direction::Forward } else { Direction::Backward },
                ),
            ),
            a_cue(
                &format!("Tilt {}", s + 1),
                2.0,
                shaped(
                    ParameterKind::Tilt,
                    Shape::Sine,
                    rate,
                    ParameterValue::Float(0.3),
                    ParameterValue::Float(0.7),
                    Direction::Forward,
                ),
            ),
            a_cue(
                &format!("Chase {}", s + 1),
                3.0,
                shaped(
                    ParameterKind::Intensity,
                    Shape::SawDown,
                    rate * 2.0,
                    ParameterValue::Float(0.0),
                    ParameterValue::Float(1.0),
                    Direction::Forward,
                ),
            ),
            a_cue(
                &format!("Hold {}", s + 1),
                4.0,
                slice.iter().map(|fixture| level(fixture.id, 0.5)).collect(),
            ),
        ];
        for cue in &mut cues {
            cue.fade_in_ms = 2_000;
        }

        // Started, so the rig is moving the moment the show opens.
        a_stack(into, &format!("Look {}", s + 1), cues, true).await?;
    }

    Ok(())
}
