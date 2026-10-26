//! The command wing, run by the station it hangs off.
//!
//! All of it: the faders, flash keys and motors, and since the gig the rest too — the
//! keypad and the command line, the encoders and their calculator, GO, CLEAR, OOPS,
//! STORE and UPDATE, learning, the digital inputs and the lights. These were a page's:
//! the station relayed every event to whichever browser drove the wing and that page
//! did the programming, so a page busy drawing the rig lagged every key behind it, and
//! with no page open the wing did nothing at all (found at the gig, 2026-10-25/26). Now
//! the events are acted on as they come off the USB thread, whether or not a page is
//! there.
//!
//! **What a page is still for** is what needs a screen. A page that claims the wing
//! shares its selection with it (`wing.context`) — the encoders turn what the operator
//! clicked, and a group recalled on the wing becomes the page's selection — shows the
//! command line, the encoder bar and the calculator from the LOCAL `wing` row, and
//! answers the asks on `wing_asks`: the Store dialog, a learned control waiting to be
//! bound, ESC and PLEASE for the dialog on top. Writes are the claiming page's user's,
//! so OOPS takes back what the wing did the way Ctrl-Z does on the page; with no page
//! claiming, they are the show's default operator's.
//!
//! Three rules carried over unchanged, because each was a bug once. **Only a fader a
//! hand is on is listened to** — its motor's own travel arrives exactly like a hand's,
//! and written back it would turn every cue into a programmer value. **A flash knows
//! which value it put there itself**, so a second tap before the first release has
//! landed does not take the key's own full for the operator's and put it back. And
//! **an encoder turns each target from its own value**, so a fan of levels stays a fan,
//! remembering what it last wrote for a moment because the programmer has not caught up
//! with a fast spin yet.

pub mod keys;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pult_schema::{
    path::{Path, PathSegment},
    registry::path_lifecycle,
    types::{
        cue::{Cue, FollowMode, ParameterCapture},
        fixture::{Fixture, FixtureType, ParameterKind, ParameterValue},
        group::{elements, evaluate, targets, Element, Group, ParameterTarget, SelectionClause, SelectionCombine, SelectionQuery, SelectionTerm},
        parameter_key,
        plugin::{PluginStatus, PluginsState},
        programmer::{programmer_entry_id, ProgrammerValue},
        scene::SceneObject,
        sequence::{now_ms, Sequence},
        surface::TileTarget,
        user::User,
        variable::Variable,
        wing::{
            BindingAction, ColorComponent, ControlBinding, StoreOnto, SurfaceControl, WingAsk, WingCalculator,
            WingContext, WingEncoders, WingSelection, WingSlot, COMMAND_WING,
        },
    },
};
use pult_wing::{Event, WingMap};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    engine::{EngineHandle, UpdateBroadcast},
    infra::{
        connectors::OutputHandle,
        plugins::PluginsHandle,
        wing::{Inbound, WingStation, ASKS_PATH},
    },
    wing::WingHandle,
};
use keys::{Encoder, FaderKey, Verb};

/// How often a moving fader's value is written: a frame of 40 Hz.
const WRITE_EVERY: Duration = Duration::from_millis(25);
/// How often the motors and the lights are brought up to date.
const FEEDBACK_EVERY: Duration = Duration::from_millis(50);
/// How often the lights' view of the show is read again: which sequences are up,
/// which timelines run. A fifth of a second late on a key's light is not felt.
const SHOW_EVERY: Duration = Duration::from_millis(200);
/// How often the bindings and the encoder bar are read again, beside being told.
const BINDINGS_EVERY: Duration = Duration::from_secs(2);
/// How long an encoder's own last value is trusted over the programmer's.
/// Where a master fader counts as down: below it, a sequence that is on goes off, and
/// leaving it upwards takes a sequence that is off. A fader at rest reads a count or two
/// above zero, so zero itself would never be crossed.
const FADER_BOTTOM: f32 = 0.01;

/// The fader a level moved by an encoder is written under: none, so it has no gesture.
const NO_FADER: u16 = u16::MAX;

const RECENT: Duration = Duration::from_millis(400);
/// How long a run of encoder clicks stays one gesture after the last of them: the
/// page's `nudging`, so a spin is one Ctrl-Z and a pause ends it.
const NUDGE_TAIL: Duration = Duration::from_millis(400);

/// The show as the lights read it, and nothing more of it than that.
#[derive(Default)]
struct Lit {
    sequences: Vec<SequenceLight>,
    timelines: Vec<TimelineLight>,
    variables: Vec<Variable>,
}

#[derive(Deserialize)]
struct SequenceLight {
    id: Uuid,
    #[serde(default)]
    active_cue_index: Option<usize>,
    #[serde(default)]
    went_at: Option<u64>,
    #[serde(default)]
    master: Option<f32>,
}

#[derive(Deserialize)]
struct TimelineLight {
    id: Uuid,
    #[serde(default)]
    running: bool,
}

struct Control {
    station: WingStation,
    wing: WingHandle,
    engine: EngineHandle,
    output: OutputHandle,
    plugins: PluginsHandle,
    broadcast: UpdateBroadcast,
    map: Option<Arc<WingMap>>,
    fader_keys: HashMap<u16, FaderKey>,
    leds: Vec<(usize, u16)>,
    bindings: Vec<ControlBinding>,
    lit: Lit,

    /// What the claiming page last said about itself.
    context: WingContext,
    /// The `rev` of the last selection made on the wing, which a page has to have seen
    /// before its own selection may replace it.
    wing_rev: u32,

    /// Programmer writes waiting for the next tick, newest value per entry, with the
    /// fader that moved it.
    pending: HashMap<Uuid, (u16, Uuid, ParameterKind, ParameterValue)>,
    /// Master levels waiting for the next tick, per sequence, with the fader.
    masters: HashMap<Uuid, (u16, f32)>,
    /// One gesture per fader a hand is on: touching a fader is where an act begins, and
    /// lifting off is where it ends — the one control that says so.
    fader_gestures: HashMap<u16, Uuid>,
    /// What each flash key held down will put back: the value it displaced, or none.
    flashed: HashMap<Uuid, Option<ParameterValue>>,
    /// Entries a flash put there itself and has not yet taken away.
    flash_owned: HashSet<Uuid>,
    /// The hue each color a fader moves had, last time it was not black.
    hues: HashMap<Uuid, (f32, f32, f32)>,
    /// What an encoder last turned each entry to, and when.
    recent: HashMap<Uuid, (ParameterValue, Instant)>,
    /// The gesture a run of encoder clicks is, and when it was last pushed on.
    nudge: Option<(Uuid, Instant)>,
    /// The keys held down right now, for lighting them while they are.
    held_keys: HashSet<u16>,
    /// The digital inputs as last read, which an edge is measured from.
    digital: Option<u8>,
    /// Where each master fader was last read, which leaving or reaching the bottom is
    /// measured from.
    fader_levels: HashMap<u16, f32>,
    /// After MOVE, the control whose binding is going somewhere, until the second one
    /// is touched.
    moving: Option<SurfaceControl>,
}

/// Run the wing for as long as this station runs.
pub async fn run(
    station: WingStation,
    wing: WingHandle,
    engine: EngineHandle,
    output: OutputHandle,
    plugins: PluginsHandle,
    broadcast: UpdateBroadcast,
) {
    let map = wing.map();
    let mut events = wing.events();
    let Some(mut inbound) = station.take_inbound() else {
        warn!("[wing] a second control loop for one wing; this one does nothing");
        return;
    };
    let mut control = Control {
        fader_keys: map.as_deref().map(keys::fader_keys).unwrap_or_default(),
        leds: map.as_deref().map(keys::led_keys).unwrap_or_default(),
        map,
        station,
        wing,
        engine,
        output,
        plugins,
        broadcast,
        bindings: Vec::new(),
        lit: Lit::default(),
        context: WingContext::default(),
        wing_rev: 0,
        pending: HashMap::new(),
        masters: HashMap::new(),
        fader_gestures: HashMap::new(),
        flashed: HashMap::new(),
        flash_owned: HashSet::new(),
        hues: HashMap::new(),
        recent: HashMap::new(),
        nudge: None,
        held_keys: HashSet::new(),
        digital: None,
        fader_levels: HashMap::new(),
        moving: None,
    };
    control.read_bindings().await;
    control.read_show().await;
    control.refresh_encoders().await;

    let interval = |every| {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick
    };
    let (mut write, mut feedback, mut show, mut bindings) =
        (interval(WRITE_EVERY), interval(FEEDBACK_EVERY), interval(SHOW_EVERY), interval(BINDINGS_EVERY));

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
            Some(message) = inbound.recv() => control.on_inbound(message).await,
            _ = write.tick() => control.write_pending().await,
            _ = feedback.tick() => control.feedback().await,
            _ = show.tick() => control.read_show().await,
            _ = bindings.tick() => {
                control.read_bindings().await;
                control.refresh_encoders().await;
            }
        }
    }
}

impl Control {
    // ── Reading the show ──────────────────────────────────────────────────────

    async fn read_bindings(&mut self) {
        let rows: Vec<ControlBinding> = self.read("control_bindings").await;
        self.bindings = rows.into_iter().filter(|b| b.surface == COMMAND_WING).collect();
    }

    /// What the lights and GO need of the show. A timeline row carries its clips, takes
    /// and beats, so the timelines are read only where a binding presses one; the same
    /// for the variables.
    async fn read_show(&mut self) {
        let presses = |want: fn(&TileTarget) -> bool| {
            self.bindings.iter().any(|b| matches!(&b.action, BindingAction::Press { target } if want(target)))
        };
        let timelines = presses(|t| matches!(t, TileTarget::Timeline { .. }));
        let variables = presses(|t| matches!(t, TileTarget::VariableSetter { .. } | TileTarget::VariableReadout { .. }));
        self.lit = Lit {
            sequences: self.read("sequences").await,
            timelines: if timelines { self.read("timelines").await } else { Vec::new() },
            variables: if variables { self.read("variables").await } else { Vec::new() },
        };
        let go = self.go_sequence();
        if self.station.operator().go_sequence != go {
            self.station.update_operator(|op| op.go_sequence = go).await;
        }
    }

    async fn read<T: serde::de::DeserializeOwned>(&self, table: &str) -> Vec<T> {
        let value = self.engine.get(vec![PathSegment::Key(table.into())]).await.ok();
        match value {
            Some(Value::Array(rows)) => rows.into_iter().filter_map(|row| serde_json::from_value(row).ok()).collect(),
            _ => Vec::new(),
        }
    }

    async fn read_one<T: serde::de::DeserializeOwned>(&self, table: &str, id: Uuid) -> Option<T> {
        let row = self.engine.get(vec![PathSegment::Key(table.into()), PathSegment::Id(id)]).await.ok()?;
        serde_json::from_value(row).ok()
    }

    /// What a control does on the page the wing is on.
    fn binding(&self, control: &SurfaceControl) -> Option<&BindingAction> {
        let page = self.station.operator().page;
        self.bindings.iter().find(|b| &b.control == control && b.page() == page).map(|b| &b.action)
    }

    /// The last page anything is bound on, which PAGE_UP may go one past: an empty
    /// page is where the next thing gets learned.
    fn last_page(&self) -> u32 {
        self.bindings.iter().map(ControlBinding::page).max().unwrap_or(1)
    }

    /// PAGE_UP and PAGE_DOWN: every control acts on another page of bindings. What the
    /// faders moved on this page goes out first, under the bindings it was moved under.
    async fn turn_binding_page(&mut self, by: i32) {
        let page = self.station.operator().page;
        let next = (page as i32 + by).clamp(1, self.last_page() as i32 + 1) as u32;
        if next == page {
            return;
        }
        self.write_pending().await;
        self.station.update_operator(|op| op.page = next).await;
        self.feedback().await;
    }

    fn code_of(&self, index: u16) -> Option<String> {
        self.map.as_ref()?.hardkeys.get(&index)?.code.clone()
    }

    // ── What the page says ────────────────────────────────────────────────────

    async fn on_inbound(&mut self, message: Inbound) {
        match message {
            Inbound::Context(context) => {
                let current = self.station.operator().selection;
                // A page that has not yet seen a selection made on the wing must not
                // put its older one back over it.
                let take_selection = context.seen_rev >= self.wing_rev
                    && (current.fixtures != context.fixtures || current.cells != context.cells);
                let selection = take_selection.then(|| WingSelection {
                    fixtures: context.fixtures.clone(),
                    cells: context.cells.clone(),
                    query: context.query.clone(),
                    rev: current.rev + 1,
                    from_wing: false,
                });
                self.context = context;
                let go = self.go_sequence();
                self.station
                    .update_operator(|op| {
                        if let Some(selection) = selection.clone() {
                            op.selection = selection;
                        }
                        op.go_sequence = go;
                    })
                    .await;
                if selection.is_some() {
                    self.refresh_encoders().await;
                }
            }
            Inbound::Released => {
                // The page has gone, and with it its cue sheet and its dialog. The
                // selection stays: it is what the wing is still acting on.
                self.context = WingContext { seen_rev: self.context.seen_rev, ..WingContext::default() };
                let go = self.go_sequence();
                self.station.update_operator(|op| op.go_sequence = go).await;
            }
            Inbound::Learn(on) => self.station.update_operator(|op| op.learning = on).await,
            Inbound::Calculator { slot, text, set } => {
                match slot {
                    None => self.station.update_operator(|op| op.calculator = None).await,
                    Some(slot) => {
                        let text = text.unwrap_or_default();
                        self.station
                            .update_operator(|op| op.calculator = Some(WingCalculator { slot, text: text.clone() }))
                            .await;
                        if set {
                            self.set_typed().await;
                        }
                    }
                }
            }
        }
    }

    /// Who the wing's writes are attributed to: the claiming page's user, or the
    /// show's default operator — never nobody, which could never be taken back.
    fn user(&self) -> Uuid {
        self.context.user_id.unwrap_or(User::DEFAULT_ID)
    }

    // ── Events ────────────────────────────────────────────────────────────────

    async fn on_event(&mut self, event: Event) {
        match event {
            Event::Fader { index, value } => self.on_fader(index as u16, value).await,
            Event::Key { index, down } => {
                if down {
                    self.held_keys.insert(index);
                } else {
                    self.held_keys.remove(&index);
                }
                self.on_key(index, down).await;
            }
            Event::Encoder { index, delta } => self.on_encoder(index, delta).await,
            Event::Digital { bits } => self.on_digital(bits).await,
        }
    }

    /// Learning takes the next control touched. Answers whether it took this one.
    async fn catch_learn(&mut self, control: SurfaceControl) -> bool {
        if !self.station.operator().learning {
            return false;
        }
        self.station.update_operator(|op| op.learning = false).await;
        self.ask(WingAsk::Learned { control });
        true
    }

    async fn on_fader(&mut self, index: u16, raw: u16) {
        // A fader is only learned by a hand on it, for the reason it is only listened
        // to under one: the motor moves it too.
        if !self.wing.touched(index as usize) {
            return;
        }
        if self.catch_learn(SurfaceControl::Fader { index }).await {
            return;
        }
        let Some(action) = self.binding(&SurfaceControl::Fader { index }).cloned() else { return };
        let level = keys::level_of(raw);
        match action {
            BindingAction::Parameter { fixture_id, parameter } => {
                let id = entry_id(fixture_id, &parameter);
                let like = self.current(fixture_id, &parameter).await;
                if let Some(hue) = hue_of(&like) {
                    self.hues.insert(id, hue);
                }
                let value = from_fader(&like, level, self.hues.get(&id).copied());
                self.pending.insert(id, (index, fixture_id, parameter, value));
            }
            BindingAction::Master { sequence_id } => {
                self.masters.insert(sequence_id, (index, level));
                let light = self.lit.sequences.iter().find(|s| s.id == sequence_id);
                let before = self
                    .fader_levels
                    .insert(index, level)
                    .unwrap_or_else(|| light.and_then(|s| s.master).unwrap_or(1.0));
                self.master_moved(sequence_id, before, level).await;
            }
            _ => {}
        }
    }

    async fn on_key(&mut self, index: u16, down: bool) {
        let code = self.code_of(index).unwrap_or_default();
        let fader_key = self.fader_keys.get(&index).copied();

        // A hand on a fader is a gesture beginning, and lifting off is it ending.
        if let Some(FaderKey::Touch(fader)) = fader_key {
            if down && self.edit_binding(SurfaceControl::Fader { index: fader }).await {
                return;
            }
            if down {
                self.fader_gestures.insert(fader, Uuid::new_v4());
            } else {
                // What the hand moved last goes out inside the gesture it belongs to.
                self.write_pending().await;
                self.fader_gestures.remove(&fader);
            }
            return;
        }

        // A dialog open on the claiming page is what the operator is looking at, so ESC
        // and PLEASE answer it rather than the command line behind it.
        let verb = keys::key_verb(&code);
        if down && self.context.dialog && matches!(verb, Some(Verb::Escape | Verb::Please)) {
            self.ask(WingAsk::Dialog { accept: verb == Some(Verb::Please) });
            return;
        }

        // A press belongs to the attribute under that encoder, which moves with the
        // selection and the page — so it is never learned or bound, and does only this.
        if let Some(slot) = keys::encoder_press(&code) {
            if down {
                self.open_calculator(slot as u8).await;
            }
            return;
        }
        if down && self.station.operator().calculator.is_some() && self.calculator_key(&code).await {
            return;
        }

        if verb == Some(Verb::Learn) {
            if down {
                self.station.update_operator(|op| op.learning = !op.learning).await;
            }
            return;
        }
        if down && self.catch_learn(SurfaceControl::Key { index }).await {
            return;
        }

        // After DELETE or MOVE, a key is the binding being deleted or moved — or, for
        // a fader's own key with nothing on it, the fader's.
        if down {
            let control = self.bound_control(index, fader_key);
            if self.edit_binding(control).await {
                return;
            }
        }

        // After STORE, a playback control is where the programmer goes: MA's Store onto
        // an executor.
        if self.station.operator().line.trim() == "store" {
            if let Some(onto) = self.store_onto(index, &code, fader_key) {
                if down {
                    self.station.update_operator(|op| op.line.clear()).await;
                    self.store_onto_control(onto).await;
                }
                return;
            }
        }

        if let Some(action) = self.binding(&SurfaceControl::Key { index }).cloned() {
            self.run(action, down).await;
            return;
        }

        // The keys of a fader that is a sequence's master are that sequence's Go and
        // Off.
        if let Some(FaderKey::Go(fader) | FaderKey::Off(fader)) = fader_key {
            if let Some(BindingAction::Master { sequence_id }) =
                self.binding(&SurfaceControl::Fader { index: fader }).cloned()
            {
                if !down {
                    return;
                }
                let verb = if matches!(fader_key, Some(FaderKey::Go(_))) { "goNext" } else { "off" };
                self.set(field("sequences", sequence_id, verb), json!({ "at": now_ms() }), None).await;
                return;
            }
        }

        if !down {
            return;
        }
        let line = self.station.operator().line;
        // UPDATE with nothing typed is the top bar's Update: one press, no target.
        if code == "UPDATE" && line.trim().is_empty() {
            self.update().await;
            return;
        }
        if let Some(word) = keys::keypad_word(&code) {
            self.station.update_operator(|op| op.line = keys::typed(&op.line, word)).await;
            return;
        }
        match verb {
            Some(Verb::Please) => self.please().await,
            Some(Verb::Clear) => self.clear_step().await,
            Some(Verb::Escape) => self.station.update_operator(|op| op.line.clear()).await,
            Some(Verb::Oops) => {
                if line.trim().is_empty() {
                    self.oops().await;
                } else {
                    self.station.update_operator(|op| op.line = keys::oops(&op.line)).await;
                }
            }
            Some(Verb::Go) => self.go(false).await,
            Some(Verb::GoBack) => self.go(true).await,
            Some(Verb::PageUp) => self.turn_page(-1).await,
            Some(Verb::PageDown) => self.turn_page(1).await,
            Some(Verb::NextPage) => self.turn_binding_page(1).await,
            Some(Verb::PrevPage) => self.turn_binding_page(-1).await,
            _ => {}
        }
    }

    /// A contact on the rear panel: a key with no LED and no meaning of MA's, so it
    /// does only what it is learned or bound to.
    async fn on_digital(&mut self, bits: u8) {
        let before = self.digital.replace(bits);
        // The first reading is where the contacts rest, not a press.
        let Some(before) = before else { return };
        for (index, down) in keys::digital_edges(before, bits) {
            let control = SurfaceControl::Digital { index };
            if down && self.catch_learn(control).await {
                continue;
            }
            if let Some(action) = self.binding(&control).cloned() {
                self.run(action, down).await;
            }
        }
    }

    async fn on_encoder(&mut self, index: u8, delta: i32) {
        if self.catch_learn(SurfaceControl::Encoder { index: index as u16 }).await {
            return;
        }
        if self.edit_binding(SurfaceControl::Encoder { index: index as u16 }).await {
            return;
        }
        if let Some(action) = self.binding(&SurfaceControl::Encoder { index: index as u16 }).cloned() {
            match action {
                BindingAction::Parameter { fixture_id, parameter } => {
                    let target = ParameterTarget { fixture_id, parameter_kind: parameter };
                    self.turn(vec![target], None, delta, keys::COARSE, None).await;
                }
                // A knob as a sequence's level: a hundred clicks from the bottom to full.
                BindingAction::Master { sequence_id } => {
                    let before = self
                        .masters
                        .get(&sequence_id)
                        .map(|(_, level)| *level)
                        .or_else(|| self.lit.sequences.iter().find(|s| s.id == sequence_id).map(|s| s.master.unwrap_or(1.0)))
                        .unwrap_or(1.0);
                    let level = (before + delta as f32 * 0.01).clamp(0.0, 1.0);
                    self.masters.insert(sequence_id, (NO_FADER, level));
                    self.master_moved(sequence_id, before, level).await;
                }
                _ => {}
            }
            return;
        }
        let Some(role) = self.map.as_deref().and_then(|map| keys::encoder_role(map, index)) else { return };
        match role {
            Encoder::DimmerWheel => {
                let targets = self.selection_targets(&ParameterKind::Intensity).await;
                self.turn(targets, Some(ParameterValue::Float(0.0)), delta, keys::WHEEL, None).await;
            }
            Encoder::Slot { slot, fine } => {
                let Some(Some(on)) = self.station.operator().encoders.slots.get(slot).cloned() else { return };
                let targets = self.selection_targets(&on.kind).await;
                let step = if fine { keys::FINE } else { keys::COARSE };
                self.turn(targets, Some(on.default_value.clone()), delta, step, on.component).await;
            }
        }
    }

    /// Up off the bottom takes a sequence that is off, and down onto it takes one that
    /// is on off: MA's fader that starts and stops its executor.
    async fn master_moved(&mut self, sequence_id: Uuid, before: f32, level: f32) {
        let on = self.lit.sequences.iter().any(|s| s.id == sequence_id && s.active_cue_index.is_some());
        if before <= FADER_BOTTOM && level > FADER_BOTTOM && !on {
            self.set(field("sequences", sequence_id, "goNext"), json!({ "at": now_ms() }), None).await;
        } else if before > FADER_BOTTOM && level <= FADER_BOTTOM && on {
            self.set(field("sequences", sequence_id, "off"), json!({ "at": now_ms() }), None).await;
        }
    }

    // ── DELETE and MOVE on a binding ──────────────────────────────────────────

    /// The control a key stands for when a binding is deleted or moved: the key, where
    /// something is bound to it, and otherwise the fader it belongs to, where that is.
    fn bound_control(&self, index: u16, fader_key: Option<FaderKey>) -> SurfaceControl {
        let key = SurfaceControl::Key { index };
        if self.binding(&key).is_some() {
            return key;
        }
        match fader_key {
            Some(FaderKey::Go(fader) | FaderKey::Off(fader)) if self.binding(&SurfaceControl::Fader { index: fader }).is_some() => {
                SurfaceControl::Fader { index: fader }
            }
            _ => key,
        }
    }

    /// The binding row a control has on this page.
    fn binding_row(&self, control: &SurfaceControl) -> Option<&ControlBinding> {
        let page = self.station.operator().page;
        self.bindings.iter().find(|b| &b.control == control && b.page() == page)
    }

    /// DELETE then a control unbinds it; MOVE then two controls moves the first one's
    /// binding onto the second, and swaps where the second had one. Only the binding:
    /// what it pointed at — a sequence, a group — stays in the show. Answers whether the
    /// control was taken for this, so it does nothing else.
    async fn edit_binding(&mut self, control: SurfaceControl) -> bool {
        let line = self.station.operator().line.trim().to_string();
        if line != "move" {
            self.moving = None;
        }
        match line.as_str() {
            "delete" => {
                self.station.update_operator(|op| op.line.clear()).await;
                let Some(row) = self.binding_row(&control).cloned() else {
                    self.toast("Nothing is bound to that on this page", false);
                    return true;
                };
                let gesture = Some(Uuid::new_v4());
                self.set(field("control_bindings", row.id, "__delete"), Value::Null, gesture).await;
                self.read_bindings().await;
                self.toast(&format!("{} unbound", self.control_label(&control)), true);
                true
            }
            "move" => {
                let Some(from) = self.moving.take() else {
                    if self.binding_row(&control).is_none() {
                        self.station.update_operator(|op| op.line.clear()).await;
                        self.toast("Nothing is bound to that on this page to move", false);
                    } else {
                        self.moving = Some(control);
                    }
                    return true;
                };
                self.station.update_operator(|op| op.line.clear()).await;
                // A binding moved from a fader onto an executor's key goes to that key's
                // fader: a sequence's level cannot be a key.
                let to = match (from, control) {
                    (SurfaceControl::Fader { .. }, SurfaceControl::Key { index }) => match self.fader_keys.get(&index) {
                        Some(FaderKey::Go(fader) | FaderKey::Off(fader)) => SurfaceControl::Fader { index: *fader },
                        _ => control,
                    },
                    _ => control,
                };
                if to == from {
                    return true;
                }
                if std::mem::discriminant(&from) != std::mem::discriminant(&to) {
                    self.toast("A binding moves onto a control of the same kind: a key to a key, a fader to a fader", false);
                    return true;
                }
                let Some(row) = self.binding_row(&from).cloned() else { return true };
                let gesture = Some(Uuid::new_v4());
                if let Some(there) = self.binding_row(&to).cloned() {
                    self.set(field("control_bindings", there.id, "control"), json!(from), gesture).await;
                }
                self.set(field("control_bindings", row.id, "control"), json!(to), gesture).await;
                self.read_bindings().await;
                self.toast(&format!("Moved to {}", self.control_label(&to)), true);
                true
            }
            _ => false,
        }
    }

    /// What MA calls a control, for a toast.
    fn control_label(&self, control: &SurfaceControl) -> String {
        let map = self.map.as_deref();
        let label = match control {
            SurfaceControl::Key { index } => map.and_then(|m| m.hardkeys.get(index)).map(|c| c.label.clone()),
            SurfaceControl::Fader { index } => map.and_then(|m| m.faders.get(&(*index as u8))).map(|c| c.label.clone()),
            SurfaceControl::Encoder { index } => map.and_then(|m| m.encoders.get(&(*index as u8))).map(|c| c.label.clone()),
            SurfaceControl::Digital { index } => Some(format!("digital in {}", index + 1)),
        };
        label.unwrap_or_else(|| "That control".into())
    }

    // ── A binding, run ────────────────────────────────────────────────────────

    async fn run(&mut self, action: BindingAction, down: bool) {
        match action {
            BindingAction::Flash { fixture_id, parameter } => self.flash(fixture_id, parameter, down).await,
            BindingAction::Press { target } if down => self.press(target).await,
            BindingAction::CommandLine { line } if down => {
                self.station.update_operator(|op| op.line = line.clone()).await;
                self.please().await;
            }
            BindingAction::SelectGroup { group_id } if down => {
                match self.read_one::<Group>("groups", group_id).await {
                    Some(group) => self.select(group.query, Vec::new()).await,
                    None => self.toast("That group is no longer in the show.", false),
                }
            }
            _ => {}
        }
    }

    /// What a tile presses, pressed — the same acts `surface.ts`'s `pressTarget` makes.
    async fn press(&mut self, target: TileTarget) {
        let at = json!({ "at": now_ms() });
        match target {
            TileTarget::FlowButton { node_id } => {
                self.set(field("flow_nodes", node_id, "press"), json!({}), None).await;
            }
            TileTarget::VariableSetter { variable_id, op } => {
                if let Err(e) = self.engine.set_variable(variable_id, op).await {
                    self.toast(&format!("{e}"), false);
                }
            }
            TileTarget::Timeline { timeline_id, from_start } => {
                let running = self.lit.timelines.iter().any(|t| t.id == timeline_id && t.running);
                if running {
                    self.set(field("timelines", timeline_id, "stop"), at, None).await;
                } else if from_start {
                    self.set(field("timelines", timeline_id, "play"), json!({ "at": now_ms(), "fromMs": 0 }), None).await;
                } else {
                    self.set(field("timelines", timeline_id, "play"), at, None).await;
                }
            }
            TileTarget::SequenceGo { sequence_id } => {
                self.set(field("sequences", sequence_id, "goNext"), at, None).await;
            }
            TileTarget::SequenceOff { sequence_id } => {
                self.set(field("sequences", sequence_id, "off"), at, None).await;
            }
            _ => {}
        }
    }

    /// Full while held, and on release whatever was there before — or nothing.
    ///
    /// Unattributed, deliberately: a flash puts back what it found, and a press landing
    /// in the operator's history would be what OOPS takes back instead of their edit.
    async fn flash(&mut self, fixture_id: Uuid, parameter: ParameterKind, down: bool) {
        let id = entry_id(fixture_id, &parameter);
        // A press that lands before writes still waiting would be overtaken by them.
        self.write_pending().await;
        if down {
            let before = if self.flash_owned.contains(&id) { None } else { self.held(id).await.map(|e| e.value) };
            if before.is_none() {
                self.flash_owned.insert(id);
            }
            self.flashed.insert(id, before);
            let like = self.current(fixture_id, &parameter).await;
            let existing = self.held(id).await;
            self.write_value(fixture_id, &parameter, full(&like), existing, Write::Unattributed).await;
        } else {
            match self.flashed.remove(&id) {
                // A release with no press behind it — the press was what learning
                // caught — lets go of nothing.
                None => {}
                Some(Some(before)) => {
                    let existing = self.held(id).await;
                    self.write_value(fixture_id, &parameter, before, existing, Write::Unattributed).await;
                }
                Some(None) => {
                    self.delete_entry(id, Write::Unattributed).await;
                    self.flash_owned.remove(&id);
                }
            }
        }
    }

    // ── The command line ──────────────────────────────────────────────────────

    async fn please(&mut self) {
        let line = self.station.operator().line.trim().to_string();
        if line.is_empty() {
            return;
        }
        self.station.update_operator(|op| op.line.clear()).await;
        match line.as_str() {
            // STORE and PLEASE is the Store dialog, which is a page's to show.
            "store" => {
                if self.station.claimed() {
                    self.ask(WingAsk::Store { preselect: Vec::new(), onto: None });
                } else {
                    self.toast("Store onto a master's key, or type where: store sequence 1 cue 4", false);
                }
            }
            "update" => self.update().await,
            _ => self.run_line(&line).await,
        }
    }

    /// A line, run through the command-line plugin the way the panel runs one, with
    /// the wing's selection as its context.
    async fn run_line(&mut self, line: &str) {
        let Some(plugin) = self.command_line_plugin().await else {
            self.toast("The wing types into the command line, and no command-line plugin is running", false);
            return;
        };
        let selection = self.station.operator().selection;
        let args = json!({
            "payload": { "line": line },
            "ctx": { "selection": selection.fixtures, "cells": selection.cells, "userId": self.user() },
        });
        match self.plugins.call(plugin, "surface.exec".into(), args).await {
            Ok(result) => {
                self.apply_effects(&result).await;
                if let Some(message) = result.pointer("/error/message").and_then(Value::as_str) {
                    self.toast(&format!("{line}: {message}"), false);
                } else {
                    let said: Vec<&str> = result
                        .get("lines")
                        .and_then(Value::as_array)
                        .map(|lines| lines.iter().filter_map(|l| l.get("text").and_then(Value::as_str)).collect())
                        .unwrap_or_default();
                    if !said.is_empty() {
                        self.toast(&said.join(" · "), true);
                    }
                }
            }
            Err(e) => self.toast(&format!("{line}: {e}"), false),
        }
    }

    /// The plugin the page would type into: the first running one with a surface.
    async fn command_line_plugin(&self) -> Option<String> {
        let state: PluginsState = self
            .engine
            .get(vec![PathSegment::Key("plugins".into())])
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())?;
        state
            .plugins
            .into_iter()
            .filter(|p| !matches!(p.status, PluginStatus::Failed(_)))
            .find(|p| p.surfaces.iter().any(|s| s.kind != "bar"))
            .map(|p| p.id)
    }

    /// What a command line asked to be selected: a group's question, or a list.
    async fn apply_effects(&mut self, result: &Value) {
        let Some(asked) = result.pointer("/effects/selection") else { return };
        if let Some(query) = asked.get("query").and_then(|q| serde_json::from_value::<SelectionQuery>(q.clone()).ok()) {
            self.select(query, Vec::new()).await;
        } else if let Some(ids) = asked.get("fixtureIds").and_then(|v| serde_json::from_value::<Vec<Uuid>>(v.clone()).ok()) {
            let cells = asked.get("cells").and_then(|c| serde_json::from_value(c.clone()).ok()).unwrap_or_default();
            self.select(ids_query(ids), cells).await;
        }
    }

    // ── The selection ─────────────────────────────────────────────────────────

    /// Select on the wing: a group key, a typed `fixture 3`. The claiming page takes
    /// this on as its own selection.
    async fn select(&mut self, mut query: SelectionQuery, cells: Vec<pult_schema::types::group::CellSpan>) {
        if !cells.is_empty() {
            query.cells = cells;
        }
        let fixtures: Vec<Fixture> = self.read("fixtures").await;
        let objects: Vec<SceneObject> = self.read("scene_objects").await;
        let ids = evaluate(&query, &fixtures, None, &objects);
        let rev = self.station.operator().selection.rev + 1;
        self.wing_rev = rev;
        let selection = WingSelection { fixtures: ids, cells: query.cells.clone(), query: Some(query), rev, from_wing: true };
        self.station.update_operator(|op| op.selection = selection.clone()).await;
        self.refresh_encoders().await;
    }

    /// Where a value of `kind` set on the wing's selection lands: the schema's rule,
    /// the one `selection.targets` answers and the Programmer follows.
    async fn selection_targets(&self, kind: &ParameterKind) -> Vec<ParameterTarget> {
        let selection = self.station.operator().selection;
        if selection.fixtures.is_empty() {
            return Vec::new();
        }
        let fixtures: Vec<Fixture> = self.read("fixtures").await;
        let types: HashMap<Uuid, FixtureType> =
            self.read::<FixtureType>("fixture_types").await.into_iter().map(|t| (t.id, t)).collect();
        let type_of: HashMap<Uuid, &FixtureType> =
            fixtures.iter().filter_map(|f| types.get(&f.fixture_type_id).map(|t| (f.id, t))).collect();
        let address: HashMap<Uuid, &pult_schema::types::fixture::FixtureAddress> =
            fixtures.iter().map(|f| (f.id, &f.address)).collect();
        // Cut, the cells it picks; not cut, each fixture whole — the page's
        // `targetElements` and the station's `selection.targets` alike.
        let picked: Vec<Element> = if selection.cells.is_empty() {
            selection.fixtures.iter().map(|id| Element { fixture: *id, cell: None }).collect()
        } else {
            elements(&selection.fixtures, &selection.cells, |id| {
                type_of.get(id).filter(|t| t.has_cells()).map_or(0, |t| t.cells)
            })
        };
        targets(
            kind,
            &picked,
            |id| type_of.get(id).copied(),
            |id, key| match (type_of.get(id), address.get(id)) {
                (Some(t), Some(a)) => t.reaches(a, key),
                _ => true,
            },
        )
    }

    /// The encoder bar, worked out again from the selection and the rig.
    async fn refresh_encoders(&mut self) {
        let selection = self.station.operator().selection;
        let fixtures: Vec<Fixture> = self.read("fixtures").await;
        let types: HashMap<Uuid, FixtureType> =
            self.read::<FixtureType>("fixture_types").await.into_iter().map(|t| (t.id, t)).collect();
        let type_of: HashMap<Uuid, &FixtureType> =
            fixtures.iter().filter_map(|f| types.get(&f.fixture_type_id).map(|t| (f.id, t))).collect();
        let all = keys::encoder_slots(selection.fixtures.iter().copied(), |id| type_of.get(id).copied());
        let pages = keys::page_count(all.len());
        let page = (self.station.operator().encoders.page as usize).min(pages - 1);
        let slots: Vec<Option<WingSlot>> =
            (0..keys::ENCODERS).map(|i| all.get(page * keys::ENCODERS + i).cloned()).collect();
        let encoders = WingEncoders { page: page as u32, pages: pages as u32, slots };
        if self.station.operator().encoders != encoders {
            self.station.update_operator(|op| op.encoders = encoders.clone()).await;
        }
    }

    /// A page turn moves every encoder to another attribute, the open one included.
    async fn turn_page(&mut self, by: i32) {
        let encoders = self.station.operator().encoders;
        let page = (encoders.page as i32 + by).clamp(0, encoders.pages as i32 - 1) as u32;
        self.station
            .update_operator(|op| {
                op.calculator = None;
                op.encoders.page = page;
            })
            .await;
        self.refresh_encoders().await;
    }

    // ── Encoders and the calculator ───────────────────────────────────────────

    /// Turn each target from its own value, so a fan of levels stays a fan — the
    /// relative move every desk's encoder makes. One gesture for a run of clicks.
    async fn turn(
        &mut self,
        targets: Vec<ParameterTarget>,
        fallback: Option<ParameterValue>,
        clicks: i32,
        step: f32,
        component: Option<ColorComponent>,
    ) {
        if targets.is_empty() {
            return;
        }
        let now = Instant::now();
        let gesture = match self.nudge {
            Some((id, at)) if now.duration_since(at) < NUDGE_TAIL => id,
            _ => Uuid::new_v4(),
        };
        self.nudge = Some((gesture, now));
        let held = self.programmer().await;
        let showing = self
            .output
            .values(targets.iter().map(|t| (t.fixture_id, parameter_key(&t.parameter_kind))).collect())
            .await;
        for (target, shown) in targets.into_iter().zip(showing) {
            let id = entry_id(target.fixture_id, &target.parameter_kind);
            let existing = held.get(&id).cloned();
            let from = match self.recent.get(&id) {
                Some((value, at)) if now.duration_since(*at) < RECENT => Some(value.clone()),
                _ => None,
            }
            .or_else(|| existing.as_ref().map(|e| e.value.clone()))
            .or(shown)
            .or_else(|| fallback.clone());
            let Some(from) = from else { continue };
            let next = keys::turned(&from, clicks, step, component);
            self.recent.insert(id, (next.clone(), now));
            self.write_value(target.fixture_id, &target.parameter_kind, next, existing, Write::In(gesture)).await;
        }
    }

    /// Open the calculator on this encoder, or close it if it is the one open.
    async fn open_calculator(&mut self, slot: u8) {
        let operator = self.station.operator();
        let open = operator.calculator.as_ref().is_some_and(|c| c.slot == slot);
        let there = operator.encoders.slots.get(slot as usize).is_some_and(Option::is_some);
        self.station
            .update_operator(|op| {
                op.calculator = (!open && there).then(|| WingCalculator { slot, text: String::new() });
            })
            .await;
    }

    /// A key while the calculator is open. Answers whether it was the calculator's:
    /// digits and the dot type, CLEAR empties, OOPS takes one back, ESC closes and
    /// PLEASE sets. The keypad's other words are swallowed rather than typed into the
    /// command line behind it; every other key does what it always does.
    async fn calculator_key(&mut self, code: &str) -> bool {
        if let Some(word) = keys::keypad_word(code) {
            self.station
                .update_operator(|op| {
                    if let Some(c) = op.calculator.as_mut() {
                        c.text = keys::calculator_typed(&c.text, word);
                    }
                })
                .await;
            return true;
        }
        match keys::key_verb(code) {
            Some(Verb::Clear) => {
                self.station.update_operator(|op| op.calculator.iter_mut().for_each(|c| c.text.clear())).await;
            }
            Some(Verb::Oops) => {
                self.station
                    .update_operator(|op| op.calculator.iter_mut().for_each(|c| {
                        c.text.pop();
                    }))
                    .await;
            }
            Some(Verb::Escape) => self.station.update_operator(|op| op.calculator = None).await,
            Some(Verb::Please) => self.set_typed().await,
            _ => return false,
        }
        true
    }

    /// Set what the calculator holds on every target of the selection, as one gesture
    /// — the same value for each, since typing a number is saying where, not how far.
    async fn set_typed(&mut self) {
        let operator = self.station.operator();
        let Some(open) = operator.calculator.clone() else { return };
        self.station.update_operator(|op| op.calculator = None).await;
        let Some(Some(slot)) = operator.encoders.slots.get(open.slot as usize).cloned() else { return };
        let targets = self.selection_targets(&slot.kind).await;
        let held = self.programmer().await;
        let showing = self
            .output
            .values(targets.iter().map(|t| (t.fixture_id, parameter_key(&t.parameter_kind))).collect())
            .await;
        let mut writes = Vec::new();
        for (target, shown) in targets.into_iter().zip(showing) {
            let id = entry_id(target.fixture_id, &target.parameter_kind);
            let existing = held.get(&id).cloned();
            let like = existing.as_ref().map(|e| e.value.clone()).or(shown).unwrap_or_else(|| slot.default_value.clone());
            let Some(value) = keys::typed_value(&open.text, &like, slot.component) else {
                self.toast(&format!("Nothing to set from \"{}\"", open.text), false);
                return;
            };
            writes.push((target, value, existing));
        }
        let gesture = Uuid::new_v4();
        for (target, value, existing) in writes {
            self.write_value(target.fixture_id, &target.parameter_kind, value, existing, Write::In(gesture)).await;
        }
    }

    // ── GO, CLEAR, OOPS ───────────────────────────────────────────────────────

    /// The sequence GO acts on: the claiming page's cue sheet touched last, else the
    /// one that went most recently from anywhere.
    fn go_sequence(&self) -> Option<Uuid> {
        if let Some(focused) = self.context.focused_sequence {
            if self.lit.sequences.iter().any(|s| s.id == focused) {
                return Some(focused);
            }
        }
        self.lit.sequences.iter().filter(|s| s.went_at.is_some()).max_by_key(|s| s.went_at).map(|s| s.id)
    }

    async fn go(&mut self, back: bool) {
        let Some(sequence) = self.go_sequence() else {
            self.toast("Go is on a cue sheet, and no sequence has gone yet: click one first", false);
            return;
        };
        let verb = if back { "goBack" } else { "goNext" };
        self.set(field("sequences", sequence, verb), json!({ "at": now_ms() }), None).await;
    }

    /// CLEAR, the way MA's works: a line being typed is emptied first, then the
    /// selection, then the programmer — one per press, the steps the page's button and
    /// Esc take (`stores/clear.ts`). A locked value is parked on purpose and stays.
    async fn clear_step(&mut self) {
        let operator = self.station.operator();
        if !operator.line.trim().is_empty() {
            self.station.update_operator(|op| op.line.clear()).await;
            return;
        }
        if !operator.selection.fixtures.is_empty() {
            self.select(SelectionQuery::empty(), Vec::new()).await;
            return;
        }
        self.clear_programmer(Uuid::new_v4()).await;
    }

    /// Everything unlocked out of the programmer, inside `gesture`.
    async fn clear_programmer(&mut self, gesture: Uuid) {
        self.pending.clear();
        let held = self.programmer().await;
        for entry in held.values().filter(|e| !e.locked) {
            self.delete_entry(entry.id, Write::In(gesture)).await;
        }
    }

    async fn oops(&mut self) {
        let undone = self.engine.undo(self.user(), false).await;
        match undone.len() {
            0 => self.toast("Nothing to take back.", false),
            1 => {}
            n => self.toast(&format!("Took back {n} changes."), true),
        }
    }

    // ── STORE and UPDATE, the top bar's ───────────────────────────────────────

    /// Update: every held value into the cue that is driving it now, one gesture over
    /// however many cues it touches, and the programmer cleared when everything found a
    /// home — the top bar's Update (`stores/programmer.ts`'s `updateDriven`). Keys
    /// nothing is driving are handed to the claiming page's Store dialog, ticked.
    async fn update(&mut self) {
        self.write_pending().await;
        let held = self.programmer().await;
        if held.is_empty() {
            self.toast("The programmer is empty: nothing to update", false);
            return;
        }
        // A cue loaded for editing is where Update goes, as the page's Update does:
        // left out, the wing merged into the driving cue, cleared the programmer and
        // left the edit open — and the page's Update after it then wrote the empty
        // programmer over the whole cue (HHG, 2026-10-26).
        let editing = self
            .engine
            .get(vec![PathSegment::Key("show".into()), PathSegment::Key("editing_cue".into())])
            .await
            .ok()
            .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
        if let Some(cue_id) = editing {
            self.update_edit(cue_id, &held).await;
            return;
        }
        let sequences: Vec<Sequence> = self.read("sequences").await;
        let cues: Vec<Cue> = self.read("cues").await;
        let driving = driving_cues(&sequences, &cues);
        let mut per_cue: Vec<(Uuid, Vec<&ProgrammerValue>)> = Vec::new();
        let mut orphans = Vec::new();
        let mut entries: Vec<&ProgrammerValue> = held.values().collect();
        entries.sort_by_key(|e| e.id);
        for entry in entries {
            let key = (entry.fixture_id, parameter_key(&entry.parameter_kind));
            match driving.get(&key) {
                Some(cue) => match per_cue.iter_mut().find(|(id, _)| id == cue) {
                    Some((_, list)) => list.push(entry),
                    None => per_cue.push((*cue, vec![entry])),
                },
                None => orphans.push(entry.id),
            }
        }
        let gesture = Uuid::new_v4();
        for (cue_id, stored) in &per_cue {
            let Some(cue) = cues.iter().find(|c| c.id == *cue_id) else { continue };
            let captures = store_captures(&cue.captures, stored);
            if let Ok(value) = serde_json::to_value(&captures) {
                self.set(field("cues", *cue_id, "captures"), value, Some(gesture)).await;
            }
        }
        let updated = per_cue.len();
        let cues_word = |n: usize| if n == 1 { "cue" } else { "cues" };
        if orphans.is_empty() {
            self.clear_programmer(gesture).await;
            self.toast(&format!("Updated {updated} {}", cues_word(updated)), true);
            return;
        }
        if updated > 0 {
            self.toast(&format!("Updated {updated} {} — {} left to store", cues_word(updated), orphans.len()), true);
        } else {
            self.toast("Nothing here is being driven by a cue", false);
        }
        if self.station.claimed() {
            self.ask(WingAsk::Store { preselect: orphans, onto: None });
        }
    }

    /// Update while a cue is loaded for editing: the cue becomes exactly what the
    /// programmer holds — `programmer.ts`'s `updateEdit`, timing kept — the programmer
    /// is cleared and the edit ends, one gesture.
    async fn update_edit(&mut self, cue_id: Uuid, held: &HashMap<Uuid, ProgrammerValue>) {
        let Some(cue) = self.read_one::<Cue>("cues", cue_id).await else {
            self.toast("The cue being edited is no longer in the show", false);
            return;
        };
        let mut entries: Vec<&ProgrammerValue> = held.values().collect();
        entries.sort_by_key(|e| e.id);
        let keep: HashSet<(Uuid, String)> = entries.iter().map(|e| (e.fixture_id, parameter_key(&e.parameter_kind))).collect();
        let captures: Vec<ParameterCapture> = store_captures(&cue.captures, &entries)
            .into_iter()
            .filter(|c| keep.contains(&(c.fixture_id, parameter_key(&c.parameter_kind))))
            .collect();
        let gesture = Uuid::new_v4();
        let Ok(value) = serde_json::to_value(&captures) else { return };
        self.set(field("cues", cue_id, "captures"), value, Some(gesture)).await;
        self.clear_programmer(gesture).await;
        self.set(vec![PathSegment::Key("show".into()), PathSegment::Key("editing_cue".into())], Value::Null, Some(gesture)).await;
        self.toast(&format!("Updated \"{}\"", cue.name), true);
    }

    /// Which playback a key is, for STORE: a fader's executor key is its fader — a
    /// sequence's master or nothing yet — and an X key or a key that goes a sequence is
    /// itself. `None` for any other key, which STORE leaves to do what it does.
    fn store_onto(&self, index: u16, code: &str, fader_key: Option<FaderKey>) -> Option<StoreOnto> {
        let page = self.station.operator().page;
        // The key under a fader is that fader's playback. The key above it is one of
        // its own, like an X key: it is the fader's Off only while nothing is on it.
        if let Some(FaderKey::Go(fader)) = fader_key {
            let control = SurfaceControl::Fader { index: fader };
            return match self.binding(&control) {
                Some(BindingAction::Master { sequence_id }) => {
                    Some(StoreOnto { control, page, sequence_id: Some(*sequence_id) })
                }
                None => Some(StoreOnto { control, page, sequence_id: None }),
                // A fader moving a parameter is not a playback.
                Some(_) => None,
            };
        }
        let control = SurfaceControl::Key { index };
        match self.binding(&control) {
            Some(BindingAction::Press {
                target: TileTarget::SequenceGo { sequence_id } | TileTarget::SequenceOff { sequence_id },
            }) => Some(StoreOnto { control, page, sequence_id: Some(*sequence_id) }),
            None if keys::playback_key(code) => {
                Some(StoreOnto { control, page, sequence_id: None })
            }
            _ => None,
        }
    }

    /// STORE onto a playback: the Store dialog on the claiming page, opened on that
    /// playback's sequence or a new one. With no page there is no dialog, so a sequence
    /// gets a new cue straight away and a control with none is refused.
    async fn store_onto_control(&mut self, onto: StoreOnto) {
        if self.station.claimed() {
            self.write_pending().await;
            self.ask(WingAsk::Store { preselect: Vec::new(), onto: Some(onto) });
            return;
        }
        match onto.sequence_id {
            Some(sequence_id) => self.store_into(sequence_id).await,
            None => self.toast("Nothing is on that yet: a new sequence needs the Store dialog, on a page showing the wing", false),
        }
    }

    /// STORE onto a master's key: the programmer as a new cue at the end of that
    /// sequence, with the cue fades a new cue gets in the Store dialog, and the
    /// programmer cleared — one gesture.
    async fn store_into(&mut self, sequence_id: Uuid) {
        self.write_pending().await;
        let held = self.programmer().await;
        if held.is_empty() {
            self.toast("The programmer is empty: nothing to store", false);
            return;
        }
        let Some(sequence) = self.read_one::<Sequence>("sequences", sequence_id).await else { return };
        let mut entries: Vec<&ProgrammerValue> = held.values().collect();
        entries.sort_by_key(|e| e.id);
        let number = (sequence.cue_ids.len() + 1) as f64;
        let cue = Cue {
            id: Uuid::new_v4(),
            name: format!("Cue {}", sequence.cue_ids.len() + 1),
            number,
            captures: store_captures(&[], &entries),
            follow_mode: FollowMode::Manual,
            fade_in_ms: DEFAULT_FADE_MS,
            fade_out_ms: DEFAULT_FADE_MS,
            easing: None,
            is_active: false,
        };
        let gesture = Uuid::new_v4();
        let Ok(row) = serde_json::to_value(&cue) else { return };
        self.set(vec![PathSegment::Key("cues".into()), PathSegment::Key("__create".into())], row, Some(gesture)).await;
        let mut ids = sequence.cue_ids.clone();
        ids.push(cue.id);
        self.set(field("sequences", sequence_id, "cue_ids"), json!(ids), Some(gesture)).await;
        self.clear_programmer(gesture).await;
        self.toast(&format!("Stored as \"{}\" in \"{}\"", cue.name, sequence.name), true);
    }

    // ── Writing ───────────────────────────────────────────────────────────────

    /// Write what the faders moved since the last tick, each in its fader's gesture.
    async fn write_pending(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        if !pending.is_empty() {
            let held = self.programmer().await;
            for (id, (fader, fixture_id, parameter, value)) in pending {
                let gesture = self.fader_gestures.get(&fader).copied();
                let how = gesture.map_or(Write::Attributed, Write::In);
                self.write_value(fixture_id, &parameter, value, held.get(&id).cloned(), how).await;
            }
        }
        for (sequence_id, (fader, level)) in std::mem::take(&mut self.masters) {
            // Full is written as no level at all, which is what a show opens at.
            let value = if level >= 0.999 { Value::Null } else { json!(level) };
            let gesture = self.fader_gestures.get(&fader).copied();
            self.set(field("sequences", sequence_id, "master"), value, gesture).await;
        }
    }

    /// One programmer value, the way a page writes one: a field write to an entry that
    /// is there, the whole row where it carried a preset (moving a value breaks the
    /// link, in one write), and a create where there is none.
    async fn write_value(
        &self,
        fixture_id: Uuid,
        parameter: &ParameterKind,
        value: ParameterValue,
        existing: Option<ProgrammerValue>,
        how: Write,
    ) {
        let id = entry_id(fixture_id, parameter);
        let table = || PathSegment::Key("programmer_values".into());
        match existing {
            Some(entry) if entry.value == value => {}
            Some(entry) if entry.preset.is_some() => {
                let row = ProgrammerValue { value, preset: None, ..entry };
                if let Ok(json) = serde_json::to_value(&row) {
                    self.write(vec![table(), PathSegment::Id(id)], json, how).await;
                }
            }
            Some(_) => {
                if let Ok(json) = serde_json::to_value(&value) {
                    self.write(field("programmer_values", id, "value"), json, how).await;
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
                if let Ok(json) = serde_json::to_value(&row) {
                    self.write(vec![table(), PathSegment::Key("__create".into())], json, how).await;
                }
            }
        }
    }

    async fn delete_entry(&self, id: Uuid, how: Write) {
        self.write(field("programmer_values", id, "__delete"), Value::Null, how).await;
    }

    async fn write(&self, path: Path, value: Value, how: Write) {
        match how {
            Write::Unattributed => {
                let lifecycle = path_lifecycle(&path);
                if let Err(e) = self.engine.set(path.clone(), lifecycle, value).await {
                    debug!("[wing] {path:?}: {e}");
                }
            }
            Write::Attributed => self.set(path, value, None).await,
            Write::In(gesture) => self.set(path, value, Some(gesture)).await,
        }
    }

    /// A write as the wing's operator, so it can be taken back.
    async fn set(&self, path: Path, value: Value, gesture: Option<Uuid>) {
        let lifecycle = path_lifecycle(&path);
        if let Err(e) = self.engine.set_as(self.user(), gesture, path.clone(), lifecycle, value).await {
            debug!("[wing] {path:?}: {e}");
            self.toast(&format!("{e}"), false);
        }
    }

    // ── Reading what a parameter is doing ─────────────────────────────────────

    /// The programmer, by entry id.
    async fn programmer(&self) -> HashMap<Uuid, ProgrammerValue> {
        self.read::<ProgrammerValue>("programmer_values").await.into_iter().map(|e| (e.id, e)).collect()
    }

    async fn held(&self, id: Uuid) -> Option<ProgrammerValue> {
        self.read_one("programmer_values", id).await
    }

    /// What a parameter is now, to shape the value a fader or a flash writes: the
    /// programmer's entry, else what the rig is doing, else the type's own default.
    async fn current(&self, fixture_id: Uuid, parameter: &ParameterKind) -> ParameterValue {
        if let Some(held) = self.held(entry_id(fixture_id, parameter)).await {
            return held.value;
        }
        let key = parameter_key(parameter);
        if let Some(Some(value)) = self.output.values(vec![(fixture_id, key.clone())]).await.into_iter().next() {
            return value;
        }
        let fixture: Option<Fixture> = self.read_one("fixtures", fixture_id).await;
        let types: Vec<FixtureType> = self.read("fixture_types").await;
        fixture
            .and_then(|f| types.into_iter().find(|t| t.id == f.fixture_type_id))
            .and_then(|t| t.parameter_for_key(&key).map(|p| p.default_value.clone()))
            .unwrap_or(ParameterValue::Float(0.0))
    }

    // ── The desk, lit and moved ───────────────────────────────────────────────

    async fn feedback(&mut self) {
        self.drive_motors().await;
        self.drive_leds();
    }

    /// Put every bound fader where the show is, and let every other one go. A touched
    /// fader is left to the hand by `WingHandle::drive` itself.
    async fn drive_motors(&mut self) {
        let page = self.station.operator().page;
        let faders: Vec<(u16, BindingAction)> = self
            .bindings
            .iter()
            .filter(|b| b.page() == page)
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
        let mut positions: HashMap<usize, u16> = HashMap::new();
        for (index, action) in &faders {
            let level = match action {
                BindingAction::Parameter { .. } => values.next().flatten().as_ref().and_then(to_fader),
                BindingAction::Master { sequence_id } => {
                    self.lit.sequences.iter().find(|s| s.id == *sequence_id).map(|s| s.master.unwrap_or(1.0))
                }
                _ => None,
            };
            if let Some(level) = level {
                positions.insert(*index as usize, (level.clamp(0.0, 1.0) * keys::FADER_FULL).round() as u16);
            }
        }
        // An executor fader nothing on this page is bound to goes to the bottom, so a
        // page turned to reads as empty rather than showing the last page's levels.
        // The rest — the crossfaders — are only ever let go.
        let executors: HashSet<usize> = self
            .map
            .as_deref()
            .map(|map| map.faders.iter().filter(|(_, c)| c.executor.is_some()).map(|(i, _)| *i as usize).collect())
            .unwrap_or_default();
        self.wing.drive(|out| {
            for index in 0..out.faders.len() {
                match positions.get(&index) {
                    Some(raw) => out.set_fader(index, *raw),
                    None if executors.contains(&index) => out.set_fader(index, 0),
                    None => out.release_fader(index),
                }
            }
        });
    }

    /// The lights the desk should be showing: a key held is lit, a bound key is lit
    /// while what it presses is live and dim otherwise, a master's Go is lit while its
    /// sequence is up, LEARN blinks while learning, and every key that does something
    /// is dim so the desk says what can be pressed.
    fn drive_leds(&self) {
        let Some(map) = self.map.as_deref() else { return };
        let operator = self.station.operator();
        let blink = now_ms() % 600 < 300;
        let mut frame: Vec<(usize, u8)> = Vec::new();
        for (led, index) in &self.leds {
            let code = map.hardkeys.get(index).and_then(|c| c.code.as_deref()).unwrap_or("");
            let verb = keys::key_verb(code);
            let master = match self.fader_keys.get(index) {
                Some(FaderKey::Go(f)) => Some((true, *f)),
                Some(FaderKey::Off(f)) => Some((false, *f)),
                _ => None,
            }
            .and_then(|(go, fader)| match self.binding(&SurfaceControl::Fader { index: fader }) {
                Some(BindingAction::Master { sequence_id }) => Some((go, *sequence_id)),
                _ => None,
            });
            let storing = operator.line.trim() == "store";
            let level = if self.held_keys.contains(index) {
                keys::LIT
            } else if storing && self.store_onto(*index, code, self.fader_keys.get(index).copied()).is_some() {
                // Waiting for where: every playback STORE can go onto says so.
                if blink { keys::LIT } else { keys::DIM }
            } else if verb == Some(Verb::Learn) {
                if operator.learning {
                    if blink { keys::LIT } else { 0 }
                } else {
                    keys::DIM
                }
            } else if let Some(action) = self.binding(&SurfaceControl::Key { index: *index }) {
                if self.live(action) { keys::LIT } else { keys::DIM }
            } else if let Some((go, sequence_id)) = master {
                let up = self.lit.sequences.iter().any(|s| s.id == sequence_id && s.active_cue_index.is_some());
                if go && up { keys::LIT } else { keys::DIM }
            } else if verb == Some(Verb::PageUp) {
                if operator.encoders.page > 0 { keys::DIM } else { 0 }
            } else if verb == Some(Verb::PageDown) {
                if operator.encoders.page + 1 < operator.encoders.pages { keys::DIM } else { 0 }
            } else if verb == Some(Verb::PrevPage) {
                if operator.page > 1 { keys::DIM } else { 0 }
            } else if verb == Some(Verb::NextPage) {
                if operator.page <= self.last_page() { keys::DIM } else { 0 }
            } else if (code == "STORE" && storing)
                || (code == "DELETE" && operator.line.trim() == "delete")
                || (code == "MOVE" && operator.line.trim() == "move")
            {
                // Waiting for where: MA's Store blinks until it is told.
                if blink { keys::LIT } else { keys::DIM }
            } else if keys::keypad_word(code).is_some() || verb.is_some() {
                keys::DIM
            } else {
                0
            };
            if let Some(led) = map.leds.get(*led) {
                frame.extend(keys::led_channels(led).map(|channel| (channel, level)));
            }
        }
        self.wing.drive(|out| {
            for (channel, level) in frame {
                out.set_led(channel, level);
            }
        });
    }

    /// Whether what a bound key presses is live: a running timeline, a sequence with a
    /// cue up, a variable that holds something — `surface.ts`'s `targetLive`.
    fn live(&self, action: &BindingAction) -> bool {
        let BindingAction::Press { target } = action else { return false };
        match target {
            TileTarget::Timeline { timeline_id, .. } => self.lit.timelines.iter().any(|t| t.id == *timeline_id && t.running),
            TileTarget::SequenceGo { sequence_id } | TileTarget::SequenceOff { sequence_id } => {
                self.lit.sequences.iter().any(|s| s.id == *sequence_id && s.active_cue_index.is_some())
            }
            TileTarget::VariableSetter { variable_id, .. } | TileTarget::VariableReadout { variable_id, .. } => {
                self.lit.variables.iter().any(|v| v.id == *variable_id && v.current() != 0.0)
            }
            _ => false,
        }
    }

    // ── Telling the page ──────────────────────────────────────────────────────

    fn ask(&self, ask: WingAsk) {
        if !self.station.claimed() {
            return;
        }
        if let Ok(value) = serde_json::to_value(&ask) {
            let _ = self.broadcast.0.send((vec![PathSegment::Key(ASKS_PATH.into())], value));
        }
    }

    /// Something for the operator to read: on the claiming page, and in the log either
    /// way, because a wing nobody is looking at a screen for still did it.
    fn toast(&self, text: &str, good: bool) {
        info!("[wing] {text}");
        self.ask(WingAsk::Toast { text: text.to_string(), good });
    }
}

/// How a write is attributed.
#[derive(Clone, Copy)]
enum Write {
    /// Nobody's: a flash, which puts back what it found.
    Unattributed,
    /// The wing's operator's, standing alone.
    Attributed,
    /// The wing's operator's, as part of one act.
    In(Uuid),
}

/// The fade a new cue gets: none, the Store dialog's, `cues.ts`'s `DEFAULT_FADE_MS`.
const DEFAULT_FADE_MS: u32 = 0;

/// Which cue is driving each key: the latest capture of it over the tracked stack of
/// each live sequence, the sequence that went most recently winning — what a Go put
/// there. Not `live_fades`: a key the programmer holds is exactly the key playback stops
/// publishing. The page's `drivingCues` and the command line's `update` are the same
/// rule.
fn driving_cues(sequences: &[Sequence], cues: &[Cue]) -> HashMap<(Uuid, String), Uuid> {
    let mut live: Vec<&Sequence> = sequences.iter().filter(|s| s.active_cue_index.is_some()).collect();
    live.sort_by_key(|s| s.went_at.unwrap_or(0));
    let mut out = HashMap::new();
    for sequence in live {
        let Some(active) = sequence.active_cue_index else { continue };
        for id in sequence.cue_ids.iter().take(active + 1) {
            let Some(cue) = cues.iter().find(|c| c.id == *id) else { continue };
            for capture in &cue.captures {
                out.insert((capture.fixture_id, parameter_key(&capture.parameter_kind)), cue.id);
            }
        }
    }
    out
}

/// What a cue's captures become once these entries are merged into it: the cue keeps
/// everything else it said, and a capture it already had keeps its own timing and
/// curve — the programmer holds values and never timing. `programmer.ts`'s
/// `storeCaptures` in merge mode.
fn store_captures(existing: &[ParameterCapture], entries: &[&ProgrammerValue]) -> Vec<ParameterCapture> {
    let key = |fixture: Uuid, kind: &ParameterKind| (fixture, parameter_key(kind));
    let before: HashMap<(Uuid, String), &ParameterCapture> =
        existing.iter().map(|c| (key(c.fixture_id, &c.parameter_kind), c)).collect();
    let stored: Vec<ParameterCapture> = entries
        .iter()
        .map(|entry| {
            let kept = before.get(&key(entry.fixture_id, &entry.parameter_kind));
            ParameterCapture {
                fixture_id: entry.fixture_id,
                parameter_kind: entry.parameter_kind.clone(),
                value: entry.value.clone(),
                preset: entry.preset,
                fade_in_ms: kept.map_or(0, |c| c.fade_in_ms),
                fade_out_ms: kept.map_or(0, |c| c.fade_out_ms),
                delay_in_ms: kept.map_or(0, |c| c.delay_in_ms),
                // A stored effect drops its anchor: the cue's `went_at` is what it is
                // measured from on every Go.
                effect: entry.effect.clone().map(|spec| pult_schema::types::effect::EffectSpec { t0: None, ..spec }),
                easing: kept.and_then(|c| c.easing),
            }
        })
        .collect();
    let taken: HashSet<(Uuid, String)> = stored.iter().map(|c| key(c.fixture_id, &c.parameter_kind)).collect();
    existing
        .iter()
        .filter(|c| !taken.contains(&key(c.fixture_id, &c.parameter_kind)))
        .cloned()
        .chain(stored)
        .collect()
}

/// A hand-picked list as a question: `selection.ts`'s `idsQuery`.
fn ids_query(ids: Vec<Uuid>) -> SelectionQuery {
    SelectionQuery {
        clauses: vec![SelectionClause { combine: SelectionCombine::Add, term: SelectionTerm::Ids { ids } }],
        ..SelectionQuery::empty()
    }
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

fn entry_id(fixture_id: Uuid, parameter: &ParameterKind) -> Uuid {
    programmer_entry_id(&fixture_id.to_string(), &parameter_key(parameter)).parse().unwrap_or_default()
}

fn field(table: &str, id: Uuid, name: &str) -> Path {
    vec![PathSegment::Key(table.into()), PathSegment::Id(id), PathSegment::Key(name.into())]
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
        assert_eq!(keys::level_of(4096), 1.0);
        assert_eq!(keys::level_of(9000), 1.0, "clamped");
    }

    fn capture(fixture: Uuid, fade: u32) -> ParameterCapture {
        ParameterCapture {
            fixture_id: fixture,
            parameter_kind: ParameterKind::Intensity,
            value: ParameterValue::Float(0.2),
            preset: None,
            fade_in_ms: fade,
            fade_out_ms: 0,
            delay_in_ms: 0,
            effect: None,
            easing: None,
        }
    }

    /// Update keeps the timing an operator set on a capture, and the rest of the cue.
    #[test]
    fn an_update_keeps_a_captures_timing_and_the_rest_of_the_cue() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let entry = ProgrammerValue {
            id: Uuid::new_v4(),
            fixture_id: a,
            parameter_kind: ParameterKind::Intensity,
            value: ParameterValue::Float(0.9),
            effect: None,
            preset: None,
            locked: false,
        };
        let out = store_captures(&[capture(a, 3000), capture(b, 100)], &[&entry]);
        assert_eq!(out.len(), 2);
        let mine = out.iter().find(|c| c.fixture_id == a).unwrap();
        assert_eq!(mine.value, ParameterValue::Float(0.9));
        assert_eq!(mine.fade_in_ms, 3000, "kept");
        assert!(out.iter().any(|c| c.fixture_id == b), "the rest of the cue stays");
    }
}
