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
use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        cue::ParameterCapture,
        effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread},
        fixture::{ParameterKind, ParameterValue},
        sequence::Sequence,
        speedmaster::SpeedMaster,
    },
};
use uuid::Uuid;

use super::{
    haunt::{a_cue, a_fixture, a_type, aimed, capture, colour, dmx, intensity, pan, tilt},
    id, now_ms, Seeder,
};
use pult_schema::types::fixture::Vec3;

/// Straight down off a truss, which is where a head rests before anything aims it.
const HANGING: Vec3 = Vec3 { x: 90.0, y: 0.0, z: 0.0 };

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Club").await?;

    let mover = a_type("Beam 7R", vec![intensity(), colour(), pan(), tilt()]);
    let wash = a_type("LED Wash", vec![intensity(), colour()]);
    let strobe = a_type("Strobe", vec![intensity()]);
    for kind in [&mover, &wash, &strobe] {
        into.create("fixture_types", kind).await?;
    }

    // Two trusses, downstage and upstage, at different heights so the rig view is
    // not a single flat row.
    let mut next = 1u16;
    for (row, (z, y)) in [(3.0f32, 6.0f32), (-3.0, 7.0)].into_iter().enumerate() {
        for n in 0..4u16 {
            let x = -4.5 + 3.0 * n as f32;
            let fixture = a_fixture(
                &format!("Mover {}", row * 4 + n as usize + 1),
                mover.id,
                dmx(1, next),
                aimed(x, y, z, HANGING),
            );
            into.create("fixtures", &fixture).await?;
            next += mover.channel_count;
        }
        for n in 0..4u16 {
            let x = -6.0 + 4.0 * n as f32;
            let fixture = a_fixture(
                &format!("Wash {}", row * 4 + n as usize + 1),
                wash.id,
                dmx(1, next),
                aimed(x, y, z, HANGING),
            );
            into.create("fixtures", &fixture).await?;
            next += wash.channel_count;
        }
    }
    for n in 0..4u16 {
        let fixture = a_fixture(
            &format!("Strobe {}", n + 1),
            strobe.id,
            dmx(1, next),
            aimed(-6.0 + 4.0 * n as f32, 2.0, 0.0, HANGING),
        );
        into.create("fixtures", &fixture).await?;
        next += strobe.channel_count;
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

    // A tilt wave across the movers: one effect id, one phase per head, so the
    // effects panel gathers them back into a single editable wave rather than eight
    // unrelated sines. Spread is what turns the phase offsets into a wave rather than
    // eight heads doing the same thing at different times for no reason.
    let wave = id();
    let mut tilt_wave = a_cue("Tilt wave", 1.0, Vec::new());
    tilt_wave.fade_in_ms = 1_500;
    for (n, fixture) in movers.iter().enumerate() {
        tilt_wave
            .captures
            .push(capture(*fixture, ParameterKind::Intensity, ParameterValue::Float(1.0)));
        tilt_wave.captures.push(ParameterCapture {
            effect: Some(EffectSpec {
                effect_id: wave,
                curve: Curve::Shape(Shape::Sine),
                rate: Rate::Master { id: half.id, multiplier: 1.0 },
                low: ParameterValue::Float(0.25),
                high: ParameterValue::Float(0.75),
                width: 0.5,
                direction: Direction::Forward,
                phase: n as f32 / movers.len() as f32,
                spread: Spread::Linear,
                t0: None,
            }),
            ..capture(*fixture, ParameterKind::Tilt, ParameterValue::Float(0.5))
        });
    }

    // A colour chase across the washes, on the beat.
    let chase = id();
    let mut colour_chase = a_cue("Colour chase", 1.0, Vec::new());
    colour_chase.fade_in_ms = 2_000;
    for (n, fixture) in washes.iter().enumerate() {
        colour_chase
            .captures
            .push(capture(*fixture, ParameterKind::Intensity, ParameterValue::Float(0.8)));
        colour_chase.captures.push(ParameterCapture {
            effect: Some(EffectSpec {
                effect_id: chase,
                curve: Curve::Shape(Shape::Sine),
                rate: Rate::Master { id: beat.id, multiplier: 0.25 },
                low: ParameterValue::rgb(1.0, 0.0, 0.4),
                high: ParameterValue::rgb(0.0, 0.6, 1.0),
                width: 0.5,
                direction: Direction::Forward,
                phase: n as f32 / washes.len() as f32,
                spread: Spread::Linear,
                t0: None,
            }),
            ..capture(*fixture, ParameterKind::ColorRgb, ParameterValue::rgb(0.0, 0.0, 0.0))
        });
    }

    // And the strobes, stepping rather than sweeping: a square wave is a strobe that
    // is on or off, which is what a step list says and a sine cannot.
    let steps = id();
    let mut hits = a_cue("Strobe hits", 1.0, Vec::new());
    for (n, fixture) in strobes.iter().enumerate() {
        hits.captures.push(ParameterCapture {
            effect: Some(EffectSpec {
                effect_id: steps,
                curve: Curve::Shape(Shape::Square),
                rate: Rate::Master { id: beat.id, multiplier: 1.0 },
                low: ParameterValue::Float(0.0),
                high: ParameterValue::Float(1.0),
                width: 0.2,
                direction: Direction::Forward,
                phase: n as f32 / strobes.len() as f32,
                spread: Spread::Linear,
                t0: None,
            }),
            ..capture(*fixture, ParameterKind::Intensity, ParameterValue::Float(0.0))
        });
    }

    // One sequence each, and each left *running*, so the show has something in it
    // the moment it opens rather than waiting to be told to.
    for (name, cue) in [("Movers", tilt_wave), ("Washes", colour_chase), ("Strobes", hits)] {
        into.create("cues", &cue).await?;
        let sequence = Sequence {
            id: id(),
            name: name.to_string(),
            cue_ids: vec![cue.id],
            active_cue_index: None,
            went_at: None,
        };
        into.create("sequences", &sequence).await?;
        // Through the sequence's own Go, not by writing `active_cue_index`: taking a
        // cue is what anchors `went_at`, and an effect with no anchor renders
        // nothing.
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
