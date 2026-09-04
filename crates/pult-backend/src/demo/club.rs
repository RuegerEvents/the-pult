//! Movers, washes and strobes, with effects left running.
//!
//! The show to open when the question is whether the console *works*: three
//! sequences are live at once, each holding a slice of the rig against one of two
//! speed masters, so the rig view has something moving in it and the tempo controls
//! do something visible the moment they are touched.
//!
//! It is also the one that shows what an effect is. Nothing here stores a value that
//! moves — the cues store *shapes*, anchored on the cue's own `went_at`, and every
//! consumer works out a number for the moment it needs one. Which is why a station
//! that joins half way through a night lands on the same beat as the one that has
//! been running since the doors opened.

use anyhow::Result;
use pult_schema::types::{
    cue::{Cue, ParameterCapture},
    effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread},
    fixture::{ParameterKind, ParameterValue, Vec3},
    mount::Mount,
    scene::Transform,
    speedmaster::SpeedMaster,
};
use uuid::Uuid;

use super::{
    id,
    kit::{
        a_clamped_fixture, a_cue, a_fixture, a_piece, a_stack, a_type, aimed, capture, colour, facing, hue, intensity,
        level, pan, strobe_rate, tilt, truss_run, under, Addresses,
    },
    now_ms, Seeder,
};

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Club").await?;

    let mover = a_type("Beam 7R", vec![intensity(), colour(), pan(), tilt()]);
    let wash = a_type("LED Wash", vec![intensity(), colour()]);
    let strobe = a_type("Strobe", vec![intensity(), strobe_rate()]);
    for kind in [&mover, &wash, &strobe] {
        into.create("fixture_types", kind).await?;
    }

    // Two trusses, downstage and upstage, at different heights so the rig view is
    // not a single flat row — and a riser across the back for the strobes, which in
    // a club sit at floor level and look up.
    let downstage =
        truss_run(into, "Front truss", None, Vec3 { x: 0.0, y: 6.0, z: 3.0 }, 12.0).await?;
    let upstage =
        truss_run(into, "Back truss", None, Vec3 { x: 0.0, y: 7.0, z: -3.0 }, 12.0).await?;
    for (n, x) in [-3.0f32, 0.0, 3.0].into_iter().enumerate() {
        a_piece(
            into,
            &format!("Riser {}", n + 1),
            "deck-2x1",
            Transform {
                position: Vec3 { x, y: 0.6, z: -5.0 },
                ..Transform::default()
            },
            None,
        )
        .await?;
    }

    // Eight positions along each twelve-metre truss, a mover and a wash by turns, all
    // clamped under the bar. Interleaved along it rather than the washes hung on a
    // second line beside it: the first version of this demo put them 600 mm off the
    // truss, and a light that hangs off nothing is what that drew.
    let mut addresses = Addresses::from(1);
    for (row, truss) in [downstage, upstage].into_iter().enumerate() {
        for slot in 0..8u16 {
            let x = -5.25 + 1.5 * slot as f32;
            let n = row * 4 + usize::from(slot / 2) + 1;
            let mount = Mount::along(x);
            let fixture = if slot % 2 == 0 {
                a_clamped_fixture(
                    &format!("Mover {n}"),
                    mover.id,
                    addresses.take(mover.channel_count),
                    truss,
                    under(mount, facing::DOWN),
                    mount,
                )
            } else {
                // The washes are angled at the floor rather than straight down, so
                // the two systems do not simply overlap.
                let aim = if row == 0 { facing::DOWNSTAGE } else { facing::UPSTAGE };
                a_clamped_fixture(
                    &format!("Wash {n}"),
                    wash.id,
                    addresses.take(wash.channel_count),
                    truss,
                    under(mount, aim),
                    mount,
                )
            };
            into.create("fixtures", &fixture).await?;
        }
    }
    for n in 0..4u16 {
        // On the deck, looking up and downstage at the room.
        let fixture = a_fixture(
            &format!("Strobe {}", n + 1),
            strobe.id,
            addresses.take(strobe.channel_count),
            aimed(-4.5 + 3.0 * n as f32, 0.9, -4.6, Vec3 { x: 0.0, y: -0.2, z: 1.0 }),
        );
        into.create("fixtures", &fixture).await?;
    }

    // Two tempos, because the argument for a speed master is that two things can run
    // against different ones and still be edited as one thing each. 128 bpm is the
    // room; the second is half of it, for what should feel slower without being a
    // different rhythm.
    let beat = SpeedMaster {
        id: id(),
        name: "Beat".to_string(),
        bpm: 128.0,
        multiplier: 1.0,
        running: true,
        t0: now_ms(),
    };
    let half = SpeedMaster {
        id: id(),
        name: "Half time".to_string(),
        bpm: 128.0,
        multiplier: 0.5,
        running: true,
        t0: now_ms(),
    };
    into.create("speed_masters", &beat).await?;
    into.create("speed_masters", &half).await?;

    let rig = into.fixtures().await;
    let of = |prefix: &str| -> Vec<Uuid> {
        rig.iter().filter(|f| f.name.starts_with(prefix)).map(|f| f.id).collect()
    };
    let movers = of("Mover");
    let washes = of("Wash");
    let strobes = of("Strobe");

    // ── The movers ────────────────────────────────────────────────────────────
    //
    // One effect id per look, one phase per head, so the effects panel gathers each
    // back into a single editable wave rather than eight unrelated sines.
    let shaped = |on: &[Uuid],
                  kind: ParameterKind,
                  shape,
                  rate,
                  low: ParameterValue,
                  high: ParameterValue,
                  width|
     -> Vec<ParameterCapture> {
        let effect = id();
        let mut captures = Vec::new();
        for (n, fixture) in on.iter().enumerate() {
            captures.push(level(*fixture, 1.0));
            captures.push(ParameterCapture {
                effect: Some(EffectSpec {
                    effect_id: effect,
                    curve: Curve::Shape(shape),
                    rate,
                    low: low.clone(),
                    high: high.clone(),
                    width,
                    direction: Direction::Forward,
                    phase: n as f32 / on.len().max(1) as f32,
                    spread: Spread::Linear,
                    t0: None,
                }),
                ..capture(*fixture, kind.clone(), low.clone())
            });
        }
        captures
    };

    let on_beat = |multiplier: f32| Rate::Master { id: beat.id, multiplier };
    let on_half = |multiplier: f32| Rate::Master { id: half.id, multiplier };

    let mover_looks = vec![
        with_fade(
            a_cue(
                "Tilt wave",
                1.0,
                shaped(&movers, ParameterKind::Tilt, Shape::Sine, on_half(1.0),
                       ParameterValue::Float(0.25), ParameterValue::Float(0.75), 0.5),
            ),
            1_500,
        ),
        with_fade(
            a_cue(
                "Pan sweep",
                2.0,
                shaped(&movers, ParameterKind::Pan, Shape::Triangle, on_half(0.5),
                       ParameterValue::Float(0.2), ParameterValue::Float(0.8), 0.5),
            ),
            2_000,
        ),
        with_fade(
            a_cue(
                "Snap positions",
                3.0,
                shaped(&movers, ParameterKind::Pan, Shape::Square, on_beat(1.0),
                       ParameterValue::Float(0.3), ParameterValue::Float(0.7), 0.5),
            ),
            0,
        ),
        // Not everything is an effect: a static look is a cue too, and it is what
        // an operator drops to when the room needs to calm down.
        with_fade(
            a_cue(
                "Centre",
                4.0,
                movers
                    .iter()
                    .flat_map(|fixture| {
                        [
                            level(*fixture, 0.6),
                            capture(*fixture, ParameterKind::Pan, ParameterValue::Float(0.5)),
                            capture(*fixture, ParameterKind::Tilt, ParameterValue::Float(0.5)),
                        ]
                    })
                    .collect(),
            ),
            3_000,
        ),
        with_fade(
            a_cue("Movers out", 5.0, movers.iter().map(|f| level(*f, 0.0)).collect()),
            2_000,
        ),
    ];

    let wash_looks = vec![
        with_fade(
            a_cue(
                "Colour chase",
                1.0,
                shaped(&washes, ParameterKind::ColorRgb, Shape::Sine, on_beat(0.25),
                       ParameterValue::rgb(1.0, 0.0, 0.4), ParameterValue::rgb(0.0, 0.6, 1.0), 0.5),
            ),
            2_000,
        ),
        with_fade(
            a_cue(
                "Warm",
                2.0,
                washes.iter().flat_map(|f| [level(*f, 0.7), hue(*f, 1.0, 0.55, 0.2)]).collect(),
            ),
            4_000,
        ),
        with_fade(
            a_cue(
                "Deep blue",
                3.0,
                washes.iter().flat_map(|f| [level(*f, 0.5), hue(*f, 0.05, 0.1, 0.9)]).collect(),
            ),
            4_000,
        ),
        with_fade(
            a_cue(
                "Level chase",
                4.0,
                shaped(&washes, ParameterKind::Intensity, Shape::SawDown, on_beat(1.0),
                       ParameterValue::Float(0.05), ParameterValue::Float(1.0), 0.5),
            ),
            500,
        ),
    ];

    // And the strobes, stepping rather than sweeping: a square wave is a strobe that
    // is on or off, which is what a step list says and a sine cannot.
    let strobe_looks = vec![
        with_fade(
            a_cue(
                "Hits",
                1.0,
                shaped(&strobes, ParameterKind::Intensity, Shape::Square, on_beat(1.0),
                       ParameterValue::Float(0.0), ParameterValue::Float(1.0), 0.2),
            ),
            0,
        ),
        // A strobe channel carries a *rate*: the console sends the byte and the
        // fixture does the flashing, so there is nothing here for the evaluator to
        // work out.
        with_fade(
            a_cue(
                "Run them",
                2.0,
                strobes
                    .iter()
                    .flat_map(|f| {
                        [
                            level(*f, 1.0),
                            capture(*f, ParameterKind::Strobe, ParameterValue::Float(0.6)),
                        ]
                    })
                    .collect(),
            ),
            0,
        ),
        with_fade(
            a_cue("Strobes out", 3.0, strobes.iter().map(|f| level(*f, 0.0)).collect()),
            1_000,
        ),
    ];

    // Each left *running*, so the show has something in it the moment it opens
    // rather than waiting to be told to.
    a_stack(into, "Movers", mover_looks, true).await?;
    a_stack(into, "Washes", wash_looks, true).await?;
    a_stack(into, "Strobes", strobe_looks, true).await?;

    Ok(())
}

fn with_fade(mut cue: Cue, fade_in_ms: u32) -> Cue {
    cue.fade_in_ms = fade_in_ms;
    cue
}
