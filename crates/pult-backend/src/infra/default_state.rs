//! The show as it comes up: which sequences are on and at which cue, their masters, and
//! which timelines run. Saved from the show as it stands, applied by hand, and applied
//! by the leader when a show opens with nothing running.
//!
//! **When it is applied by itself is the whole design.** SYNCED state — a cue up, a
//! timeline running — does not survive a restart, so a console that comes up with the
//! house lights off and the ambience silent is a console somebody has to drive before
//! the doors open. But a station coming back into a session that is already running must
//! not reset the show the others are playing. So only a station with no leader of its own
//! to go back to applies it (`remembered::load` is empty: the station that leads), only
//! once it leads, only after [`GRACE`] — long enough for a stand-in to hand the live show
//! back — and only if, by then, nothing is running at all.
//!
//! Saving and clearing are field writes, so each is one Ctrl-Z. Applying is Gos, Offs
//! and plays, which nothing in this console takes back, as a Go pressed by hand is not.

use std::time::Duration;

use pult_schema::{
    events::operation::NodeId,
    path::{Path, PathSegment},
    registry::path_lifecycle,
    types::{
        sequence::{now_ms, Sequence},
        show::{DefaultSequence, DefaultState},
        timeline::Timeline,
        user::User,
    },
};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{engine::EngineHandle, infra::net::NetHandle};

/// How long a station that opened a show waits before deciding it is the one to bring
/// it up: past the ten seconds in which a stand-in hands the lead back.
pub const GRACE: Duration = Duration::from_secs(20);

/// What was saved, for a toast.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub sequences: usize,
    pub timelines: usize,
}

/// The show as it stands, as its default state: every sequence that is on or not at
/// full, and every running timeline. Written onto the show under the caller's name.
pub async fn save(engine: &EngineHandle, user: Uuid) -> Result<Saved, String> {
    let sequences: Vec<Sequence> = read(engine, "sequences").await;
    let timelines: Vec<Timeline> = read(engine, "timelines").await;
    let state = of(&sequences, &timelines, now_ms());
    let saved = Saved { sequences: state.sequences.iter().filter(|s| s.cue_id.is_some()).count(), timelines: state.timelines.len() };
    set(engine, user, Uuid::new_v4(), show_field("default_state"), to_value(&state)?).await?;
    Ok(saved)
}

/// Forget the default state: the show comes up with nothing running.
pub async fn clear(engine: &EngineHandle, user: Uuid) -> Result<(), String> {
    set(engine, user, Uuid::new_v4(), show_field("default_state"), Value::Null).await
}

/// What a show's sequences and timelines amount to as a default state.
pub fn of(sequences: &[Sequence], timelines: &[Timeline], at: u64) -> DefaultState {
    DefaultState {
        sequences: sequences
            .iter()
            .filter_map(|s| {
                let cue_id = s.active_cue_index.and_then(|i| s.cue_ids.get(i).copied());
                let master = s.master.filter(|m| (*m - 1.0).abs() > f32::EPSILON);
                (cue_id.is_some() || master.is_some()).then_some(DefaultSequence { sequence_id: s.id, cue_id, master })
            })
            .collect(),
        timelines: timelines.iter().filter(|t| t.running).map(|t| t.id).collect(),
        saved_at: at,
    }
}

/// Put the show into its default state: the sequences it names on their cues and every
/// other one off, the masters it names and every other one at full, the timelines it
/// names playing from their beginning and every other one stopped. One gesture, though
/// only its master levels can be taken back: a Go is not undoable.
///
/// `Ok(false)` where the show has no default state.
pub async fn apply(engine: &EngineHandle, user: Uuid) -> Result<bool, String> {
    let Some(state) = engine
        .get(show_field("default_state"))
        .await
        .ok()
        .and_then(|v| serde_json::from_value::<Option<DefaultState>>(v).ok())
        .flatten()
    else {
        return Ok(false);
    };
    let sequences: Vec<Sequence> = read(engine, "sequences").await;
    let timelines: Vec<Timeline> = read(engine, "timelines").await;
    let gesture = Uuid::new_v4();
    let at = now_ms();
    for sequence in &sequences {
        let wanted = state.sequences.iter().find(|d| d.sequence_id == sequence.id);
        let cue = wanted.and_then(|d| d.cue_id).filter(|c| sequence.cue_ids.contains(c));
        let now = sequence.active_cue_index.and_then(|i| sequence.cue_ids.get(i).copied());
        match cue {
            // Taken again even where it is already up: the default is the look, and a
            // cue that has been edited since it went should come up as it is now.
            Some(cue_id) => {
                set(engine, user, gesture, field("sequences", sequence.id, "goToCue"), json!({ "cueId": cue_id, "at": at })).await?
            }
            None if now.is_some() => set(engine, user, gesture, field("sequences", sequence.id, "off"), json!({ "at": at })).await?,
            None => {}
        }
        let master = wanted.and_then(|d| d.master);
        if sequence.master != master {
            set(engine, user, gesture, field("sequences", sequence.id, "master"), json!(master)).await?;
        }
    }
    for timeline in &timelines {
        if state.timelines.contains(&timeline.id) {
            set(engine, user, gesture, field("timelines", timeline.id, "play"), json!({ "at": at, "fromMs": 0 })).await?;
        } else if timeline.running {
            set(engine, user, gesture, field("timelines", timeline.id, "stop"), json!({ "at": at })).await?;
        }
    }
    Ok(true)
}

/// Bring a show that has just opened up in its default state, if this station is the
/// one to: see the module note. Run once per station start; `remembers` is whether this
/// station goes back to a remembered leader at all (`Config::rejoin`).
pub async fn at_start(engine: EngineHandle, net: NetHandle, node_id: NodeId, remembers: bool) {
    if remembers && crate::infra::session::remembered::load(node_id).is_some() {
        // A station that goes back to a leader takes that leader's show, running or not.
        return;
    }
    {
        tokio::time::sleep(GRACE).await;
        if !net.standing().is_leader {
            return;
        }
        let sequences: Vec<Sequence> = read(&engine, "sequences").await;
        let timelines: Vec<Timeline> = read(&engine, "timelines").await;
        if sequences.iter().any(|s| s.active_cue_index.is_some()) || timelines.iter().any(|t| t.running) {
            info!("[show] something is already running; the default state is left alone");
            return;
        }
        match apply(&engine, User::DEFAULT_ID).await {
            Ok(true) => info!("[show] came up in its default state"),
            Ok(false) => {}
            Err(e) => warn!("[show] could not bring the show up in its default state: {e}"),
        }
    }
}

async fn set(engine: &EngineHandle, user: Uuid, gesture: Uuid, path: Path, value: Value) -> Result<(), String> {
    let lifecycle = path_lifecycle(&path);
    engine.set_as(user, Some(gesture), path, lifecycle, value).await.map(|_| ()).map_err(|e| e.to_string())
}

async fn read<T: serde::de::DeserializeOwned>(engine: &EngineHandle, table: &str) -> Vec<T> {
    match engine.get(vec![PathSegment::Key(table.into())]).await.ok() {
        Some(Value::Array(rows)) => rows.into_iter().filter_map(|row| serde_json::from_value(row).ok()).collect(),
        _ => Vec::new(),
    }
}

fn field(table: &str, id: Uuid, name: &str) -> Path {
    vec![PathSegment::Key(table.into()), PathSegment::Id(id), PathSegment::Key(name.into())]
}

fn show_field(name: &str) -> Path {
    vec![PathSegment::Key("show".into()), PathSegment::Key(name.into())]
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sequence(cues: usize, active: Option<usize>, master: Option<f32>) -> Sequence {
        Sequence {
            id: Uuid::new_v4(),
            name: String::new(),
            cue_ids: (0..cues).map(|_| Uuid::new_v4()).collect(),
            active_cue_index: active,
            went_at: None,
            master,
            folder: None,
            merge: None,
            wrap: None,
        }
    }

    /// What is on, and what is not at full, and nothing else.
    #[test]
    fn a_default_state_is_what_is_on_and_what_is_not_at_full() {
        let on = sequence(3, Some(1), None);
        let dimmed = sequence(2, None, Some(0.5));
        let full = sequence(2, None, Some(1.0));
        let idle = sequence(2, None, None);
        let state = of(&[on.clone(), dimmed.clone(), full, idle], &[], 7);
        assert_eq!(
            state.sequences,
            vec![
                DefaultSequence { sequence_id: on.id, cue_id: Some(on.cue_ids[1]), master: None },
                DefaultSequence { sequence_id: dimmed.id, cue_id: None, master: Some(0.5) },
            ]
        );
        assert_eq!(state.saved_at, 7);
    }
}
