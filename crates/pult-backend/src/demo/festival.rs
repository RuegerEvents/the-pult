//! A festival rig: five kinds of light on six trusses and the floor, seven playbacks
//! deep, and not everything on at once.
//!
//! The show to open when the question is what the console does under a rig somebody
//! would actually hire. Three things it has that the smaller demos do not:
//!
//! - **Systems that are different things.** A front truss of profiles, washes and
//!   spots by turns over the stage, a row of blinders on the downstage face, beams
//!   and LED strobes along the back wall, a floor package standing on the deck, and
//!   a wash tower each side. Each is its own playback with a short stack of looks
//!   that end in *Out* — the shape a festival operator busks a set on, one hand per
//!   system and Go on whichever the song wants next.
//! - **Some of it is running and some of it is waiting.** The front, the washes, the
//!   spots, the back wall and the towers come up in a look; the blinders and the floor
//!   package sit at nothing until somebody presses Go. A rig where everything is
//!   asserted at once is a rig where nothing reads.
//! - **Layers**, one per system, so hiding the back wall to see the overheads is one
//!   click and the Layers panel has something to be about.
//!
//! The trusses are runs of real sections and the heads hang off them, so dragging a
//! truss moves its lights. Deterministic, and it needs no asset: everything comes out
//! of the console's own catalogue, so opening this costs no download and no network.

use anyhow::Result;
use pult_schema::types::{
    cue::{Cue, ParameterCapture},
    effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread},
    fixture::{Fixture, FixtureType, ParameterKind, ParameterValue, Vec3},
    mount::Mount,
    scene::{Layer, Transform},
    speedmaster::SpeedMaster,
};
use uuid::Uuid;

use super::{
    id,
    kit::{
        a_clamped_fixture, a_cue, a_fixture, a_piece, a_stack, a_type_with_beam, aimed, boom,
        capture, colour, facing, hue, intensity, level, on, pan, strobe_rate, tilt, truss_run,
        under, Addresses,
    },
    now_ms, Seeder,
};

/// Fifteen metres of overhead truss, which is five three-metre lengths.
const SPAN: f32 = 15.0;

/// The five kinds of light in this rig, as indices into the types the seed makes.
#[derive(Clone, Copy)]
enum Kind {
    Spot,
    Wash,
    Beam,
    Blinder,
    Strobe,
}

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Festival").await?;

    // Five types, each with the beam angle the rig view draws it at: a wash is wide,
    // a beam is a pencil, and a blinder is a lamp with no lens at all.
    let spot = a_type_with_beam("Spot 350", vec![intensity(), colour(), pan(), tilt()], 14.0);
    let wash = a_type_with_beam("Wash 19×40W", vec![intensity(), colour(), pan(), tilt()], 28.0);
    let beam = a_type_with_beam("Beam 7R", vec![intensity(), colour(), pan(), tilt()], 4.0);
    let blinder = a_type_with_beam("Blinder 4-lite", vec![intensity()], 45.0);
    let strobe =
        a_type_with_beam("LED Strobe", vec![intensity(), colour(), strobe_rate()], 60.0);
    let types = [&spot, &wash, &beam, &blinder, &strobe];
    for kind in types {
        into.create("fixture_types", kind).await?;
    }
    let type_of = |kind: Kind| -> &FixtureType {
        match kind {
            Kind::Spot => &spot,
            Kind::Wash => &wash,
            Kind::Beam => &beam,
            Kind::Blinder => &blinder,
            Kind::Strobe => &strobe,
        }
    };
    let called = |kind: Kind| match kind {
        Kind::Spot => "Spot",
        Kind::Wash => "Wash",
        Kind::Beam => "Beam",
        Kind::Blinder => "Blinder",
        Kind::Strobe => "Strobe",
    };

    // A layer per system. A layer is a thing an operator points at, and "the back
    // wall" is what they point at when the overheads are in the way of it.
    let mut layers = Vec::new();
    for (n, name) in ["Front of house", "Overhead", "Back wall", "Floor", "Side"]
        .into_iter()
        .enumerate()
    {
        let layer = Layer { id: id(), name: name.to_string(), locked: false, sort_order: n as u32 };
        into.create("layers", &layer).await?;
        layers.push(layer.id);
    }
    let [foh, overhead, back_wall, floor, side]: [Uuid; 5] =
        layers.try_into().expect("five layers");

    // ── The room ──────────────────────────────────────────────────────────────
    //
    // A sixteen-by-ten stage a metre up, out of decks scaled to four-by-two, and a
    // wall of flats behind it. The audience is at +Z.
    for row in 0..5 {
        for column in 0..4 {
            a_piece(
                into,
                &format!("Deck {}", row * 4 + column + 1),
                "deck-2x1",
                Transform {
                    position: Vec3 { x: -6.0 + 4.0 * column as f32, y: 1.0, z: 4.0 - 2.0 * row as f32 },
                    scale: Vec3 { x: 2.0, y: 1.0, z: 2.0 },
                    ..Transform::default()
                },
                None,
            )
            .await?;
        }
    }
    for n in 0..8 {
        a_piece(
            into,
            &format!("Backdrop {}", n + 1),
            "flat-1x24",
            Transform {
                position: Vec3 { x: -7.0 + 2.0 * n as f32, y: 1.0, z: -8.5 },
                scale: Vec3 { x: 2.0, y: 2.0, z: 1.0 },
                ..Transform::default()
            },
            None,
        )
        .await?;
    }

    // ── The trusses ───────────────────────────────────────────────────────────
    //
    // Front of house out over the audience; four overheads stepping back and up over
    // the stage so the rig reads as a stack rather than a wall; a low one along the
    // back for the eye candy; and a tower each side, stood on the floor.
    let foh_truss =
        truss_run(into, "FOH truss", Some(foh), Vec3 { x: 0.0, y: 7.0, z: 11.0 }, SPAN).await?;
    let mut overheads = Vec::new();
    for n in 0..4 {
        overheads.push(
            truss_run(
                into,
                &format!("Truss {}", n + 1),
                Some(overhead),
                Vec3 { x: 0.0, y: 8.5 + n as f32 * 0.35, z: 4.0 - n as f32 * 3.0 },
                SPAN,
            )
            .await?,
        );
    }
    let back_truss =
        truss_run(into, "Back truss", Some(back_wall), Vec3 { x: 0.0, y: 6.0, z: -7.5 }, SPAN)
            .await?;
    let towers = [
        boom(into, "Tower SL", Some(side), Vec3 { x: -10.0, y: 3.0, z: 0.0 }, 6.0).await?,
        boom(into, "Tower SR", Some(side), Vec3 { x: 10.0, y: 3.0, z: 0.0 }, 6.0).await?,
    ];

    // ── The hang ──────────────────────────────────────────────────────────────
    //
    // A truss is hung as a *pattern* along it — "SWB" is a spot, a wash and a blinder
    // by turns — spread evenly over the span and clamped under the bar. Addressed
    // straight through, rolling into the next universe when one is full.
    let mut addresses = Addresses::from(1);
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    let mut hang = |truss: Uuid,
                    layer: Uuid,
                    pattern: &[Kind],
                    repeats: usize,
                    aim: Vec3|
     -> Vec<(Kind, Fixture)> {
        let slots = pattern.len() * repeats;
        let step = SPAN / slots as f32;
        let mut made = Vec::new();
        for slot in 0..slots {
            let kind = pattern[slot % pattern.len()];
            let definition = type_of(kind);
            let n = counts.entry(called(kind)).or_default();
            *n += 1;
            let mount = Mount::along(-SPAN / 2.0 + step * (slot as f32 + 0.5));
            let mut fixture = a_clamped_fixture(
                &format!("{} {}", called(kind), n),
                definition.id,
                addresses.take(definition.channel_count),
                truss,
                under(mount, aim),
                mount,
            );
            fixture.layer = Some(layer);
            made.push((kind, fixture));
        }
        made
    };

    let mut hung: Vec<(Kind, Fixture)> = Vec::new();
    // Profiles out front, pointed back at the stage.
    let at_the_stage = Vec3 { x: 0.0, y: -0.5, z: -0.85 };
    hung.extend(hang(foh_truss, foh, &[Kind::Spot], 16, at_the_stage));
    // The downstage truss carries the blinders, which look out at the crowd; the
    // other three are spots and washes by turns, straight down.
    let at_the_crowd = Vec3 { x: 0.0, y: -0.35, z: 1.0 };
    for (n, truss) in overheads.iter().enumerate() {
        if n == 0 {
            hung.extend(hang(*truss, overhead, &[Kind::Spot, Kind::Wash, Kind::Blinder], 10, facing::DOWN));
        } else {
            hung.extend(hang(*truss, overhead, &[Kind::Spot, Kind::Wash], 13, facing::DOWN));
        }
    }
    // Beams and strobes along the back wall, over the band's heads at the crowd.
    hung.extend(hang(back_truss, back_wall, &[Kind::Beam, Kind::Strobe], 14, at_the_crowd));

    // A blinder hung straight down is a blinder nobody sees. Turn that row at the
    // audience after the fact, since the pattern hung the whole truss one way.
    for (kind, fixture) in &mut hung {
        if matches!(kind, Kind::Blinder) {
            let at = fixture.position.expect("hung").position;
            fixture.position = Some(aimed(at.x, at.y, at.z, at_the_crowd));
        }
    }
    for (_, fixture) in &hung {
        into.create("fixtures", fixture).await?;
    }

    // The floor package: a dozen beams standing on the deck along the back edge,
    // fanned up and out at the crowd. Not hung off anything — they stand.
    for n in 0..12 {
        let mut fixture = a_fixture(
            &format!("Floor beam {}", n + 1),
            beam.id,
            addresses.take(beam.channel_count),
            aimed(-6.75 + 1.5 * n as f32 - 0.375, 1.2, -6.5, Vec3 { x: 0.0, y: 0.55, z: 1.0 }),
        );
        fixture.layer = Some(floor);
        into.create("fixtures", &fixture).await?;
    }

    // And six washes up each tower on sidearms, looking across the stage.
    for (t, (tower, handle)) in towers.iter().enumerate() {
        // Clamped to the chord that faces centre stage. A tower is a run stood on its
        // end, so its own X runs up the world's Y and `along` is the height up it —
        // which is what makes the mount the same two degrees here as on a bar.
        // The clamp goes on the chord facing centre stage, and the roll pushes the
        // body out along it: a tower is a run turned a quarter about Z, so its local
        // −Y is the world's +X.
        let (side_name, aim, chord, roll) = if t == 0 {
            ("SL", facing::FROM_LEFT, 0, 0.0)
        } else {
            ("SR", facing::FROM_RIGHT, 2, 180.0)
        };
        for n in 0..6 {
            let mount = Mount { chord, along: -2.4 + 0.96 * n as f32, roll };
            let mut fixture = a_clamped_fixture(
                &format!("Tower {side_name} {}", n + 1),
                wash.id,
                addresses.take(wash.channel_count),
                *tower,
                on(handle, mount, aim),
                mount,
            );
            fixture.layer = Some(side);
            into.create("fixtures", &fixture).await?;
        }
    }

    // ── The playbacks ─────────────────────────────────────────────────────────

    let master = SpeedMaster {
        id: id(),
        name: "Show".to_string(),
        bpm: 128.0,
        multiplier: 1.0,
        running: true,
        t0: now_ms(),
    };
    into.create("speed_masters", &master).await?;
    let on_beat = |multiplier: f32| Rate::Master { id: master.id, multiplier };

    let rig = into.fixtures().await;
    let of = |prefix: &str| -> Vec<Uuid> {
        rig.iter().filter(|f| f.name.starts_with(prefix)).map(|f| f.id).collect()
    };
    let front: Vec<Uuid> = rig
        .iter()
        .filter(|f| f.layer == Some(foh))
        .map(|f| f.id)
        .collect();
    let washes: Vec<Uuid> = rig
        .iter()
        .filter(|f| f.layer == Some(overhead) && f.fixture_type_id == wash.id)
        .map(|f| f.id)
        .collect();
    let spots: Vec<Uuid> = rig
        .iter()
        .filter(|f| f.layer == Some(overhead) && f.fixture_type_id == spot.id)
        .map(|f| f.id)
        .collect();
    let blinders = of("Blinder");
    let beams = of("Beam");
    let strobes = of("Strobe");
    let floor_beams = of("Floor beam");
    let towers: Vec<Uuid> = of("Tower");

    // One effect id per look and one phase per head, so the effects panel gathers
    // each back into a single editable wave rather than thirty unrelated sines.
    let shaped = |on: &[Uuid],
                  kind: ParameterKind,
                  shape: Shape,
                  rate: Rate,
                  low: ParameterValue,
                  high: ParameterValue,
                  width: f32,
                  direction: Direction|
     -> Vec<ParameterCapture> {
        let effect = id();
        on.iter()
            .enumerate()
            .map(|(n, fixture)| ParameterCapture {
                effect: Some(EffectSpec {
                    effect_id: effect,
                    curve: Curve::Shape(shape),
                    rate,
                    low: low.clone(),
                    high: high.clone(),
                    width,
                    direction,
                    phase: n as f32 / on.len().max(1) as f32,
                    spread: Spread::Linear,
                    t0: None,
                }),
                ..capture(*fixture, kind.clone(), low.clone())
            })
            .collect()
    };
    let all = |on: &[Uuid], at: f32| -> Vec<ParameterCapture> {
        on.iter().map(|f| level(*f, at)).collect()
    };
    let coloured = |on: &[Uuid], at: f32, r: f32, g: f32, b: f32| -> Vec<ParameterCapture> {
        on.iter().flat_map(|f| [level(*f, at), hue(*f, r, g, b)]).collect()
    };
    // Pan spread evenly across a system, so a row of heads fans out rather than all
    // pointing the same way.
    let fanned = |on: &[Uuid], from: f32, to: f32| -> Vec<ParameterCapture> {
        on.iter()
            .enumerate()
            .flat_map(|(n, f)| {
                let t = if on.len() > 1 { n as f32 / (on.len() - 1) as f32 } else { 0.5 };
                [
                    capture(*f, ParameterKind::Pan, ParameterValue::Float(from + (to - from) * t)),
                    capture(*f, ParameterKind::Tilt, ParameterValue::Float(0.5)),
                ]
            })
            .collect()
    };
    let joined = |parts: Vec<Vec<ParameterCapture>>| -> Vec<ParameterCapture> {
        parts.into_iter().flatten().collect()
    };
    let out = |on: &[Uuid], number: f64| with_fade(a_cue("Out", number, all(on, 0.0)), 2_000);

    // Front: the profiles on the stage. Comes up in the wash and stays there.
    a_stack(
        into,
        "Front",
        vec![
            with_fade(a_cue("Stage wash", 1.0, coloured(&front, 0.85, 1.0, 0.92, 0.8)), 3_000),
            with_fade(
                a_cue(
                    "Key and fill",
                    2.0,
                    front
                        .iter()
                        .enumerate()
                        .flat_map(|(n, f)| {
                            // The middle of the truss keys the stage; the ends fill.
                            let middle = (5..11).contains(&n);
                            [level(*f, if middle { 1.0 } else { 0.4 }), hue(*f, 1.0, 0.92, 0.8)]
                        })
                        .collect(),
                ),
                2_000,
            ),
            out(&front, 3.0),
        ],
        true,
    )
    .await?;

    // Washes: the overhead colour, rolling slowly the moment the show opens.
    a_stack(
        into,
        "Washes",
        vec![
            with_fade(
                a_cue(
                    "Colour roll",
                    1.0,
                    joined(vec![
                        all(&washes, 0.7),
                        shaped(&washes, ParameterKind::ColorRgb, Shape::Sine, on_beat(0.125),
                               ParameterValue::rgb(1.0, 0.25, 0.05), ParameterValue::rgb(0.05, 0.2, 1.0),
                               0.5, Direction::Forward),
                    ]),
                ),
                3_000,
            ),
            with_fade(a_cue("Deep blue", 2.0, coloured(&washes, 0.6, 0.05, 0.15, 0.9)), 4_000),
            with_fade(a_cue("Amber", 3.0, coloured(&washes, 0.8, 1.0, 0.55, 0.15)), 4_000),
            with_fade(a_cue("White", 4.0, coloured(&washes, 0.5, 1.0, 1.0, 1.0)), 2_000),
            out(&washes, 5.0),
        ],
        true,
    )
    .await?;

    // Spots: the overhead movers. A slow tilt wave to open on, and busier looks to
    // Go to.
    a_stack(
        into,
        "Spots",
        vec![
            with_fade(
                a_cue(
                    "Tilt wave",
                    1.0,
                    joined(vec![
                        coloured(&spots, 0.5, 0.9, 0.95, 1.0),
                        shaped(&spots, ParameterKind::Tilt, Shape::Sine, on_beat(0.25),
                               ParameterValue::Float(0.35), ParameterValue::Float(0.65), 0.5,
                               Direction::Forward),
                    ]),
                ),
                3_000,
            ),
            with_fade(
                a_cue("Fan", 2.0, joined(vec![coloured(&spots, 0.8, 1.0, 1.0, 1.0), fanned(&spots, 0.3, 0.7)])),
                2_500,
            ),
            with_fade(
                a_cue(
                    "Ballyhoo",
                    3.0,
                    joined(vec![
                        coloured(&spots, 0.9, 1.0, 1.0, 1.0),
                        shaped(&spots, ParameterKind::Pan, Shape::Triangle, on_beat(0.5),
                               ParameterValue::Float(0.3), ParameterValue::Float(0.7), 0.5,
                               Direction::Forward),
                        shaped(&spots, ParameterKind::Tilt, Shape::Sine, on_beat(1.0),
                               ParameterValue::Float(0.4), ParameterValue::Float(0.6), 0.5,
                               Direction::Backward),
                    ]),
                ),
                1_000,
            ),
            with_fade(
                a_cue("Centre", 4.0, joined(vec![coloured(&spots, 0.6, 1.0, 1.0, 1.0), fanned(&spots, 0.5, 0.5)])),
                3_000,
            ),
            out(&spots, 5.0),
        ],
        true,
    )
    .await?;

    // Blinders: waiting. A blinder that is on when the show opens is a blinder that
    // has nothing left to do, so this stack sits at nothing until somebody presses
    // Go — and the first thing Go does is hit.
    a_stack(
        into,
        "Blinders",
        vec![
            with_fade(a_cue("Hit", 1.0, all(&blinders, 1.0)), 0),
            with_fade(
                a_cue(
                    "Pulse",
                    2.0,
                    shaped(&blinders, ParameterKind::Intensity, Shape::Square, on_beat(0.5),
                           ParameterValue::Float(0.0), ParameterValue::Float(1.0), 0.3,
                           Direction::Forward),
                ),
                0,
            ),
            with_fade(a_cue("Glow", 3.0, all(&blinders, 0.12)), 3_000),
            out(&blinders, 4.0),
        ],
        false,
    )
    .await?;

    // The back wall: beams chasing along it, with the strobes held off until their
    // own look — a strobe in a chase is noise, not eye candy.
    a_stack(
        into,
        "Back wall",
        vec![
            with_fade(
                a_cue(
                    "Beam chase",
                    1.0,
                    joined(vec![
                        beams.iter().flat_map(|f| [hue(*f, 0.2, 0.9, 1.0)]).collect(),
                        fanned(&beams, 0.42, 0.58),
                        shaped(&beams, ParameterKind::Intensity, Shape::SawDown, on_beat(1.0),
                               ParameterValue::Float(0.0), ParameterValue::Float(1.0), 0.5,
                               Direction::Forward),
                        all(&strobes, 0.0),
                    ]),
                ),
                1_000,
            ),
            with_fade(
                a_cue(
                    "Beam fan",
                    2.0,
                    joined(vec![coloured(&beams, 0.9, 0.9, 0.3, 1.0), fanned(&beams, 0.25, 0.75), all(&strobes, 0.0)]),
                ),
                2_000,
            ),
            with_fade(
                a_cue(
                    "Strobe hits",
                    3.0,
                    joined(vec![
                        all(&beams, 0.0),
                        strobes
                            .iter()
                            .flat_map(|f| {
                                [
                                    level(*f, 1.0),
                                    hue(*f, 1.0, 1.0, 1.0),
                                    // A strobe channel carries a rate: the console
                                    // sends the byte and the fixture does the flashing.
                                    capture(*f, ParameterKind::Strobe, ParameterValue::Float(0.5)),
                                ]
                            })
                            .collect(),
                    ]),
                ),
                0,
            ),
            with_fade(
                a_cue(
                    "Out",
                    4.0,
                    joined(vec![
                        all(&beams, 0.0),
                        strobes
                            .iter()
                            .flat_map(|f| [level(*f, 0.0), capture(*f, ParameterKind::Strobe, ParameterValue::Float(0.0))])
                            .collect(),
                    ]),
                ),
                1_000,
            ),
        ],
        true,
    )
    .await?;

    // The floor package: waiting, like the blinders. Its first look fans the beams
    // up over the crowd.
    a_stack(
        into,
        "Floor",
        vec![
            with_fade(
                a_cue("Fan up", 1.0, joined(vec![coloured(&floor_beams, 0.8, 1.0, 0.1, 0.6), fanned(&floor_beams, 0.3, 0.7)])),
                1_500,
            ),
            with_fade(
                a_cue(
                    "Sweep",
                    2.0,
                    joined(vec![
                        coloured(&floor_beams, 0.9, 0.3, 0.6, 1.0),
                        shaped(&floor_beams, ParameterKind::Pan, Shape::Triangle, on_beat(0.25),
                               ParameterValue::Float(0.3), ParameterValue::Float(0.7), 0.5,
                               Direction::Forward),
                    ]),
                ),
                1_000,
            ),
            out(&floor_beams, 3.0),
        ],
        false,
    )
    .await?;

    // The towers: a colour from each side, and a chase to Go to.
    a_stack(
        into,
        "Side",
        vec![
            with_fade(a_cue("Magenta", 1.0, coloured(&towers, 0.7, 1.0, 0.1, 0.6)), 3_000),
            with_fade(a_cue("Cyan", 2.0, coloured(&towers, 0.7, 0.1, 0.8, 1.0)), 3_000),
            with_fade(
                a_cue(
                    "Colour chase",
                    3.0,
                    joined(vec![
                        all(&towers, 0.8),
                        shaped(&towers, ParameterKind::ColorRgb, Shape::Sine, on_beat(0.5),
                               ParameterValue::rgb(1.0, 0.1, 0.6), ParameterValue::rgb(0.1, 0.8, 1.0),
                               0.5, Direction::Forward),
                    ]),
                ),
                1_000,
            ),
            out(&towers, 4.0),
        ],
        true,
    )
    .await?;

    Ok(())
}

fn with_fade(mut cue: Cue, fade_in_ms: u32) -> Cue {
    cue.fade_in_ms = fade_in_ms;
    cue
}
