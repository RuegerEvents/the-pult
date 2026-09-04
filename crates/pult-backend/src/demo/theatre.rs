//! Conventionals and a cue stack: what most of the world actually runs.
//!
//! No movers and no effects, deliberately. The interesting things here are the ones
//! a rig of moving heads hides: systems of identical lanterns addressed and grouped
//! by where they point rather than by what they are, a stack of cues that mostly
//! changes a few of them at a time, and **split fade times** — the front coming up
//! over five seconds while the previous state goes out over two is the ordinary
//! shape of a theatre cue and is invisible in a show that never sets one.
//!
//! Two universes, because forty lanterns is more than one holds once the cyc is in —
//! which is also what makes the Patch panel's universe view worth looking at.

use anyhow::Result;
use pult_schema::types::{
    fixture::{Fixture, FixtureType, ParameterKind, Vec3},
    mount::Mount,
    group::{Group, SelectionClause, SelectionCombine, SelectionOrder, SelectionQuery,
            SelectionTerm},
    scene::Transform,
};

use super::{
    id,
    kit::{
        a_clamped_fixture, a_cue, a_piece, a_stack, a_type, boom, capture, colour, facing,
        intensity, level, on, sky, truss_run, under, Addresses,
    },
    Seeder,
};

/// Where a system hangs: under a bar, or up a boom.
enum Hang {
    /// An index into the bars below.
    Bar(usize),
    /// An index into the booms below.
    Boom(usize),
}

/// One lighting system: what it is called, what type it is made of, how many, and
/// where they hang.
struct System {
    name: &'static str,
    /// Index into the types below.
    kind: usize,
    count: u16,
    hang: Hang,
    /// The direction the whole system points.
    aim: Vec3,
}

pub async fn seed(into: &Seeder) -> Result<()> {
    into.name_the_show("Theatre").await?;

    // Three conventionals: a profile to shape, a fresnel to wash, and an LED batten
    // for the cloth. Only the batten mixes, which is why it is the one with a colour
    // parameter — everything else is a lantern with gel in it, and a console that
    // offered a colour picker for one would be lying.
    let profile = a_type("Profile 26°", vec![intensity()]);
    let fresnel = a_type("Fresnel 1kW", vec![intensity()]);
    let cyc = a_type("LED Cyc Batten", vec![intensity(), colour()]);
    let types: Vec<&FixtureType> = vec![&profile, &fresnel, &cyc];
    for kind in &types {
        into.create("fixture_types", *kind).await?;
    }

    // The room. A twelve-metre opening, three bars over it, a batten for the cloth,
    // and a boom each side — three metres of truss stood on its end with its foot on
    // the deck, which is what a boom is and what the first version of this demo drew
    // as a fourth and fifth horizontal bar.
    // Each bar with its length beside it, because a system is spread along the bar
    // it hangs on and not along some other one: the first version spread the cyc
    // batten's eight cells over ten metres of a nine-metre bar, and the outer ones
    // hung in the air past its ends.
    let bars = [
        (truss_run(into, "FOH bar", None, Vec3 { x: 0.0, y: 6.0, z: 5.0 }, 12.0).await?, 12.0),
        (truss_run(into, "LX 2", None, Vec3 { x: 0.0, y: 6.5, z: -1.0 }, 12.0).await?, 12.0),
        (truss_run(into, "LX 4", None, Vec3 { x: 0.0, y: 6.5, z: -4.0 }, 12.0).await?, 12.0),
        (truss_run(into, "Cyc batten", None, Vec3 { x: 0.0, y: 5.0, z: -6.0 }, 9.0).await?, 9.0),
    ];
    let booms = [
        boom(into, "Boom SL", None, Vec3 { x: -7.0, y: 1.5, z: 0.0 }, 3.0).await?,
        boom(into, "Boom SR", None, Vec3 { x: 7.0, y: 1.5, z: 0.0 }, 3.0).await?,
    ];

    // And something for the light to land on: a low rostrum upstage centre, and a
    // cloth to wash — three flats standing side by side across the back.
    a_piece(into, "Rostrum", "deck-2x1", super::kit::at(0.0, 0.4, -3.0), None).await?;
    for (n, x) in [-3.0f32, 0.0, 3.0].into_iter().enumerate() {
        a_piece(
            into,
            &format!("Cloth {}", n + 1),
            "wall-2x1",
            Transform {
                position: Vec3 { x, y: 0.0, z: -7.0 },
                // Three metres of cloth out of a two-metre panel, and three high.
                scale: Vec3 { x: 1.5, y: 3.0, z: 1.0 },
                ..Transform::default()
            },
            None,
        )
        .await?;
    }

    let systems = [
        System { name: "Front", kind: 0, count: 12, hang: Hang::Bar(0), aim: facing::DOWNSTAGE },
        System { name: "Back", kind: 0, count: 10, hang: Hang::Bar(2), aim: facing::UPSTAGE },
        System { name: "Side SL", kind: 1, count: 4, hang: Hang::Boom(0), aim: facing::FROM_LEFT },
        System { name: "Side SR", kind: 1, count: 4, hang: Hang::Boom(1), aim: facing::FROM_RIGHT },
        System { name: "Cyc", kind: 2, count: 8, hang: Hang::Bar(3), aim: facing::AT_THE_CLOTH },
    ];

    let mut addresses = Addresses::from(1);
    for system in &systems {
        let kind = types[system.kind];
        for n in 0..system.count {
            // Spread evenly along whatever they hang on, so the rig view shows a
            // system rather than a heap.
            let spread = |span: f32| {
                if system.count > 1 {
                    -span / 2.0 + span * (n as f32 / (system.count - 1) as f32)
                } else {
                    0.0
                }
            };
            let (parent, mount, position) = match system.hang {
                // Under the bar, on a clamp, spread over all but the last half metre
                // at each end of it.
                Hang::Bar(bar) => {
                    let (id, metres) = bars[bar];
                    let mount = Mount::along(spread(metres - 1.0));
                    (id, mount, under(mount, system.aim))
                }
                // Up the boom, clamped to the chord that faces centre stage rather
                // than drawn inside it. A boom is a run stood on its end, so `along`
                // is the height up it; the aim is a world term and `on` puts that in
                // the boom's own turned frame.
                Hang::Boom(index) => {
                    let (id, handle) = &booms[index];
                    // A boom is a run turned a quarter about Z, so its local −Y is
                    // the world's +X. The clamp goes on the chord facing centre
                    // stage and the roll pushes the body out along it — which is
                    // what keeps the lantern on the face of the boom instead of
                    // inside it, the thing the first Theatre demo got wrong twice.
                    let (chord, roll) = if handle.position.x < 0.0 { (0, 0.0) } else { (2, 180.0) };
                    let mount = Mount { chord, along: spread(2.2), roll };
                    (*id, mount, on(handle, mount, system.aim))
                }
            };

            let fixture = a_clamped_fixture(
                &format!("{} {}", system.name, n + 1),
                kind.id,
                addresses.take(kind.channel_count),
                parent,
                position,
                mount,
            );
            into.create("fixtures", &fixture).await?;
        }
    }

    // A group per system, as a *question* rather than a list: "everything whose name
    // has Front in it" stays true after somebody hangs a thirteenth.
    for name in ["Front", "Back", "Side", "Cyc"] {
        into.create(
            "groups",
            &Group {
                id: id(),
                name: name.to_string(),
                query: SelectionQuery {
                    clauses: vec![SelectionClause {
                        combine: SelectionCombine::Add,
                        term: SelectionTerm::Named { text: name.to_string() },
                    }],
                    order: SelectionOrder::ByName,
                },
            },
        )
        .await?;
    }

    let rig = into.fixtures().await;
    let system_of = |name: &str| -> Vec<&Fixture> {
        rig.iter().filter(|fixture| fixture.name.starts_with(name)).collect()
    };

    // Twenty cues, each touching a few systems rather than the whole rig — which is
    // what a stack actually looks like, and is why a console does not have to
    // recompute every fixture on every Go.
    //
    // The split times are the point: a state that comes up over five seconds while
    // the last one goes out over two is the ordinary shape of a theatre cue.
    type State = (&'static str, &'static [(&'static str, f32)], u32, u32);
    let states: [State; 20] = [
        ("Preset", &[("Cyc", 0.30)], 3_000, 3_000),
        ("House to half", &[("Front", 0.50), ("Cyc", 0.30)], 2_000, 2_000),
        ("Act 1 open", &[("Front", 0.90), ("Back", 0.70), ("Cyc", 0.60)], 5_000, 2_000),
        ("Enter SL", &[("Front", 0.90), ("Side SL", 0.80), ("Back", 0.50)], 3_000, 2_500),
        ("Downstage special", &[("Front", 1.00), ("Back", 0.20)], 2_500, 4_000),
        ("Two-hander", &[("Front", 0.75), ("Side SL", 0.60), ("Side SR", 0.60)], 4_000, 3_000),
        ("Storm builds", &[("Front", 0.30), ("Side SL", 1.00), ("Cyc", 0.15)], 1_000, 6_000),
        ("Thunder", &[("Front", 0.10), ("Back", 1.00), ("Cyc", 0.05)], 250, 250),
        ("After the storm", &[("Front", 0.45), ("Cyc", 0.20)], 6_000, 1_000),
        ("Interior night", &[("Front", 0.40), ("Side SR", 0.60), ("Cyc", 0.00)], 4_000, 4_000),
        ("Candle out", &[("Front", 0.15)], 9_000, 2_000),
        ("Act 1 close", &[], 4_000, 4_000),
        ("Act 2 preset", &[("Cyc", 0.40)], 3_000, 3_000),
        ("Dawn", &[("Front", 0.60), ("Back", 0.90), ("Cyc", 1.00)], 8_000, 3_000),
        ("Act 2 open", &[("Front", 0.90), ("Back", 0.80), ("Side SL", 0.50)], 4_000, 2_000),
        ("The letter", &[("Front", 1.00), ("Side SR", 0.30)], 2_000, 5_000),
        ("Exit SR", &[("Front", 0.60), ("Side SR", 0.90), ("Back", 0.40)], 3_000, 2_000),
        ("Last light", &[("Back", 0.80), ("Cyc", 0.70)], 7_000, 5_000),
        ("Curtain", &[("Front", 1.00), ("Back", 1.00), ("Side SL", 1.00), ("Side SR", 1.00),
                      ("Cyc", 1.00)], 2_000, 2_000),
        ("Blackout", &[], 0, 3_000),
    ];

    let mut cues = Vec::new();
    for (index, (name, levels, fade_in, fade_out)) in states.iter().enumerate() {
        let mut captures = Vec::new();
        for (system, at) in levels.iter() {
            for fixture in system_of(system) {
                captures.push(level(fixture.id, *at));
                // The cyc mixes, so its colour is part of the state rather than
                // something somebody gelled in the morning.
                if *system == "Cyc" {
                    captures.push(capture(fixture.id, ParameterKind::ColorRgb, sky(index)));
                }
            }
        }
        // A cue that names nothing says so about everything, or the systems it does
        // not mention would simply stay where the last cue left them.
        if levels.is_empty() {
            captures.extend(rig.iter().map(|fixture| level(fixture.id, 0.0)));
        }
        let mut cue = a_cue(name, index as f64 + 1.0, captures);
        cue.fade_in_ms = *fade_in;
        cue.fade_out_ms = *fade_out;
        cues.push(cue);
    }

    a_stack(into, "Main", cues, false).await?;
    Ok(())
}
