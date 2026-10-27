//! Update and Store: the two acts that put the programmer into a cue, written once.
//!
//! The top bar, the Store dialog, the command wing and the command line all end up
//! here — a page through the `programmer.update` and `programmer.store` RPCs, the wing
//! by calling these directly. There used to be four copies (`programmer.ts`, the
//! Store dialog, `wing_control` and the command-line plugin), and they had already
//! parted: the wing's Update was one Ctrl-Z where the page's was two, and the wing's
//! Store onto a key knew nothing about cue only or where a new cue goes. A store made
//! from the wing and one made from the screen are the same act, so they are one code
//! path.
//!
//! Every write of one act carries one gesture under the caller's name, so each act is
//! one Ctrl-Z however many cues it touches.

use std::collections::{HashMap, HashSet};

use pult_schema::{
    path::{Path, PathSegment},
    registry::path_lifecycle,
    types::{
        cue::{tracked_through, Cue, FollowMode, ParameterCapture},
        effect::{Easing, EffectSpec},
        parameter_key,
        programmer::ProgrammerValue,
        sequence::Sequence,
        wing::{BindingAction, ControlBinding, StoreOnto, SurfaceControl, COMMAND_WING},
        surface::TileTarget,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::EngineHandle;

/// The fade a cue made without being asked for one gets: none. `cues.ts`'s
/// `DEFAULT_FADE_MS`, and the Store dialog's starting value.
pub const DEFAULT_FADE_MS: u32 = 0;

// ── Update ────────────────────────────────────────────────────────────────────

/// What an Update did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Updated {
    /// How many cues it wrote into.
    pub updated: usize,
    /// The programmer entries no cue is driving, left in the programmer for the Store
    /// dialog to open with, ticked.
    pub orphans: Vec<Uuid>,
    /// The cue loaded for editing, where Update wrote back into that instead.
    pub edited: Option<CueName>,
}

/// A cue, as a toast names it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CueName {
    pub cue_id: Uuid,
    pub number: f64,
    pub name: String,
}

/// Update: every held value into the cue that is **driving it now**, one gesture over
/// however many cues it touches, and the programmer cleared once every value has found
/// its cue.
///
/// While a cue is loaded for editing, Update means that cue instead: it becomes
/// exactly what the programmer holds, timing kept, and the edit ends.
///
/// Keys nothing is driving are handed back rather than guessed at — there is no cue
/// they belong to — and stay in the programmer.
pub async fn update(engine: &EngineHandle, user: Uuid) -> Result<Updated, String> {
    let held: Vec<ProgrammerValue> = read(engine, "programmer_values").await;
    if held.is_empty() {
        return Err("The programmer is empty: nothing to update".into());
    }
    let writer = Writer { engine, user, gesture: Uuid::new_v4() };

    let editing = engine
        .get(vec![PathSegment::Key("show".into()), PathSegment::Key("editing_cue".into())])
        .await
        .ok()
        .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
    if let Some(cue_id) = editing {
        let cue: Cue = read_one(engine, "cues", cue_id)
            .await
            .ok_or("The cue being edited is no longer in the show")?;
        // Replace: the operator has the whole cue in front of them, and a parameter
        // they took out of the programmer is one they meant the cue to stop saying.
        let entries: Vec<&ProgrammerValue> = held.iter().collect();
        let captures = store_captures(&cue.captures, &entries, true, &HashMap::new());
        writer.set(field("cues", cue.id, "captures"), to_value(&captures)?).await?;
        writer.clear_programmer(&held).await?;
        writer.set(vec![PathSegment::Key("show".into()), PathSegment::Key("editing_cue".into())], Value::Null).await?;
        return Ok(Updated {
            updated: 1,
            orphans: Vec::new(),
            edited: Some(CueName { cue_id: cue.id, number: cue.number, name: cue.name }),
        });
    }

    let sequences: Vec<Sequence> = read(engine, "sequences").await;
    let cues: Vec<Cue> = read(engine, "cues").await;
    let driving = driving_cues(&sequences, &cues);
    let mut per_cue: Vec<(Uuid, Vec<&ProgrammerValue>)> = Vec::new();
    let mut orphans = Vec::new();
    for entry in &held {
        match driving.get(&(entry.fixture_id, parameter_key(&entry.parameter_kind))) {
            Some(cue) => match per_cue.iter_mut().find(|(id, _)| id == cue) {
                Some((_, list)) => list.push(entry),
                None => per_cue.push((*cue, vec![entry])),
            },
            None => orphans.push(entry.id),
        }
    }
    for (cue_id, stored) in &per_cue {
        let Some(cue) = cues.iter().find(|c| c.id == *cue_id) else { continue };
        let captures = store_captures(&cue.captures, stored, false, &HashMap::new());
        writer.set(field("cues", *cue_id, "captures"), to_value(&captures)?).await?;
    }
    if orphans.is_empty() {
        writer.clear_programmer(&held).await?;
    }
    Ok(Updated { updated: per_cue.len(), orphans, edited: None })
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// Where the programmer goes and how: everything the Store dialog asks. Every field
/// may be left out, and what is left out is what a store from the wing with no
/// screen means — all of it, as a new cue at the end, with no fade.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoreRequest {
    /// The programmer entries to store; all of them when left out.
    pub include: Option<Vec<Uuid>>,
    /// The sequence to store into.
    pub sequence_id: Option<Uuid>,
    /// Or a new sequence of this name, made in the same gesture.
    pub new_sequence: Option<String>,
    /// A cue of that sequence to store into; a new cue when left out.
    pub cue_id: Option<Uuid>,
    /// Into an existing cue: make it exactly this rather than merging.
    pub replace: bool,
    /// A new cue's name: "Cue <number>" when left out or empty.
    pub name: Option<String>,
    /// The cue a new one goes after, with a number between it and the next; appended
    /// when left out.
    pub after: Option<Uuid>,
    pub fade_in_ms: Option<u32>,
    pub fade_out_ms: Option<u32>,
    pub easing: Option<Easing>,
    pub follow_mode: Option<FollowMode>,
    /// Per-entry timing the operator typed. An entry with none keeps what the cue
    /// already had for that key, or none.
    pub timing: HashMap<Uuid, CaptureTiming>,
    /// Cue only: the next cue is given what it was tracking, so the change stops here.
    pub cue_only: bool,
    /// Keep the programmer rather than clearing it.
    pub keep: bool,
    /// The wing control STORE was pressed onto. One with nothing on it plays the
    /// sequence stored into from now on.
    pub onto: Option<StoreOnto>,
}

/// One capture's timing, as the Store dialog's row has it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptureTiming {
    pub fade_ms: u32,
    pub delay_ms: u32,
    pub easing: Option<Easing>,
}

/// What a Store made or wrote into.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stored {
    pub sequence_id: Uuid,
    pub sequence_name: String,
    pub cue: CueName,
    /// Whether the cue is one this store made.
    pub created: bool,
}

/// Store the programmer into a cue: a new one, or one that exists.
pub async fn store(engine: &EngineHandle, user: Uuid, request: StoreRequest) -> Result<Stored, String> {
    let held: Vec<ProgrammerValue> = read(engine, "programmer_values").await;
    if held.is_empty() {
        return Err("The programmer is empty: nothing to store".into());
    }
    let entries: Vec<&ProgrammerValue> = held
        .iter()
        .filter(|e| request.include.as_ref().is_none_or(|ids| ids.contains(&e.id)))
        .collect();
    if entries.is_empty() {
        return Err("Nothing is ticked to store".into());
    }
    let writer = Writer { engine, user, gesture: Uuid::new_v4() };
    let cues: Vec<Cue> = read(engine, "cues").await;

    let sequence = match (request.sequence_id, request.new_sequence.as_deref().map(str::trim)) {
        (Some(id), _) => read_one::<Sequence>(engine, "sequences", id).await.ok_or("That sequence is no longer in the show")?,
        (None, Some(name)) if !name.is_empty() => {
            let sequence = Sequence {
                id: Uuid::new_v4(),
                name: name.to_string(),
                cue_ids: Vec::new(),
                active_cue_index: None,
                went_at: None,
                master: None,
                folder: None,
                merge: None,
                wrap: None,
            };
            writer.set(create("sequences"), to_value(&sequence)?).await?;
            sequence
        }
        _ => return Err("Store into which sequence?".into()),
    };

    let existing = match request.cue_id {
        Some(id) if sequence.cue_ids.contains(&id) => {
            Some(cues.iter().find(|c| c.id == id).ok_or("That cue is no longer in the show")?)
        }
        Some(_) => return Err("That cue is not in this sequence".into()),
        None => None,
    };
    let after = request.after.filter(|id| sequence.cue_ids.contains(id));

    // Cue only preserves what the next cue was tracking, which is a fact about the show
    // as it stands and not about the show this store is about to make — so it is read
    // before anything is written.
    let ahead = if request.cue_only {
        let into = existing.map(|c| c.id).or(after);
        into.and_then(|into| next_and_tracked(&sequence, &cues, into))
    } else {
        None
    };

    let (cue, created) = match existing {
        Some(cue) => {
            let captures = store_captures(&cue.captures, &entries, request.replace, &request.timing);
            writer.set(field("cues", cue.id, "captures"), to_value(&captures)?).await?;
            (CueName { cue_id: cue.id, number: cue.number, name: cue.name.clone() }, false)
        }
        None => {
            let number = new_cue_number(&sequence, &cues, after);
            let name = request
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| default_cue_name(number));
            let cue = Cue {
                id: Uuid::new_v4(),
                name,
                number,
                captures: store_captures(&[], &entries, false, &request.timing),
                follow_mode: request.follow_mode.clone().unwrap_or(FollowMode::Manual),
                fade_in_ms: request.fade_in_ms.unwrap_or(DEFAULT_FADE_MS),
                fade_out_ms: request.fade_out_ms.unwrap_or(DEFAULT_FADE_MS),
                easing: request.easing,
                is_active: false,
            };
            writer.set(create("cues"), to_value(&cue)?).await?;
            let mut ids = sequence.cue_ids.clone();
            let at = after.and_then(|a| ids.iter().position(|id| *id == a)).map_or(ids.len(), |i| i + 1);
            ids.insert(at, cue.id);
            writer.set(field("sequences", sequence.id, "cue_ids"), json!(ids)).await?;
            (CueName { cue_id: cue.id, number: cue.number, name: cue.name }, true)
        }
    };

    if let Some((next, before)) = ahead {
        let changed: Vec<(Uuid, String)> =
            entries.iter().map(|e| (e.fixture_id, parameter_key(&e.parameter_kind))).collect();
        if let Some(compensated) = cue_only_compensation(next, &changed, &before) {
            writer.set(field("cues", next.id, "captures"), to_value(&compensated)?).await?;
        }
    }

    if let Some(onto) = request.onto.as_ref().filter(|o| o.sequence_id.is_none()) {
        bind_onto(&writer, onto, sequence.id).await?;
    }
    if !request.keep {
        writer.clear_programmer(&held).await?;
    }
    Ok(Stored { sequence_id: sequence.id, sequence_name: sequence.name, cue, created })
}

/// A wing control that had nothing on it plays the sequence just stored onto it: a
/// fader is its level, a key its Go.
async fn bind_onto(writer: &Writer<'_>, onto: &StoreOnto, sequence_id: Uuid) -> Result<(), String> {
    let action = match onto.control {
        SurfaceControl::Fader { .. } => BindingAction::Master { sequence_id },
        _ => BindingAction::Press { target: TileTarget::SequenceGo { sequence_id } },
    };
    let bindings: Vec<ControlBinding> = read(writer.engine, "control_bindings").await;
    match bindings.iter().find(|b| b.surface == COMMAND_WING && b.page() == onto.page && b.control == onto.control) {
        Some(row) => writer.set(field("control_bindings", row.id, "action"), to_value(&action)?).await,
        None => {
            let row = ControlBinding {
                id: Uuid::new_v4(),
                surface: COMMAND_WING.into(),
                control: onto.control,
                action,
                label: String::new(),
                page: Some(onto.page),
            };
            writer.set(create("control_bindings"), to_value(&row)?).await
        }
    }
}

// ── The arithmetic ────────────────────────────────────────────────────────────

/// Which cue is driving each key: the latest capture of it over the tracked stack of
/// each live sequence, the sequence that went most recently winning — what a Go put
/// there.
///
/// **Not `live_fades`**: a key the programmer holds is exactly the key playback stops
/// publishing (`emit_motion` skips every fade and effect under one), so at the moment
/// Update needs the answer the row is gone. A sequence that has never gone counts as
/// oldest; an equal stamp goes to the one listed later, so it does not flap.
pub fn driving_cues(sequences: &[Sequence], cues: &[Cue]) -> HashMap<(Uuid, String), Uuid> {
    let mut live: Vec<&Sequence> = sequences.iter().filter(|s| s.active_cue_index.is_some()).collect();
    live.sort_by_key(|s| s.went_at.unwrap_or(0));
    let by_id: HashMap<Uuid, &Cue> = cues.iter().map(|c| (c.id, c)).collect();
    let mut out = HashMap::new();
    for sequence in live {
        let Some(active) = sequence.active_cue_index else { continue };
        let through = sequence.cue_ids.iter().take(active + 1);
        for (cue, capture) in tracked_through(through, |id| by_id.get(id).copied()) {
            out.insert((capture.fixture_id, parameter_key(&capture.parameter_kind)), cue.id);
        }
    }
    out
}

/// What a cue's captures become once these entries are stored into it.
///
/// *Merge* keeps everything else the cue said; *replace* makes it exactly these. Timing
/// is **kept, not rewritten**: a capture the cue already had keeps its own fade, delay
/// and curve unless the operator typed new ones, and a key arriving for the first time
/// gets none, which playback reads as "use the cue's". The programmer holds values and
/// never timing, so a store that wrote zeroes would throw away, on every Update, timing
/// set somewhere that was not on the screen.
pub fn store_captures(
    existing: &[ParameterCapture],
    entries: &[&ProgrammerValue],
    replace: bool,
    timing: &HashMap<Uuid, CaptureTiming>,
) -> Vec<ParameterCapture> {
    let before: HashMap<(Uuid, String), &ParameterCapture> =
        existing.iter().map(|c| ((c.fixture_id, parameter_key(&c.parameter_kind)), c)).collect();
    let stored: Vec<ParameterCapture> = entries
        .iter()
        .map(|entry| {
            let kept = before.get(&(entry.fixture_id, parameter_key(&entry.parameter_kind)));
            let typed = timing.get(&entry.id);
            ParameterCapture {
                fixture_id: entry.fixture_id,
                parameter_kind: entry.parameter_kind.clone(),
                value: entry.value.clone(),
                // Reference first, literal beside it: a deleted preset leaves the cue
                // running exactly as it was.
                preset: entry.preset,
                fade_in_ms: typed.map(|t| t.fade_ms).or(kept.map(|c| c.fade_in_ms)).unwrap_or(0),
                fade_out_ms: kept.map_or(0, |c| c.fade_out_ms),
                delay_in_ms: typed.map(|t| t.delay_ms).or(kept.map(|c| c.delay_in_ms)).unwrap_or(0),
                // A stored effect drops its anchor: the cue's `went_at` is what it is
                // measured from on every Go, so two consoles start the same cycle.
                effect: entry.effect.clone().map(|spec| EffectSpec { t0: None, ..spec }),
                // `None` is not linear: it is this capture saying nothing, so the cue
                // answers, and the show answers for the cue.
                easing: typed.map_or(kept.and_then(|c| c.easing), |t| t.easing),
            }
        })
        .collect();
    if replace {
        return stored;
    }
    let taken: HashSet<(Uuid, String)> =
        stored.iter().map(|c| (c.fixture_id, parameter_key(&c.parameter_kind))).collect();
    existing
        .iter()
        .filter(|c| !taken.contains(&(c.fixture_id, parameter_key(&c.parameter_kind))))
        .cloned()
        .chain(stored)
        .collect()
}

/// The cue after `into` in the sequence, and what it was tracking through `into`.
fn next_and_tracked<'a>(
    sequence: &Sequence,
    cues: &'a [Cue],
    into: Uuid,
) -> Option<(&'a Cue, HashMap<(Uuid, String), ParameterCapture>)> {
    let at = sequence.cue_ids.iter().position(|id| *id == into)?;
    let next_id = sequence.cue_ids.get(at + 1)?;
    let next = cues.iter().find(|c| c.id == *next_id)?;
    let by_id: HashMap<Uuid, &Cue> = cues.iter().map(|c| (c.id, c)).collect();
    let before = tracked_through(&sequence.cue_ids[..=at], |id| by_id.get(id).copied())
        .into_iter()
        .map(|(_, capture)| ((capture.fixture_id, parameter_key(&capture.parameter_kind)), capture.clone()))
        .collect();
    Some((next, before))
}

/// What the *next* cue has to say so that a store into this one does not track
/// forward: for each key the store changes that the next cue does not capture itself,
/// the value it was tracking before the store. Cue only, as every other desk calls it.
///
/// The compensating capture carries the value and none of the timing — it is going
/// into a different cue and should move the way that cue moves. A key nothing was
/// tracking needs none, and the last cue has no next, so cue only does nothing there.
pub fn cue_only_compensation(
    next: &Cue,
    changed: &[(Uuid, String)],
    before: &HashMap<(Uuid, String), ParameterCapture>,
) -> Option<Vec<ParameterCapture>> {
    let mut already: HashSet<(Uuid, String)> =
        next.captures.iter().map(|c| (c.fixture_id, parameter_key(&c.parameter_kind))).collect();
    let mut added = Vec::new();
    for key in changed {
        if already.contains(key) {
            continue;
        }
        let Some(tracked) = before.get(key) else { continue };
        added.push(ParameterCapture {
            effect: tracked.effect.clone().map(|spec| EffectSpec { t0: None, ..spec }),
            fade_in_ms: 0,
            fade_out_ms: 0,
            delay_in_ms: 0,
            easing: None,
            ..tracked.clone()
        });
        already.insert(key.clone());
    }
    (!added.is_empty()).then(|| next.captures.iter().cloned().chain(added).collect())
}

/// The number a new cue gets: between `after` and the cue behind it, or the next whole
/// number at the end. `cues.ts`'s `newCueNumber`.
pub fn new_cue_number(sequence: &Sequence, cues: &[Cue], after: Option<Uuid>) -> f64 {
    let order: Vec<&Cue> = sequence.cue_ids.iter().filter_map(|id| cues.iter().find(|c| c.id == *id)).collect();
    match after.and_then(|a| order.iter().position(|c| c.id == a)) {
        Some(at) => insert_number(order[at].number, order.get(at + 1).map(|c| c.number)),
        None => (order.len() + 1) as f64,
    }
}

/// The midpoint between two cues, or the next whole number after the last one.
pub fn insert_number(before: f64, after: Option<f64>) -> f64 {
    match after {
        Some(after) => before + (after - before) / 2.0,
        None => before.floor() + 1.0,
    }
}

/// "Cue 4", "Cue 3.5": the name a new cue gets when nobody types one.
pub fn default_cue_name(number: f64) -> String {
    if number.fract() == 0.0 {
        return format!("Cue {}", number as i64);
    }
    let text = format!("{number:.3}");
    format!("Cue {}", text.trim_end_matches('0').trim_end_matches('.'))
}

// ── Reading and writing ───────────────────────────────────────────────────────

/// Every write of one act: one user, one gesture.
struct Writer<'a> {
    engine: &'a EngineHandle,
    user: Uuid,
    gesture: Uuid,
}

impl Writer<'_> {
    async fn set(&self, path: Path, value: Value) -> Result<(), String> {
        let lifecycle = path_lifecycle(&path);
        self.engine
            .set_as(self.user, Some(self.gesture), path, lifecycle, value)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Clear what is not locked: a locked value is parked on purpose and stays.
    async fn clear_programmer(&self, held: &[ProgrammerValue]) -> Result<(), String> {
        for entry in held.iter().filter(|e| !e.locked) {
            self.set(field("programmer_values", entry.id, "__delete"), Value::Null).await?;
        }
        Ok(())
    }
}

async fn read<T: serde::de::DeserializeOwned>(engine: &EngineHandle, table: &str) -> Vec<T> {
    match engine.get(vec![PathSegment::Key(table.into())]).await.ok() {
        Some(Value::Array(rows)) => rows.into_iter().filter_map(|row| serde_json::from_value(row).ok()).collect(),
        _ => Vec::new(),
    }
}

async fn read_one<T: serde::de::DeserializeOwned>(engine: &EngineHandle, table: &str, id: Uuid) -> Option<T> {
    let row = engine.get(vec![PathSegment::Key(table.into()), PathSegment::Id(id)]).await.ok()?;
    serde_json::from_value(row).ok()
}

fn field(table: &str, id: Uuid, name: &str) -> Path {
    vec![PathSegment::Key(table.into()), PathSegment::Id(id), PathSegment::Key(name.into())]
}

fn create(table: &str) -> Path {
    vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())]
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pult_schema::types::fixture::{ParameterKind, ParameterValue};

    fn entry(fixture: Uuid, value: f32) -> ProgrammerValue {
        ProgrammerValue {
            id: Uuid::new_v4(),
            fixture_id: fixture,
            parameter_kind: ParameterKind::Intensity,
            value: ParameterValue::Float(value),
            effect: None,
            preset: None,
            locked: false,
        }
    }

    fn capture(fixture: Uuid, value: f32, fade: u32) -> ParameterCapture {
        ParameterCapture {
            fixture_id: fixture,
            parameter_kind: ParameterKind::Intensity,
            value: ParameterValue::Float(value),
            fade_in_ms: fade,
            fade_out_ms: 0,
            delay_in_ms: 0,
            effect: None,
            easing: None,
            preset: None,
        }
    }

    fn cue(captures: Vec<ParameterCapture>, number: f64) -> Cue {
        Cue {
            id: Uuid::new_v4(),
            name: String::new(),
            number,
            captures,
            follow_mode: FollowMode::Manual,
            fade_in_ms: 0,
            fade_out_ms: 0,
            easing: None,
            is_active: false,
        }
    }

    fn sequence(cues: &[&Cue], active: Option<usize>, went_at: u64) -> Sequence {
        Sequence {
            id: Uuid::new_v4(),
            name: String::new(),
            cue_ids: cues.iter().map(|c| c.id).collect(),
            active_cue_index: active,
            went_at: Some(went_at),
            master: None,
            folder: None,
            merge: None,
            wrap: None,
        }
    }

    /// Update keeps the timing an operator set on a capture, and the rest of the cue.
    #[test]
    fn a_merge_keeps_a_captures_timing_and_the_rest_of_the_cue() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let held = entry(a, 0.5);
        let out = store_captures(&[capture(a, 1.0, 3000), capture(b, 1.0, 100)], &[&held], false, &HashMap::new());
        assert_eq!(out.len(), 2);
        let mine = out.iter().find(|c| c.fixture_id == a).unwrap();
        assert_eq!(mine.value, ParameterValue::Float(0.5));
        assert_eq!(mine.fade_in_ms, 3000, "the capture's own fade survives");
    }

    #[test]
    fn a_replace_is_exactly_the_programmer_and_typed_timing_wins() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let held = entry(a, 0.5);
        let timing = HashMap::from([(held.id, CaptureTiming { fade_ms: 800, delay_ms: 200, easing: None })]);
        let out = store_captures(&[capture(a, 1.0, 3000), capture(b, 1.0, 100)], &[&held], true, &timing);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].fade_in_ms, out[0].delay_in_ms), (800, 200));
    }

    /// The palette a value came from goes into the cue beside the number, and a
    /// stored effect drops its anchor: the cue's `went_at` supplies one on every Go.
    #[test]
    fn a_store_keeps_the_preset_and_drops_an_effects_anchor() {
        let preset = Uuid::new_v4();
        let mut held = entry(Uuid::new_v4(), 0.8);
        held.preset = Some(preset);
        let out = store_captures(&[], &[&held], false, &HashMap::new());
        assert_eq!(out[0].preset, Some(preset));
        assert_eq!(out[0].value, ParameterValue::Float(0.8));
        assert_eq!((out[0].fade_in_ms, out[0].easing), (0, None), "nothing said, so the cue answers");
    }

    #[test]
    fn a_sequence_that_is_off_or_never_went_drives_accordingly() {
        let a = Uuid::new_v4();
        let one = cue(vec![capture(a, 1.0, 0)], 1.0);
        let two = cue(vec![capture(a, 0.5, 0)], 1.0);
        let cues = [one.clone(), two.clone()];
        let key = (a, parameter_key(&ParameterKind::Intensity));
        assert!(driving_cues(&[sequence(&[&one], None, 100)], &cues).is_empty(), "off drives nothing");
        let mut never = sequence(&[&one], Some(0), 0);
        never.went_at = None;
        let went = sequence(&[&two], Some(0), 1);
        assert_eq!(driving_cues(&[went.clone(), never.clone()], &cues).get(&key), Some(&two.id), "never went is oldest");
        assert_eq!(driving_cues(&[never, went], &cues).get(&key), Some(&two.id));
    }

    #[test]
    fn cue_only_leaves_a_key_the_next_cue_captures_and_one_nothing_tracked() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let key = |f| (f, parameter_key(&ParameterKind::Intensity));
        let next = cue(vec![capture(a, 0.9, 0)], 2.0);
        let before = HashMap::from([(key(a), capture(a, 0.3, 0))]);
        assert!(cue_only_compensation(&next, &[key(a), key(b)], &before).is_none());
    }

    /// The sequence that went last owns a key both capture.
    #[test]
    fn the_sequence_that_went_last_drives_a_contested_key() {
        let a = Uuid::new_v4();
        let early = cue(vec![capture(a, 1.0, 0)], 1.0);
        let late = cue(vec![capture(a, 0.2, 0)], 1.0);
        let off = cue(vec![capture(a, 0.7, 0)], 1.0);
        let sequences = [sequence(&[&late], Some(0), 20), sequence(&[&early], Some(0), 10), sequence(&[&off], None, 30)];
        let cues = [early.clone(), late.clone(), off];
        let driving = driving_cues(&sequences, &cues);
        assert_eq!(driving.get(&(a, parameter_key(&ParameterKind::Intensity))), Some(&late.id));
    }

    /// A tracked key belongs to the cue that last captured it, not the one that is up.
    #[test]
    fn a_tracked_key_is_driven_by_the_cue_that_set_it() {
        let a = Uuid::new_v4();
        let one = cue(vec![capture(a, 1.0, 0)], 1.0);
        let two = cue(vec![], 2.0);
        let cues = [one.clone(), two.clone()];
        let driving = driving_cues(&[sequence(&[&one, &two], Some(1), 5)], &cues);
        assert_eq!(driving.get(&(a, parameter_key(&ParameterKind::Intensity))), Some(&one.id));
    }

    #[test]
    fn cue_only_gives_the_next_cue_what_it_was_tracking() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let one = cue(vec![capture(a, 1.0, 500), capture(b, 1.0, 0)], 1.0);
        let two = cue(vec![capture(b, 0.0, 0)], 2.0);
        let seq = sequence(&[&one, &two], Some(0), 5);
        let cues = [one.clone(), two.clone()];
        let (next, before) = next_and_tracked(&seq, &cues, one.id).unwrap();
        assert_eq!(next.id, two.id);
        let key = |f| (f, parameter_key(&ParameterKind::Intensity));
        let out = cue_only_compensation(next, &[key(a), key(b)], &before).unwrap();
        assert_eq!(out.len(), 2, "b was already captured by the next cue; a is added");
        let added = out.iter().find(|c| c.fixture_id == a).unwrap();
        assert_eq!((added.value.clone(), added.fade_in_ms), (ParameterValue::Float(1.0), 0), "the value and none of the timing");
    }

    #[test]
    fn a_new_cue_is_numbered_between_or_after() {
        let one = cue(vec![], 1.0);
        let two = cue(vec![], 2.0);
        let seq = sequence(&[&one, &two], None, 0);
        let cues = [one.clone(), two.clone()];
        assert_eq!(new_cue_number(&seq, &cues, Some(one.id)), 1.5);
        assert_eq!(new_cue_number(&seq, &cues, Some(two.id)), 3.0);
        assert_eq!(new_cue_number(&seq, &cues, None), 3.0);
        assert_eq!(insert_number(4.75, None), 5.0);
        assert_eq!(default_cue_name(4.0), "Cue 4");
        assert_eq!(default_cue_name(3.5), "Cue 3.5");
        assert_eq!(default_cue_name(1.125), "Cue 1.125");
    }
}
