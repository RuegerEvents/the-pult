//! The wing's time-critical controls, run by the station the wing hangs off.
//!
//! A fader bound to a parameter, a flash key, a sequence's master fader with its Go and
//! Off keys, and the motors that follow the show. These used to be a page's: the station
//! relayed every event to whichever browser drove the wing, and that page wrote the
//! programmer once per animation frame and sent the motor positions back. Recording a
//! skeleton then went through a browser's main thread and a round trip, and a page busy
//! drawing something lagged the faders with it (found at the gig, 2026-10-25). So these
//! run here, on the events as they come off the USB thread, whether or not any page has
//! claimed the wing. What stays a page's is what is about one operator: the command
//! line, the encoders and their calculator, learning a binding, the lights.
//!
//! Two rules carried over unchanged, because both were bugs once. **Only a fader a hand
//! is on is listened to** — its motor's own travel arrives exactly like a hand's, and
//! written back it would turn every cue into a programmer value. And **a flash knows
//! which value it put there itself**, so a second tap before the first release has
//! landed does not take the key's own full for the operator's and put it back.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use pult_schema::{
    path::{Path, PathSegment},
    registry::path_lifecycle,
    types::{
        fixture::{Fixture, FixtureType, ParameterKind, ParameterValue},
        parameter_key,
        programmer::{programmer_entry_id, ProgrammerValue},
        sequence::Sequence,
        wing::{BindingAction, ControlBinding, SurfaceControl, COMMAND_WING},
    },
};
use pult_wing::{Event, WingMap};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{engine::EngineHandle, infra::connectors::OutputHandle, wing::WingHandle};

/// A fader's full travel, as the wing reports it.
const FADER_FULL: f32 = 4096.0;
/// How often a moving fader's value is written: the page's rate, a frame of 40 Hz.
const WRITE_EVERY: Duration = Duration::from_millis(25);
/// How often the motors are told where the show is.
const MOTORS_EVERY: Duration = Duration::from_millis(50);
/// How often the bindings are read again, beside being told they changed.
const BINDINGS_EVERY: Duration = Duration::from_secs(2);

/// What a hardkey is to a fader: the key under it (Go) or above it (Off).
#[derive(Clone, Copy)]
enum FaderKey {
    Go(u16),
    Off(u16),
}

/// Which keys are a fader's Go and Off, from MA's table: the `EXEC` key of the fader's
/// own executor is under it, the one a hundred below is above it. The station's copy of
/// `faderKeys` in the frontend's `wing.ts`, without the touch half (`wing.rs` has that).
fn fader_keys(map: &WingMap) -> HashMap<u16, FaderKey> {
    let by_executor: HashMap<&str, u16> = map
        .faders
        .iter()
        .filter_map(|(index, c)| Some((c.executor.as_deref()?, *index as u16)))
        .collect();
    let mut out = HashMap::new();
    for (index, control) in &map.hardkeys {
        if control.code.as_deref() != Some("EXEC") {
            continue;
        }
        let Some(executor) = control.executor.as_deref() else { continue };
        if let Some(fader) = by_executor.get(executor) {
            out.insert(*index, FaderKey::Go(*fader));
        }
        if let Ok(n) = executor.parse::<u32>() {
            if let Some(fader) = by_executor.get((n + 100).to_string().as_str()) {
                out.insert(*index, FaderKey::Off(*fader));
            }
        }
    }
    out
}

/// A fader's travel as a level, 0..1.
fn level_of(raw: u16) -> f32 {
    (raw as f32 / FADER_FULL).clamp(0.0, 1.0)
}

/// The value a fader at `level` makes of a parameter like `like`. A color is its
/// brightness, keeping `hue` — what it was last time it was not black, because black
/// has no hue and the fader would be dead from the bottom up.
fn from_fader(like: &ParameterValue, level: f32, hue: Option<(f32, f32, f32)>) -> ParameterValue {
    match like {
        ParameterValue::Float(_) => ParameterValue::Float(level),
        ParameterValue::Int(_) => ParameterValue::Int((level * 255.0).round() as i32),
        ParameterValue::Bool(_) => ParameterValue::Bool(level >= 0.5),
        ParameterValue::Color { overrides, .. } => {
            let (r, g, b) = hue.or_else(|| hue_of(like)).unwrap_or((1.0, 1.0, 1.0));
            ParameterValue::Color { r: r * level, g: g * level, b: b * level, overrides: overrides.clone() }
        }
        other => other.clone(),
    }
}

/// A color's hue with the brightest channel at one; `None` for black and non-colors.
fn hue_of(value: &ParameterValue) -> Option<(f32, f32, f32)> {
    let ParameterValue::Color { r, g, b, .. } = value else { return None };
    let top = r.max(*g).max(*b);
    (top > 0.0).then(|| (r / top, g / top, b / top))
}

/// Where a motor puts a fader for a value; `None` for one it cannot show.
fn to_fader(value: &ParameterValue) -> Option<f32> {
    Some(match value {
        ParameterValue::Float(v) => *v,
        ParameterValue::Int(v) => *v as f32 / 255.0,
        ParameterValue::Bool(on) => f32::from(u8::from(*on)),
        ParameterValue::Color { r, g, b, .. } => r.max(*g).max(*b),
        _ => return None,
    })
}

/// A flash's value: full of the kind the parameter is.
fn full(like: &ParameterValue) -> ParameterValue {
    match like {
        ParameterValue::Float(_) => ParameterValue::Float(1.0),
        ParameterValue::Int(_) => ParameterValue::Int(255),
        ParameterValue::Bool(_) => ParameterValue::Bool(true),
        ParameterValue::Color { .. } => {
            ParameterValue::Color { r: 1.0, g: 1.0, b: 1.0, overrides: Default::default() }
        }
        other => other.clone(),
    }
}

struct Control {
    wing: WingHandle,
    engine: EngineHandle,
    output: OutputHandle,
    bindings: Vec<ControlBinding>,
    fader_keys: HashMap<u16, FaderKey>,
    /// Programmer writes waiting for the next tick, newest value per entry.
    pending: HashMap<Uuid, (Uuid, ParameterKind, ParameterValue)>,
    /// Master levels waiting for the next tick, per sequence.
    masters: HashMap<Uuid, f32>,
    /// What each flash key held down will put back: the value it displaced, or none.
    flashed: HashMap<Uuid, Option<ParameterValue>>,
    /// Entries a flash put there itself and has not yet taken away.
    flash_owned: HashSet<Uuid>,
    /// The hue each color a fader moves had, last time it was not black.
    hues: HashMap<Uuid, (f32, f32, f32)>,
}

/// Run the wing's time-critical controls for as long as this station runs.
pub async fn run(wing: WingHandle, engine: EngineHandle, output: OutputHandle) {
    let fader_keys = wing.map().map(|m| fader_keys(&m)).unwrap_or_default();
    let mut events = wing.events();
    let mut control = Control {
        wing,
        engine,
        output,
        bindings: Vec::new(),
        fader_keys,
        pending: HashMap::new(),
        masters: HashMap::new(),
        flashed: HashMap::new(),
        flash_owned: HashSet::new(),
        hues: HashMap::new(),
    };
    control.read_bindings().await;

    let mut write = tokio::time::interval(WRITE_EVERY);
    write.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut motors = tokio::time::interval(MOTORS_EVERY);
    motors.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut bindings = tokio::time::interval(BINDINGS_EVERY);
    bindings.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            batch = events.recv() => match batch {
                Ok(batch) => {
                    for event in batch {
                        control.on_event(event).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("[wing] the station's controls fell behind by {n} batches");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = write.tick() => control.write_pending().await,
            _ = motors.tick() => control.drive_motors().await,
            _ = bindings.tick() => control.read_bindings().await,
        }
    }
}

impl Control {
    async fn read_bindings(&mut self) {
        let rows: Vec<ControlBinding> = self
            .engine
            .get(vec![PathSegment::Key("control_bindings".into())])
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        self.bindings = rows.into_iter().filter(|b| b.surface == COMMAND_WING).collect();
    }

    fn binding(&self, control: &SurfaceControl) -> Option<&BindingAction> {
        self.bindings.iter().find(|b| &b.control == control).map(|b| &b.action)
    }

    async fn on_event(&mut self, event: Event) {
        match event {
            Event::Fader { index, value } => {
                let action = self.binding(&SurfaceControl::Fader { index: index as u16 }).cloned();
                let Some(action) = action else { return };
                if !self.wing.touched(index as usize) {
                    return;
                }
                let level = level_of(value);
                match action {
                    BindingAction::Parameter { fixture_id, parameter } => {
                        let id = entry_id(fixture_id, &parameter);
                        let like = self.current(fixture_id, &parameter).await;
                        if let Some(hue) = hue_of(&like) {
                            self.hues.insert(id, hue);
                        }
                        let value = from_fader(&like, level, self.hues.get(&id).copied());
                        self.pending.insert(id, (fixture_id, parameter, value));
                    }
                    BindingAction::Master { sequence_id } => {
                        self.masters.insert(sequence_id, level);
                    }
                    _ => {}
                }
            }
            Event::Key { index, down } => {
                if let Some(BindingAction::Flash { fixture_id, parameter }) =
                    self.binding(&SurfaceControl::Key { index }).cloned()
                {
                    self.flash(fixture_id, parameter, down).await;
                    return;
                }
                if !down {
                    return;
                }
                let Some(role) = self.fader_keys.get(&index).copied() else { return };
                let (fader, verb) = match role {
                    FaderKey::Go(f) => (f, "goNext"),
                    FaderKey::Off(f) => (f, "off"),
                };
                // A key that is itself bound does what it is bound to, which is a page's
                // to run; only an unbound executor key belongs to its master fader.
                if self.binding(&SurfaceControl::Key { index }).is_some() {
                    return;
                }
                if let Some(BindingAction::Master { sequence_id }) =
                    self.binding(&SurfaceControl::Fader { index: fader }).cloned()
                {
                    let path = field(&["sequences"], sequence_id, verb);
                    let at = pult_schema::types::sequence::now_ms();
                    self.set(path, serde_json::json!({ "at": at })).await;
                }
            }
            _ => {}
        }
    }

    /// Full while held, and on release whatever was there before — or nothing.
    async fn flash(&mut self, fixture_id: Uuid, parameter: ParameterKind, down: bool) {
        let id = entry_id(fixture_id, &parameter);
        // A press that lands before writes still waiting would be overtaken by them.
        self.write_pending().await;
        if down {
            let before = if self.flash_owned.contains(&id) { None } else { self.held(id).await };
            if before.is_none() {
                self.flash_owned.insert(id);
            }
            self.flashed.insert(id, before);
            let like = self.current(fixture_id, &parameter).await;
            self.write_value(fixture_id, &parameter, full(&like)).await;
        } else {
            match self.flashed.remove(&id).flatten() {
                Some(before) => self.write_value(fixture_id, &parameter, before).await,
                None => {
                    let path = vec![
                        PathSegment::Key("programmer_values".into()),
                        PathSegment::Id(id),
                        PathSegment::Key("__delete".into()),
                    ];
                    self.set(path, serde_json::Value::Null).await;
                    if !self.flashed.contains_key(&id) {
                        self.flash_owned.remove(&id);
                    }
                }
            }
        }
    }

    /// Write what the faders moved since the last tick.
    async fn write_pending(&mut self) {
        for (_, (fixture_id, parameter, value)) in std::mem::take(&mut self.pending) {
            self.write_value(fixture_id, &parameter, value).await;
        }
        for (sequence_id, level) in std::mem::take(&mut self.masters) {
            // Full is written as no level at all, which is what a show opens at.
            let value = if level >= 0.999 { serde_json::Value::Null } else { serde_json::json!(level) };
            self.set(field(&["sequences"], sequence_id, "master"), value).await;
        }
    }

    /// Put every bound fader where the show is, and let every other one go. A touched
    /// fader is left to the hand by `WingHandle::drive` itself.
    async fn drive_motors(&mut self) {
        let faders: Vec<(u16, BindingAction)> = self
            .bindings
            .iter()
            .filter_map(|b| match b.control {
                SurfaceControl::Fader { index } => Some((index, b.action.clone())),
                _ => None,
            })
            .collect();
        let asks: Vec<(Uuid, String)> = faders
            .iter()
            .filter_map(|(_, action)| match action {
                BindingAction::Parameter { fixture_id, parameter } => Some((*fixture_id, parameter_key(parameter))),
                _ => None,
            })
            .collect();
        let mut values = self.output.values(asks).await.into_iter();
        let sequences: Vec<Sequence> = if faders.iter().any(|(_, a)| matches!(a, BindingAction::Master { .. })) {
            self.read("sequences").await
        } else {
            Vec::new()
        };
        let mut positions: HashMap<usize, u16> = HashMap::new();
        for (index, action) in &faders {
            let level = match action {
                BindingAction::Parameter { .. } => values.next().flatten().as_ref().and_then(to_fader),
                BindingAction::Master { sequence_id } => {
                    sequences.iter().find(|s| s.id == *sequence_id).map(|s| s.master.unwrap_or(1.0))
                }
                _ => None,
            };
            if let Some(level) = level {
                positions.insert(*index as usize, (level.clamp(0.0, 1.0) * FADER_FULL).round() as u16);
            }
        }
        self.wing.drive(|out| {
            for index in 0..out.faders.len() {
                match positions.get(&index) {
                    Some(raw) => out.set_fader(index, *raw),
                    None => out.release_fader(index),
                }
            }
        });
    }

    /// What a parameter is now, to shape the value a fader or a flash writes: the
    /// programmer's entry, else what the rig is doing, else the type's own default.
    async fn current(&self, fixture_id: Uuid, parameter: &ParameterKind) -> ParameterValue {
        if let Some(held) = self.held(entry_id(fixture_id, parameter)).await {
            return held;
        }
        let key = parameter_key(parameter);
        if let Some(Some(value)) = self.output.values(vec![(fixture_id, key.clone())]).await.into_iter().next() {
            return value;
        }
        let fixture: Option<Fixture> = self
            .engine
            .get(vec![PathSegment::Key("fixtures".into()), PathSegment::Id(fixture_id)])
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v).ok());
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        fixture
            .and_then(|f| types.into_iter().find(|t| t.id == f.fixture_type_id))
            .and_then(|t| t.parameter_for_key(&key).map(|p| p.default_value.clone()))
            .unwrap_or(ParameterValue::Float(0.0))
    }

    /// The programmer's value for an entry, if it holds one.
    async fn held(&self, id: Uuid) -> Option<ParameterValue> {
        let row = self
            .engine
            .get(vec![PathSegment::Key("programmer_values".into()), PathSegment::Id(id)])
            .await
            .ok()?;
        serde_json::from_value::<ProgrammerValue>(row).ok().map(|entry| entry.value)
    }

    /// One programmer value, the way a page writes one: a field write to an entry that
    /// is there, the whole row where it carried a preset (moving a value breaks the
    /// link, in one write), and a create where there is none.
    async fn write_value(&self, fixture_id: Uuid, parameter: &ParameterKind, value: ParameterValue) {
        let id = entry_id(fixture_id, parameter);
        let existing: Option<ProgrammerValue> = self
            .engine
            .get(vec![PathSegment::Key("programmer_values".into()), PathSegment::Id(id)])
            .await
            .ok()
            .and_then(|row| serde_json::from_value(row).ok());
        match existing {
            Some(entry) if entry.value == value => {}
            Some(entry) if entry.preset.is_some() => {
                let row = ProgrammerValue { value, preset: None, ..entry };
                let path = vec![PathSegment::Key("programmer_values".into()), PathSegment::Id(id)];
                if let Ok(json) = serde_json::to_value(&row) {
                    self.set(path, json).await;
                }
            }
            Some(_) => {
                let path = field(&["programmer_values"], id, "value");
                if let Ok(json) = serde_json::to_value(&value) {
                    self.set(path, json).await;
                }
            }
            None => {
                let row = ProgrammerValue {
                    id,
                    fixture_id,
                    parameter_kind: parameter.clone(),
                    value,
                    effect: None,
                    preset: None,
                    locked: false,
                };
                let path =
                    vec![PathSegment::Key("programmer_values".into()), PathSegment::Key("__create".into())];
                if let Ok(json) = serde_json::to_value(&row) {
                    self.set(path, json).await;
                }
            }
        }
    }

    async fn set(&self, path: Path, value: serde_json::Value) {
        let lifecycle = path_lifecycle(&path);
        if let Err(e) = self.engine.set(path.clone(), lifecycle, value).await {
            debug!("[wing] {path:?}: {e}");
        }
    }

    async fn read<T: serde::de::DeserializeOwned>(&self, table: &str) -> Vec<T> {
        self.engine
            .get(vec![PathSegment::Key(table.into())])
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }
}

fn entry_id(fixture_id: Uuid, parameter: &ParameterKind) -> Uuid {
    programmer_entry_id(&fixture_id.to_string(), &parameter_key(parameter)).parse().unwrap_or_default()
}

fn field(table: &[&str], id: Uuid, name: &str) -> Path {
    vec![PathSegment::Key(table[0].into()), PathSegment::Id(id), PathSegment::Key(name.into())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fader_keeps_a_colors_hue_through_black() {
        let red = ParameterValue::Color { r: 1.0, g: 0.0, b: 0.0, overrides: Default::default() };
        let black = from_fader(&red, 0.0, None);
        assert_eq!(hue_of(&black), None);
        let back = from_fader(&black, 0.5, hue_of(&red));
        assert_eq!(back, ParameterValue::Color { r: 0.5, g: 0.0, b: 0.0, overrides: Default::default() });
    }

    #[test]
    fn a_fader_writes_the_kind_it_moves() {
        assert_eq!(from_fader(&ParameterValue::Float(0.2), 0.75, None), ParameterValue::Float(0.75));
        assert_eq!(from_fader(&ParameterValue::Int(3), 1.0, None), ParameterValue::Int(255));
        assert_eq!(from_fader(&ParameterValue::Bool(false), 0.6, None), ParameterValue::Bool(true));
        assert_eq!(level_of(4096), 1.0);
        assert_eq!(level_of(9000), 1.0, "clamped");
    }
}
