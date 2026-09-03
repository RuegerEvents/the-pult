//! Conventionals and a cue stack: what most of the world actually runs.
//!
//! No movers and no effects, deliberately. The interesting things here are the ones
//! a rig of moving heads hides: systems of identical lanterns that are addressed and
//! grouped by where they point rather than by what they are, a stack of cues that
//! mostly changes a few of them at a time, and **split fade times** — the front
//! coming up over five seconds while the previous state goes out over two is the
//! ordinary shape of a theatre cue and is invisible in a show that never sets one.
//!
//! Two universes, because forty six-channel lanterns is more than one holds once the
//! cyc is in — which is also what makes the Patch panel's universe view worth
//! looking at.

use anyhow::Result;
use pult_schema::types::{
    fixture::{FixtureType, ParameterKind, ParameterValue},
    group::{Group, SelectionClause, SelectionCombine, SelectionOrder, SelectionQuery,
            SelectionTerm},
    sequence::Sequence,
};

use super::{
    haunt::{a_cue, a_fixture, a_type, aimed, capture, colour, dmx, intensity},
    id, Seeder,
};
use pult_schema::types::fixture::Vec3;

/// One lighting system: what it is called, what type it is made of, how many, and
/// where they hang.
struct System {
    name: &'static str,
    /// Index into the types below.
    kind: usize,
    count: u16,
    y: f32,
    z: f32,
    /// XYZ Euler degrees, the direction the whole system points.
    facing: Vec3,
}

/// Straight down, for a cyc batten washing a cloth from above.
const DOWN: Vec3 = Vec3 { x: 90.0, y: 0.0, z: 0.0 };
/// Downstage and down, the front-of-house angle.
const FRONT: Vec3 = Vec3 { x: 143.1301, y: 0.0, z: 180.0 };
/// Upstage and down, for backlight looking the other way.
const BACK: Vec3 = Vec3 { x: 36.8699, y: 0.0, z: 0.0 };
/// Across the stage, for booms.
const ACROSS: Vec3 = Vec3 { x: 110.0, y: 90.0, z: 0.0 };

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Theatre").await?;

    // Three conventionals: a profile to shape, a fresnel to wash, and a par for
    // colour. Only the cyc batten mixes, which is why it is the one with a colour
    // parameter — everything else is a lantern with gel in it, and a console that
    // offered a colour picker for one would be lying.
    let profile = a_type("Profile 26°", vec![intensity()]);
    let fresnel = a_type("Fresnel 1kW", vec![intensity()]);
    let cyc = a_type("LED Cyc Batten", vec![intensity(), colour()]);
    let types: Vec<&FixtureType> = vec![&profile, &fresnel, &cyc];
    for kind in &types {
        into.create("fixture_types", *kind).await?;
    }

    let systems = [
        System { name: "Front", kind: 0, count: 12, y: 6.0, z: 5.0, facing: FRONT },
        System { name: "Back", kind: 0, count: 10, y: 6.5, z: -4.0, facing: BACK },
        System { name: "Side", kind: 1, count: 8, y: 3.0, z: 0.0, facing: ACROSS },
        System { name: "Cyc", kind: 2, count: 8, y: 5.0, z: -6.0, facing: DOWN },
    ];

    // Addressed system by system, and a system that will not fit in what is left of
    // a universe starts the next one — which is how a real patch is laid out, and
    // what makes the second universe exist at all.
    let mut universe = 1u16;
    let mut next = 1u16;
    for system in &systems {
        let kind = types[system.kind];
        if next + system.count * kind.channel_count > 513 {
            universe += 1;
            next = 1;
        }
        for n in 0..system.count {
            // Spread evenly across a twelve-metre opening, so the rig view shows a
            // system rather than a heap.
            let across = if system.count > 1 {
                -6.0 + 12.0 * (n as f32 / (system.count - 1) as f32)
            } else {
                0.0
            };
            // A boom is at the side, not across the front.
            let (x, z) = match system.name {
                "Side" => (if n % 2 == 0 { -7.0 } else { 7.0 }, -3.0 + n as f32 * 1.5),
                _ => (across, system.z),
            };
            let fixture = a_fixture(
                &format!("{} {}", system.name, n + 1),
                kind.id,
                dmx(universe, next),
                aimed(x, system.y, z, system.facing),
            );
            into.create("fixtures", &fixture).await?;
            next += kind.channel_count;
        }
    }

    // A group per system, as a *question* rather than a list: "everything whose name
    // starts with Front" stays true after somebody hangs a thirteenth.
    for system in &systems {
        into.create(
            "groups",
            &Group {
                id: id(),
                name: system.name.to_string(),
                query: SelectionQuery {
                    clauses: vec![SelectionClause {
                        combine: SelectionCombine::Add,
                        term: SelectionTerm::Named { text: system.name.to_string() },
                    }],
                    order: SelectionOrder::ByName,
                },
            },
        )
        .await?;
    }

    let rig = into.fixtures().await;
    let system_of = |name: &str| -> Vec<&pult_schema::types::Fixture> {
        rig.iter().filter(|fixture| fixture.name.starts_with(name)).collect()
    };

    // Twenty cues, each touching a few systems rather than the whole rig — which is
    // what a stack actually looks like, and is why a console does not have to
    // recompute every fixture on every Go.
    //
    // The split times are the point: a state that comes up over five seconds while
    // the last one goes out over two is the ordinary shape of a theatre cue.
    let states: [(&str, &[(&str, f32)], u32, u32); 10] = [
        ("Preset", &[("Cyc", 0.3)], 3_000, 3_000),
        ("House to half", &[("Front", 0.5), ("Cyc", 0.3)], 2_000, 2_000),
        ("Act 1 open", &[("Front", 0.9), ("Back", 0.7), ("Cyc", 0.6)], 5_000, 2_000),
        ("Downstage special", &[("Front", 1.0), ("Back", 0.2)], 2_500, 4_000),
        ("Storm", &[("Front", 0.3), ("Side", 1.0), ("Cyc", 0.15)], 1_000, 6_000),
        ("Interior night", &[("Front", 0.4), ("Side", 0.6), ("Cyc", 0.0)], 4_000, 4_000),
        ("Dawn", &[("Front", 0.6), ("Back", 0.9), ("Cyc", 1.0)], 8_000, 3_000),
        ("Act 2 open", &[("Front", 0.9), ("Back", 0.8), ("Side", 0.5)], 4_000, 2_000),
        ("Curtain", &[("Front", 1.0), ("Back", 1.0), ("Side", 1.0), ("Cyc", 1.0)], 2_000, 2_000),
        ("Blackout", &[], 0, 3_000),
    ];

    let mut cues = Vec::new();
    for (index, (name, levels, fade_in, fade_out)) in states.iter().enumerate() {
        let mut captures = Vec::new();
        for (system, level) in levels.iter() {
            for fixture in system_of(system) {
                captures.push(capture(
                    fixture.id,
                    ParameterKind::Intensity,
                    ParameterValue::Float(*level),
                ));
                // The cyc mixes, so its colour is part of the state rather than
                // something somebody gelled in the morning.
                if *system == "Cyc" {
                    captures.push(capture(
                        fixture.id,
                        ParameterKind::ColorRgb,
                        cyc_colour(index),
                    ));
                }
            }
        }
        // A blackout says so about everything, or the systems it does not mention
        // would simply stay where the last cue left them.
        if levels.is_empty() {
            for fixture in &rig {
                captures.push(capture(
                    fixture.id,
                    ParameterKind::Intensity,
                    ParameterValue::Float(0.0),
                ));
            }
        }
        let mut cue = a_cue(name, index as f64 + 1.0, captures);
        cue.fade_in_ms = *fade_in;
        cue.fade_out_ms = *fade_out;
        cues.push(cue);
    }
    for cue in &cues {
        into.create("cues", cue).await?;
    }

    into.create(
        "sequences",
        &Sequence {
            id: id(),
            name: "Main".to_string(),
            cue_ids: cues.iter().map(|cue| cue.id).collect(),
            active_cue_index: None,
            went_at: None,
        },
    )
    .await?;

    Ok(())
}

/// A colour for the cloth, walking through the evening.
fn cyc_colour(index: usize) -> ParameterValue {
    const SKY: &[(f32, f32, f32)] = &[
        (0.1, 0.2, 0.6),
        (0.2, 0.3, 0.7),
        (0.4, 0.5, 0.9),
        (0.6, 0.4, 0.3),
        (0.2, 0.1, 0.3),
        (0.05, 0.05, 0.2),
        (0.9, 0.6, 0.3),
        (0.5, 0.7, 1.0),
        (1.0, 0.9, 0.8),
        (0.0, 0.0, 0.0),
    ];
    let (r, g, b) = SKY[index % SKY.len()];
    ParameterValue::rgb(r, g, b)
}
