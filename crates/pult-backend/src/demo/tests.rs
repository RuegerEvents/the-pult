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
        .filter(|object| object.catalogue.is_some())
        .count();
    assert!(drawable > 0, "{}: nothing in the room can be drawn", demo.id());
    for object in &objects {
        if let Some(parent) = object.parent {
            assert!(places.contains(&parent), "{}: {} hangs off nothing", demo.id(), object.name);
        }
        if let Some(id) = &object.catalogue {
            assert!(
                pult_schema::types::piece(id).is_some(),
                "{}: {} names {id}, which is not in the catalogue",
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
        if let Some(parent) = fixture.parent {
            let bar = by_id[&parent];
            // A run of truss lies along its own X whichever way it was turned, so
            // the distance from its centre line is the other two.
            let off_the_bar = position.position.y.hypot(position.position.z);
            assert!(
                off_the_bar >= 0.3,
                "{}: {} is {off_the_bar:.2} m from the centre of {}, which is inside it",
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

#[tokio::test]
async fn haunt_seeds_a_rig_that_hangs_together() {
    seeds_a_rig_that_hangs_together(Demo::Haunt).await;
}

#[tokio::test]
async fn theatre_seeds_a_rig_that_hangs_together() {
    seeds_a_rig_that_hangs_together(Demo::Theatre).await;
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
