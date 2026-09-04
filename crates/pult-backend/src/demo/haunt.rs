//! The small demo: five fixtures, a short stack, two flows.
//!
//! A port of what `scripts/demo-seed.mjs` used to seed over the WebSocket, moved in
//! here so that opening it is a card on the welcome screen rather than a terminal.
//! What it shows is deliberately the whole console in miniature — a patch, a rig
//! hanging off truss, a cue stack, an effect on a speed master, and two flow graphs,
//! one of which does the thing a one-row-per-rule trigger never could.

use anyhow::Result;
use pult_schema::types::{
    effect::{Curve, Direction, EffectSpec, Rate, Shape, Spread},
    fixture::{Fixture, ParameterKind, ParameterValue, Vec3},
    mount::Mount,
    flow::{FlowNodeKind, TriggerAction, TriggerCondition, TriggerSource},
    speedmaster::SpeedMaster,
};

use super::{
    id,
    kit::{
        a_clamped_fixture, a_cue, a_stack, a_type, capture, colour, draw, facing, intensity, level,
        pan, tilt, truss_run, under, Addresses,
    },
    now_ms, Seeder,
};

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Haunt").await?;

    // One ordinary DMX fixture type, so the Patch panel has something in it and an
    // Art-Net output has something to send.
    let dimmer = a_type("Dimmer", vec![intensity()]);
    into.create("fixture_types", &dimmer).await?;

    // And a moving head, so there is something to puppeteer. Nothing binds a
    // channel: where a parameter sits belongs to a mode, and a type that names none
    // has the implicit one — intensity at 1, the colour across 2 to 4, pan at 5,
    // tilt at 6.
    let spot = a_type("Spot", vec![intensity(), colour(), pan(), tilt()]);
    into.create("fixture_types", &spot).await?;

    // Two bars to hang it all off, so the rig view has structure in it rather than
    // five lights floating in the dark. Metres: X to the right as seen from front of
    // house, Y up, Z downstage towards the audience.
    let front = truss_run(into, "FOH bar", None, Vec3 { x: 0.0, y: 4.5, z: 2.0 }, 9.0).await?;
    let back = truss_run(into, "Back bar", None, Vec3 { x: 0.0, y: 5.0, z: -2.0 }, 9.0).await?;

    let mut addresses = Addresses::from(1);
    // Hung *off the bars*: a fixture's position is relative to whatever it hangs on,
    // so dragging a bar takes its lights with it.
    for (name, x, aim, bar) in [
        ("Front left", -3.0, facing::DOWNSTAGE, front),
        ("Front right", 3.0, facing::DOWNSTAGE, front),
        ("Backlight", 0.0, facing::UPSTAGE, back),
    ] {
        let mount = Mount::along(x);
        let fixture = a_clamped_fixture(
            name,
            dimmer.id,
            addresses.take(dimmer.channel_count),
            bar,
            under(mount, aim),
            mount,
        );
        into.create("fixtures", &fixture).await?;
    }

    for (name, x) in [("Head left", -2.5), ("Head right", 2.5)] {
        let mount = Mount::along(x);
        let fixture = a_clamped_fixture(
            name,
            spot.id,
            addresses.take(spot.channel_count),
            back,
            // Hanging: a moving head rests pointing at the floor, and pan and tilt
            // are angles away from that.
            under(mount, facing::DOWN),
            mount,
        );
        into.create("fixtures", &fixture).await?;
    }

    let rig = into.fixtures().await;
    let movers: Vec<&Fixture> =
        rig.iter().filter(|fixture| fixture.name.starts_with("Head")).collect();
    let fronts: Vec<&Fixture> =
        rig.iter().filter(|fixture| fixture.name.starts_with("Front")).collect();

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
    let sine = |fixture: &Fixture, phase: f32| pult_schema::types::cue::ParameterCapture {
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

    let everything = |at: f32| rig.iter().map(|f| level(f.id, at)).collect::<Vec<_>>();

    // A stack short enough to read and long enough to be one: seven cues, each doing
    // something different to the same five lights.
    let mut cues = Vec::new();

    let mut house = a_cue("House", 1.0, everything(0.2));
    house.fade_in_ms = 2_000;
    house.fade_out_ms = 2_000;
    cues.push(house);

    // Only the fronts, and slowly: what a preset looks like.
    let mut preset = a_cue(
        "Preset",
        2.0,
        fronts.iter().map(|f| level(f.id, 0.45)).collect(),
    );
    preset.fade_in_ms = 5_000;
    preset.fade_out_ms = 2_000;
    cues.push(preset);

    let mut scare = a_cue("Scare", 3.0, everything(1.0));
    scare.fade_in_ms = 3_000;
    scare.fade_out_ms = 1_500;
    cues.push(scare);

    // Everything up, and the two heads cycling through colour against each other on
    // the speed master.
    let mut possession = a_cue("Possession", 4.0, everything(0.8));
    possession.fade_in_ms = 1_000;
    possession.fade_out_ms = 1_000;
    for (index, mover) in movers.iter().enumerate() {
        possession.captures.push(sine(mover, index as f32 * 0.5));
    }
    cues.push(possession);

    // The heads alone, aimed apart: pan and tilt as stored values rather than an
    // effect, which is the other half of what a cue can hold.
    let mut apart = a_cue("Look away", 5.0, Vec::new());
    apart.fade_in_ms = 4_000;
    for (index, mover) in movers.iter().enumerate() {
        let side = if index == 0 { 0.3 } else { 0.7 };
        apart.captures.push(level(mover.id, 0.9));
        apart
            .captures
            .push(capture(mover.id, ParameterKind::Pan, ParameterValue::Float(side)));
        apart
            .captures
            .push(capture(mover.id, ParameterKind::Tilt, ParameterValue::Float(0.65)));
    }
    cues.push(apart);

    // A cue that splits its fade: the backlight comes up over eight seconds while
    // everything else goes out over one. Invisible in a show that never sets one,
    // and the reason `fade_out_ms` exists.
    let mut reveal = a_cue("Reveal", 6.0, everything(0.0));
    reveal.fade_in_ms = 8_000;
    reveal.fade_out_ms = 1_000;
    if let Some(back_light) = rig.iter().find(|f| f.name == "Backlight") {
        reveal.captures.retain(|c| c.fixture_id != back_light.id);
        reveal.captures.push(level(back_light.id, 1.0));
    }
    cues.push(reveal);

    let mut out = a_cue("Blackout", 7.0, everything(0.0));
    out.fade_in_ms = 3_000;
    cues.push(out);

    let scare_id = cues[2].id;
    let sequence = a_stack(into, "Haunt", cues, false).await?;

    // Two graphs for the Flows panel. The first is a chain anyone can set off by
    // hand; the second is the thing a one-row-per-rule trigger could never say.
    draw(
        into,
        "Panic button",
        &[
            (FlowNodeKind::Button, 40.0, 60.0),
            (FlowNodeKind::Delay { ms: 1_500 }, 280.0, 60.0),
            (FlowNodeKind::Action(TriggerAction::GoNext { sequence_id: sequence }), 520.0, 60.0),
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
            (watching(fronts[0]), 40.0, 40.0),
            (watching(fronts[1]), 40.0, 180.0),
            (FlowNodeKind::And, 300.0, 100.0),
            (FlowNodeKind::Condition(TriggerCondition::RisingEdge), 540.0, 100.0),
            (
                FlowNodeKind::Action(TriggerAction::GoToCue {
                    sequence_id: sequence,
                    cue_id: scare_id,
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
