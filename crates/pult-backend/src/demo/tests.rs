//! Every demo seeds into a fresh station, and none of them is a rig with holes in it.
//!
//! What these check is what a broken demo would actually look like from a console:
//! fixtures patched to a type that is not there, a cue capturing a fixture nobody
//! hung, a sequence naming a cue that does not exist, an effect on a master that was
//! never made. Every one of those is a show that opens and then quietly does the
//! wrong thing, which is the failure a demo must not have — it is the first thing
//! anybody sees.

use std::collections::HashSet;

use pult_schema::{
    path::PathSegment,
    types::{scene::SceneObject, Cue, Fixture, FixtureType, Sequence, SpeedMaster},
};

use super::*;
use crate::engine::{EngineHandle, ShowEngine};

/// A station with an empty show and nothing else.
async fn a_station() -> EngineHandle {
    let pool = std::sync::Arc::new(crate::infra::showfile::open_in_memory().await.unwrap());
    let (engine, handle, _broadcast) = ShowEngine::new(
        pult_schema::events::operation::NodeId::new(),
        pool,
        None,
    );
    tokio::spawn(engine.run());
    handle
}

async fn read<T: serde::de::DeserializeOwned>(engine: &EngineHandle, table: &str) -> Vec<T> {
    engine
        .get(vec![PathSegment::Key(table.into())])
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Seed one demo and check that what it wrote hangs together.
async fn seeds_a_rig_that_hangs_together(demo: Demo) {
    let engine = a_station().await;
    seed(&engine, demo).await.expect("it seeds");

    let types: Vec<FixtureType> = read(&engine, "fixture_types").await;
    let fixtures: Vec<Fixture> = read(&engine, "fixtures").await;
    let cues: Vec<Cue> = read(&engine, "cues").await;
    let sequences: Vec<Sequence> = read(&engine, "sequences").await;
    let masters: Vec<SpeedMaster> = read(&engine, "speed_masters").await;

    assert!(!fixtures.is_empty(), "{} patched nothing", demo.id());
    assert!(!cues.is_empty(), "{} has nothing to Go", demo.id());
    assert!(!sequences.is_empty(), "{} has no stack", demo.id());

    let known_types: HashSet<_> = types.iter().map(|kind| kind.id).collect();
    for fixture in &fixtures {
        assert!(
            known_types.contains(&fixture.fixture_type_id),
            "{}: {} is patched to a type nothing made",
            demo.id(),
            fixture.name,
        );
    }

    let hung: HashSet<_> = fixtures.iter().map(|fixture| fixture.id).collect();
    let known_masters: HashSet<_> = masters.iter().map(|master| master.id).collect();
    for cue in &cues {
        for capture in &cue.captures {
            assert!(
                hung.contains(&capture.fixture_id),
                "{}: {} captures a fixture nobody hung",
                demo.id(),
                cue.name,
            );
            if let Some(effect) = &capture.effect {
                if let pult_schema::types::effect::Rate::Master { id, .. } = effect.rate {
                    assert!(
                        known_masters.contains(&id),
                        "{}: {} runs on a speed master nothing made",
                        demo.id(),
                        cue.name,
                    );
                }
            }
        }
    }

    // Nothing floats. Every fixture hangs off something, and the thing it hangs off
    // is either drawable or a handle whose children are — which is the difference
    // between a rig view and a field of lights in the dark.
    let objects: Vec<SceneObject> = read(&engine, "scene_objects").await;
    let places: HashSet<_> = objects.iter().map(|object| object.id).collect();
    assert!(!objects.is_empty(), "{} hangs its rig off nothing", demo.id());
    let drawable = objects
        .iter()
        .filter(|object| object.catalog.is_some())
        .count();
    assert!(drawable > 0, "{}: nothing in the room can be drawn", demo.id());
    for object in &objects {
        if let Some(parent) = object.parent {
            assert!(places.contains(&parent), "{}: {} hangs off nothing", demo.id(), object.name);
        }
        if let Some(id) = &object.catalog {
            assert!(
                pult_schema::types::piece(id).is_some(),
                "{}: {} names {id}, which is not in the catalog",
                demo.id(),
                object.name,
            );
        }
    }

    // And every light that hangs points *down*, in the world — through whatever it
    // hangs off, since a lantern on a boom is written in the boom's own turned frame
    // and only the composed direction says where it looks. The regression: a rotation
    // written by hand meaning "hanging" was a quarter turn away from it, and three of
    // the four demos aimed their whole rig at the back wall.
    //
    // A light *standing* on the floor or the deck is the exception, since a floor
    // package looking up over the crowd is what a floor package is for.
    let by_id = pult_schema::types::scene::by_id(&objects);
    for fixture in &fixtures {
        let Some(position) = &fixture.position else {
            panic!("{}: {} was never placed", demo.id(), fixture.name);
        };
        if let Some(parent) = fixture.parent {
            assert!(places.contains(&parent), "{}: {} hangs off nothing", demo.id(), fixture.name);
        }
        let world = pult_schema::types::scene::world_transform(position, fixture.parent, &by_id);
        let facing = world.facing_direction();
        let hung = world.position.y > 2.0;
        assert!(
            !hung || facing.y < 0.0,
            "{}: {} hangs at {:.1} m and points upwards ({facing:?})",
            demo.id(),
            fixture.name,
            world.position.y,
        );
        // And nothing hangs *inside* the bar it is clamped to: a fixture whose body
        // overlaps the chords is what "next to the truss" looked like in the Club.
        // Only what is *clamped*: a par standing in a tank and a pixel on a strip hang
        // off something they are meant to be inside.
        if let Some(parent) = fixture.parent.filter(|_| fixture.mount.is_some()) {
            let bar = by_id[&parent];
            // A run of truss lies along its own X whichever way it was turned, so
            // the distance from its center line is the other two.
            let off_the_bar = position.position.y.hypot(position.position.z);
            assert!(
                off_the_bar >= 0.3,
                "{}: {} is {off_the_bar:.2} m from the center of {}, which is inside it",
                demo.id(),
                fixture.name,
                bar.name,
            );
        }
    }

    let known_cues: HashSet<_> = cues.iter().map(|cue| cue.id).collect();
    for sequence in &sequences {
        assert!(!sequence.cue_ids.is_empty(), "{}: {} is empty", demo.id(), sequence.name);
        // A stack of one is a look, not a stack. Every demo should have something to
        // press Go on more than once.
        assert!(
            sequences.iter().map(|s| s.cue_ids.len()).sum::<usize>() >= 3,
            "{} has almost nothing to run",
            demo.id(),
        );
        for cue in &sequence.cue_ids {
            assert!(
                known_cues.contains(cue),
                "{}: {} names a cue that is not there",
                demo.id(),
                sequence.name,
            );
        }
    }
}

/// A preset names fixtures, and a preset naming one the show has not got is a button
/// that quietly does less than it looks like it will — which is exactly what nobody
/// notices until the show. Also that at least one cue in the demos that carry
/// palettes actually *references* one, because a preset nothing points at
/// demonstrates nothing.
#[tokio::test]
async fn every_preset_names_only_fixtures_that_exist() {
    for demo in ALL.iter().copied() {
        let engine = a_station().await;
        seed(&engine, demo).await.expect("it seeds");
        let fixtures: Vec<Fixture> = read(&engine, "fixtures").await;
        let presets: Vec<pult_schema::types::Preset> = read(&engine, "presets").await;
        let cues: Vec<Cue> = read(&engine, "cues").await;
        let known: HashSet<_> = fixtures.iter().map(|f| f.id).collect();

        for preset in &presets {
            assert!(!preset.values.is_empty(), "{}: {} says nothing", demo.id(), preset.name);
            for value in &preset.values {
                assert!(
                    known.contains(&value.fixture_id),
                    "{}: preset {} names a fixture that is not patched",
                    demo.id(),
                    preset.name,
                );
            }
        }

        if presets.is_empty() {
            continue;
        }
        let ids: HashSet<_> = presets.iter().map(|p| p.id).collect();
        let referenced = cues
            .iter()
            .flat_map(|cue| &cue.captures)
            .filter_map(|capture| capture.preset)
            .collect::<HashSet<_>>();
        assert!(
            !referenced.is_empty(),
            "{}: it has palettes and no cue points at one",
            demo.id()
        );
        for id in referenced {
            assert!(ids.contains(&id), "{}: a cue references a preset that is gone", demo.id());
        }
    }
}

/// The demo, end to end: take the cue that references a palette, edit the palette, and
/// the cyc moves. This is the by-hand gate in the task entry, as a test.
#[tokio::test]
async fn editing_a_demo_preset_moves_the_cue_that_is_standing() {
    let engine = a_station().await;
    seed(&engine, Demo::Theater).await.expect("it seeds");

    let cues: Vec<Cue> = read(&engine, "cues").await;
    let sequences: Vec<Sequence> = read(&engine, "sequences").await;
    let presets: Vec<pult_schema::types::Preset> = read(&engine, "presets").await;
    let dawn = cues.iter().find(|c| c.name == "Dawn").expect("the Dawn cue");
    let sky = presets.iter().find(|p| p.name == "Dawn sky").expect("the Dawn sky preset");
    let sequence = sequences.iter().find(|s| s.cue_ids.contains(&dawn.id)).expect("its sequence");

    engine
        .set(
            vec![
                pult_schema::path::PathSegment::Key("sequences".into()),
                pult_schema::path::PathSegment::Id(sequence.id),
                pult_schema::path::PathSegment::Key("goToCue".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Synced,
            serde_json::json!({ "cueId": dawn.id }),
        )
        .await
        .expect("it takes");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let cyc = read::<Fixture>(&engine, "fixtures")
        .await
        .into_iter()
        .find(|f| f.name.starts_with("Cyc"))
        .expect("a cyc batten");
    let to_of = |f: &Fixture| f.live_fades.get("ColorRgb").map(|fade| fade.to.clone());
    let before = to_of(&cyc).expect("the cue is driving its color");

    // Somebody edits the palette. Nothing else about the show changes.
    let mut values = sky.values.clone();
    for value in &mut values {
        value.value = pult_schema::types::ParameterValue::rgb(0.1, 0.3, 1.0);
    }
    engine
        .set(
            vec![
                pult_schema::path::PathSegment::Key("presets".into()),
                pult_schema::path::PathSegment::Id(sky.id),
                pult_schema::path::PathSegment::Key("values".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Persisted,
            serde_json::to_value(&values).unwrap(),
        )
        .await
        .expect("the palette is written");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let after = read::<Fixture>(&engine, "fixtures")
        .await
        .into_iter()
        .find(|f| f.id == cyc.id)
        .and_then(|f| to_of(&f))
        .expect("still driven");
    assert_ne!(before, after, "the standing cue did not follow the palette");
    assert_eq!(after, pult_schema::types::ParameterValue::rgb(0.1, 0.3, 1.0));
}

#[tokio::test]
async fn haunt_seeds_a_rig_that_hangs_together() {
    seeds_a_rig_that_hangs_together(Demo::Haunt).await;
}

#[tokio::test]
async fn theater_seeds_a_rig_that_hangs_together() {
    seeds_a_rig_that_hangs_together(Demo::Theater).await;
}

#[tokio::test]
async fn club_seeds_a_rig_that_hangs_together() {
    seeds_a_rig_that_hangs_together(Demo::Club).await;
}

#[tokio::test]
async fn festival_seeds_a_rig_that_hangs_together() {
    seeds_a_rig_that_hangs_together(Demo::Festival).await;
}

#[tokio::test]
async fn garden_seeds_a_rig_that_hangs_together() {
    seeds_a_rig_that_hangs_together(Demo::Garden).await;
}

#[tokio::test]
async fn the_demos_name_no_curve_and_so_their_heads_ease_into_their_marks() {
    // The test of whether the show's default is in the right place. Every demo's
    // position cues run on the shape the *show* says, so a desk that turns the
    // setting off sees all four rigs change — which a demo that wrote `Linear` into
    // its captures could never show.
    for demo in ALL.iter().copied() {
        let engine = a_station().await;
        seed(&engine, demo).await.unwrap();
        let cues: Vec<Cue> = read(&engine, "cues").await;
        let named: Vec<_> = cues
            .iter()
            .flat_map(|cue| cue.captures.iter().map(move |c| (cue, c)))
            .filter(|(_, c)| c.easing.is_some())
            .map(|(cue, c)| format!("{} {:?}", cue.name, c.parameter_kind))
            .collect();
        assert!(named.is_empty(), "{} pins a curve: {named:?}", demo.id());
        assert!(cues.iter().all(|cue| cue.easing.is_none()), "{}: nor does a cue", demo.id());
    }
}

#[tokio::test]
async fn a_show_that_already_has_a_rig_is_left_alone() {
    // `--demo` survives a restart, so this is the difference between a console that
    // opens the demo it was told to and one that patches a second rig on top of
    // whatever somebody has done since.
    let engine = a_station().await;
    seed(&engine, Demo::Haunt).await.unwrap();
    let first: Vec<Fixture> = read(&engine, "fixtures").await;

    seed(&engine, Demo::Festival).await.unwrap();

    let second: Vec<Fixture> = read(&engine, "fixtures").await;
    assert_eq!(first.len(), second.len(), "the second seed changed nothing");
}

#[tokio::test]
async fn the_club_and_the_festival_come_up_running() {
    // The engine has no clock: a show with nothing running is a station doing
    // nothing at all, which is the wrong thing to hand somebody who opened the demo
    // to find out whether the console works. So something has to be *moving*: a
    // running sequence whose live cue carries an effect. Not everything — the
    // Festival deliberately leaves its blinders and its floor package waiting for
    // Go, because a rig where everything is asserted at once is a rig where nothing
    // reads.
    for demo in [Demo::Club, Demo::Festival] {
        let engine = a_station().await;
        seed(&engine, demo).await.unwrap();
        let sequences: Vec<Sequence> = read(&engine, "sequences").await;
        let cues: Vec<Cue> = read(&engine, "cues").await;
        let moving = sequences.iter().any(|s| {
            let Some(index) = s.active_cue_index else { return false };
            s.went_at.is_some()
                && cues
                    .iter()
                    .find(|cue| Some(&cue.id) == s.cue_ids.get(index))
                    .is_some_and(|cue| cue.captures.iter().any(|c| c.effect.is_some()))
        });
        assert!(moving, "{} came up with nothing moving: {sequences:?}", demo.id());
    }
}

#[tokio::test]
async fn the_festival_leaves_something_for_go_to_do() {
    let engine = a_station().await;
    seed(&engine, Demo::Festival).await.unwrap();
    let sequences: Vec<Sequence> = read(&engine, "sequences").await;
    let waiting = sequences.iter().filter(|s| s.active_cue_index.is_none()).count();
    assert!(waiting >= 2, "everything in the festival is on at once: {sequences:?}");
    // And every playback ends in a way off, so an operator can kill a system.
    let cues: Vec<Cue> = read(&engine, "cues").await;
    for sequence in &sequences {
        let last = sequence.cue_ids.last().and_then(|id| cues.iter().find(|c| &c.id == id));
        assert!(
            last.is_some_and(|cue| cue.name == "Out"),
            "{} does not end in Out",
            sequence.name
        );
    }
}

#[test]
fn every_demo_can_be_spelled_and_read_back() {
    // The ids are what `--demo` takes and what the welcome screen's cards carry, so
    // the two cannot be allowed to drift.
    for demo in ALL {
        assert_eq!(Demo::parse(demo.id()), Some(*demo));
        assert!(!demo.title().is_empty());
        assert!(!demo.blurb().is_empty());
    }
    assert_eq!(Demo::parse("HAUNT"), Some(Demo::Haunt), "case is not a spelling mistake");
    assert_eq!(Demo::parse("nonesuch"), None);
}

// ── The Garden ────────────────────────────────────────────────────────────────

/// What the Garden says a V0 is, against what the simulator — the firmware's side of
/// the contract, in software — says it is. A drift here is ten demo nodes whose types
/// a real node would rebuild the moment it came up.
#[test]
fn the_garden_describes_a_v0_as_the_simulator_does() {
    use openhaunt_node_sim::{pixels::StripConfig, ModuleKind};
    let (ours, pixels) = garden::v0_description();
    let sim_ports: Vec<serde_json::Value> = ModuleKind::NodeV0
        .ports()
        .into_iter()
        .chain(StripConfig::default_for(0).ports())
        .chain(StripConfig::default_for(1).ports())
        .map(|port| serde_json::to_value(port).unwrap())
        .collect();
    let theirs: Vec<pult_schema::types::openhaunt::PortDescription> =
        serde_json::from_value(serde_json::Value::Array(sim_ports)).unwrap();
    assert_eq!(ours.ports, theirs);
    assert_eq!(ModuleKind::NodeV0.type_id(), garden::V0_MODULE);
    assert_eq!(pixels.effects, openhaunt_node_sim::pixels::EFFECTS.map(str::to_string).to_vec());
}

/// A station with an empty show and somewhere to put what the Garden generates.
async fn a_station_with_a_store() -> EngineHandle {
    let pool = std::sync::Arc::new(crate::infra::showfile::open_in_memory().await.unwrap());
    let station = pult_schema::events::operation::NodeId::new();
    let (engine, handle, _broadcast) = ShowEngine::new(station, pool.clone(), None);
    tokio::spawn(engine.run());
    let dir = std::env::temp_dir().join(format!("pult-garden-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let assets = crate::infra::assets::AssetStore::new(Some(dir), pool);
    seed_with(&handle, Some(assets), Some(station), Demo::Garden).await.expect("the garden seeds");
    handle
}

async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if check().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("never: {what}");
}

/// The Garden's maze, run by nothing but its sensors: groups walk in and out of the
/// first two areas, the scare goes off for the first group into an empty area and not
/// for the one behind it, and the traffic light follows the second area. No hardware,
/// no operator — sensed values in, and what the show's own flows make of them.
#[tokio::test]
async fn the_garden_maze_runs_on_its_sensors() {
    use pult_schema::types::{timeline::Timeline, Variable};

    let engine = a_station_with_a_store().await;
    let fixtures: Vec<Fixture> = read(&engine, "fixtures").await;
    let node = |name: &str| fixtures.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name}")).id;
    let (area1, area2, entrance) = (node("Area 1 node"), node("Area 2 node"), node("Entrance node"));

    let timelines: Vec<Timeline> = read(&engine, "timelines").await;
    let scare1 = timelines.iter().find(|t| t.name == "Area 1 scare").unwrap().id;
    assert!(
        timelines.iter().filter(|t| t.name.ends_with("scare")).all(|t| !t.clips.is_empty()),
        "every scare has its sound"
    );
    assert_eq!(
        timelines.iter().filter(|t| !t.tracks.is_empty()).count(),
        2,
        "both skeletons have a take"
    );

    let edge = |fixture: Uuid, port: u8, closed: bool| {
        let engine = engine.clone();
        async move {
            engine
                .set_sensed_value(
                    fixture,
                    format!("Contact:{port}"),
                    serde_json::json!({ "type": "Bool", "value": closed }),
                )
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
    };
    let walk = |fixture: Uuid, port: u8| {
        let edge = edge.clone();
        async move {
            edge(fixture, port, true).await;
            edge(fixture, port, false).await;
        }
    };
    let inside = |name: &'static str| {
        let engine = engine.clone();
        async move {
            let vars: Vec<Variable> = read(&engine, "variables").await;
            vars.iter().find(|v| v.name == name).unwrap().current()
        }
    };
    let running = |id: Uuid| {
        let engine = engine.clone();
        async move {
            let timelines: Vec<Timeline> = read(&engine, "timelines").await;
            timelines.iter().find(|t| t.id == id).unwrap().running
        }
    };
    let lamp = |relay: u8| {
        let engine = engine.clone();
        async move {
            let fixtures: Vec<Fixture> = read(&engine, "fixtures").await;
            let entrance = fixtures.iter().find(|f| f.id == entrance).unwrap();
            entrance
                .live_fades
                .get(&format!("Switch:{relay}"))
                .map(|fade| fade.to == pult_schema::types::ParameterValue::Bool(true))
        }
    };

    // The flows meet the sensors once before anything happens.
    edge(area1, 4, false).await;
    edge(area2, 4, false).await;

    // The first group into area 1.
    walk(area1, 4).await;
    eventually("area 1 counts them in", || async { inside("Area 1 inside").await == 1.0 }).await;
    eventually("and scares them", || async { running(scare1).await }).await;
    // The scare finishes, and the next group walks in behind the first.
    engine
        .set(
            vec![PathSegment::Key("timelines".into()), PathSegment::Id(scare1), PathSegment::Key("stop".into())],
            pult_schema::lifecycle::Lifecycle::Synced,
            serde_json::json!({}),
        )
        .await
        .unwrap();
    walk(area1, 4).await;
    eventually("two groups inside", || async { inside("Area 1 inside").await == 2.0 }).await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(!running(scare1).await, "an occupied area does not scare again");
    walk(area1, 5).await;
    walk(area1, 5).await;
    eventually("both groups out", || async { inside("Area 1 inside").await == 0.0 }).await;

    // Area 2, and the light at the entrance.
    walk(area2, 4).await;
    eventually("red while somebody is in area 2", || async {
        lamp(0).await == Some(true) && lamp(1).await == Some(false)
    })
    .await;
    walk(area2, 5).await;
    eventually("green when area 2 is free", || async {
        lamp(1).await == Some(true) && lamp(0).await == Some(false)
    })
    .await;
}
