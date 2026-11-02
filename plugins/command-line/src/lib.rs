//! The command-line plugin: the console's grammar, attached to the console.
//!
//! `command-line-core` decides what the words mean; this crate is the part
//! that has hands — it reads the show to turn "sequence 2" into a uuid,
//! writes the programmer, presses Go. It is also the reference example for
//! writing a plugin: one type implementing [`PultPlugin`], registered at the
//! bottom with `plugin_main!`.
//!
//! Other plugins drive it through the same entry point the console panel
//! uses: `exec` with a `{ "line": ... }` — that is the whole inter-plugin API.

use command_line_core as core;
use core::{
    Catalog, Command, Completions, Expectation, Level, ParseError, SelOp, SelectTarget, Target,
    Then,
};
use pult_plugin_sdk::{
    self as sdk, data,
    host, output_line,
    schema::{parameter_key, Preset, PresetValue},
    surface, PultPlugin,
};
use serde_json::{json, Value};

struct CommandLine {
    catalog: Catalog,
}

impl PultPlugin for CommandLine {
    fn init(_config: Value) -> Result<Self, String> {
        // The whole vocabulary comes from the console at startup. Reloading the
        // plugin (or the console) refreshes it; nothing about the schema is
        // written down in here.
        let catalog =
            Catalog::from_introspection(&host::entities(), &host::commands(), &host::rpcs());
        sdk::log_info!(
            "command line ready: {} collections, {} commands, {} rpcs",
            catalog.entities.len(),
            catalog.commands.len(),
            catalog.rpcs.len()
        );
        Ok(CommandLine { catalog })
    }

    fn handle(&mut self, method: &str, args: Value, ctx: Value) -> Result<Value, String> {
        match method {
            // The console surface and other plugins share one entry point on
            // purpose: anything an operator can type, a plugin can ask for.
            "surface.exec" | "exec" => {
                let line = args
                    .get("line")
                    .and_then(Value::as_str)
                    .ok_or("exec takes { \"line\": \"...\" }")?;
                let response = self.exec(line, &ctx);
                serde_json::to_value(&response).map_err(|e| e.to_string())
            }
            "surface.complete" => {
                let line = args.get("line").and_then(Value::as_str).unwrap_or("");
                let cursor =
                    args.get("cursor").and_then(Value::as_u64).unwrap_or(line.len() as u64);
                let response = self.complete(line, cursor as usize);
                serde_json::to_value(&response).map_err(|e| e.to_string())
            }
            "surface.help" => {
                let topic = args.get("topic").and_then(Value::as_str);
                let text = core::help(&self.catalog, topic);
                Ok(json!({ "text": text }))
            }
            // The grammar as text, for a plugin that wants to explain this
            // command line to something else — a prompt, a manual, a tooltip.
            "grammar" => Ok(json!({ "text": core::help(&self.catalog, None) })),
            _ => Err(format!("the command line has no method called {method:?}")),
        }
    }
}

sdk::plugin_main!(CommandLine);

// ── Execution ─────────────────────────────────────────────────────────────────

impl CommandLine {
    fn exec(&self, line: &str, ctx: &Value) -> surface::ExecResponse {
        let command = match core::parse(&self.catalog, line) {
            Ok(command) => command,
            Err(error) => return parse_error_response(error),
        };
        match self.run(command, ctx) {
            Ok(response) => response,
            Err(message) => error_response(message),
        }
    }

    fn run(&self, command: Command, ctx: &Value) -> Result<surface::ExecResponse, String> {
        match command {
            Command::Help { topic } => {
                let text = core::help(&self.catalog, topic.as_deref());
                Ok(lines_response(vec![output_line("info", text)]))
            }
            Command::Select { ops, cells, at } => self.select(ops, cells, at, ctx),
            Command::Clear { also_selection } => self.clear(also_selection),
            Command::Intensity { level } => self.intensity(level, ctx),
            Command::Home => self.home(&selection_of(ctx), &cells_of(ctx)),
            Command::EntityCommand { table, target, command, args } => {
                self.entity_command(&table, target, &command, args)
            }
            Command::Create { table, name } => self.create(&table, name),
            Command::Delete { table, target } => self.delete(&table, target),
            Command::SetField { table, target, field, value } => {
                self.set_field(&table, target, &field, value)
            }
            Command::Store { sequence, cue } => self.store(sequence, cue),
            Command::Preset(target) => {
                let selection = selection_of(ctx);
                let (name, n) = self.recall_preset(&target, &selection)?;
                Ok(lines_response(vec![output_line(
                    "result",
                    format!("preset \"{name}\" on {n} {}", if n == 1 { "fixture" } else { "fixtures" }),
                )]))
            }
            Command::StorePreset(target) => self.store_preset(target),
            Command::Update => self.update(),
            Command::Rpc { method, args } => {
                let args: Value = args.into_iter().collect::<serde_json::Map<_, _>>().into();
                let result = host::call(&method, &args)?;
                let text = match &result {
                    Value::Null => format!("{method}: done"),
                    other => format!("{method}: {other}"),
                };
                Ok(lines_response(vec![output_line("result", text)]))
            }
        }
    }

    // ── Selection and the programmer ──────────────────────────────────────────

    fn select(
        &self,
        ops: Vec<(SelOp, SelectTarget)>,
        cells: Vec<core::Range>,
        at: Option<Then>,
        ctx: &Value,
    ) -> Result<surface::ExecResponse, String> {
        // `. 1 thru 20` with nothing before it cuts what is already selected.
        let cut_only = ops.is_empty();
        let spans: Vec<Value> =
            cells.iter().map(|r| json!({ "from": r.from, "to": r.to })).collect();
        // `group 3` on its own hands the browser the group's *question*, so what it
        // leaves selected is what recalling the group in the panel leaves: a
        // selection that goes on following the rig. Anything mixed — a group plus a
        // range, or a group taken away from one — cannot be said as one query,
        // because a group's own clauses may narrow or subtract and appending them
        // would narrow the whole line rather than the group. So that resolves to a
        // list, and says as much by being one.
        let live_query = match ops.as_slice() {
            [(SelOp::Replace, SelectTarget::Group(target))] => {
                let (_, _, query) = self.group(target)?;
                Some(query)
            }
            _ => None,
        };

        let fixtures = if cut_only { Vec::new() } else { collection("fixtures")? };
        let mut selected: Vec<String> = selection_of(ctx);
        let mut only_group: Option<String> = None;
        for (op, target) in ops {
            let ids: Vec<String> = match target {
                SelectTarget::Fixtures(range) => {
                    let found: Vec<String> = core::numbered(&fixture_numbers(&fixtures), range)
                        .into_iter()
                        .filter_map(|i| fixtures[i].get("id").and_then(Value::as_str).map(String::from))
                        .collect();
                    if found.is_empty() {
                        return Err(if range.from == range.to {
                            format!("no fixture is numbered {}", range.from)
                        } else {
                            format!("no fixture is numbered {} to {}", range.from, range.to)
                        });
                    }
                    found
                }
                SelectTarget::Group(ref target) => {
                    let (id, name, _) = self.group(target)?;
                    only_group = Some(name);
                    // The station evaluates it, not this plugin: one evaluator per
                    // side of the wire, and the browser's is the other one.
                    let answer = host::call("selection.resolve", &json!({ "groupId": id }))?;
                    serde_json::from_value(answer)
                        .map_err(|e| format!("the station's answer did not parse: {e}"))?
                }
            };
            match op {
                SelOp::Replace => selected = ids,
                SelOp::Add => {
                    for id in ids {
                        if !selected.contains(&id) {
                            selected.push(id);
                        }
                    }
                }
                SelOp::Remove => selected.retain(|id| !ids.contains(id)),
            }
        }
        let count = selected.len();
        let cut = said_cells(&cells);
        let mut text = match (count, live_query.is_some().then_some(only_group).flatten()) {
            (0, Some(name)) => format!("{name} is empty"),
            (0, None) => "nothing selected".to_string(),
            (1, Some(name)) => format!("{name}: 1 fixture"),
            (n, Some(name)) => format!("{name}: {n} fixtures"),
            (1, None) => "1 fixture selected".to_string(),
            (n, None) => format!("{n} fixtures selected"),
        };
        if let Some(cut) = &cut {
            text.push_str(&format!(", {cut}"));
        }
        // The combined form: `fixture 1 thru 3 @ 80` sets the level on what it
        // just selected, not on whatever was selected before.
        match at {
            Some(_) if selected.is_empty() => {
                return Err("that selects nothing, so there is nothing to set".into());
            }
            Some(Then::At(level)) => {
                self.apply_level(&selected, &spans, level)?;
                text.push_str(&format!(", {}", said(level)));
            }
            Some(Then::Home) => {
                self.send_home(&selected, &spans)?;
                text.push_str(", home");
            }
            Some(Then::Preset(target)) => {
                let (name, n) = self.recall_preset(&target, &selected)?;
                text.push_str(&format!(", preset \"{name}\" on {n}"));
            }
            None => {}
        }
        let mut response = lines_response(vec![output_line("result", text)]);
        // A cut is part of the selection's question, so it travels as a query: the
        // group's own with its cells replaced, or the ids with the cells beside them.
        response.effects = Some(match (live_query, spans.is_empty()) {
            (Some(query), true) => json!({ "selection": { "query": query } }),
            (Some(mut query), false) => {
                query["cells"] = Value::Array(spans);
                json!({ "selection": { "query": query } })
            }
            (None, true) => json!({ "selection": { "fixtureIds": selected } }),
            (None, false) => json!({ "selection": { "query": {
                "clauses": [{ "combine": "Add", "term": { "kind": "Ids", "ids": selected } }],
                "order": { "kind": "Manual", "order": [] },
                "cells": spans,
            } } }),
        });
        Ok(response)
    }

    /// `group 3` / `group "Movers"` → its id, its name, and the question it asks.
    fn group(&self, target: &Target) -> Result<(String, String, Value), String> {
        let (id, name) = resolve("groups", target)?;
        let query = host::get(&["groups", &id, "query"])?;
        Ok((id, name, query))
    }

    fn clear(&self, also_selection: bool) -> Result<surface::ExecResponse, String> {
        let entries = collection("programmer_values")?;
        let mut dropped = 0;
        for entry in &entries {
            let locked = entry.get("locked").and_then(Value::as_bool).unwrap_or(false);
            let Some(id) = entry.get("id").and_then(Value::as_str) else { continue };
            if !locked {
                host::set(&["programmer_values", id, "__delete"], &Value::Null)?;
                dropped += 1;
            }
        }
        let kept = entries.len() - dropped;
        let mut text = match dropped {
            0 => "programmer was already empty".to_string(),
            n => format!("cleared {n} value{}", if n == 1 { "" } else { "s" }),
        };
        if kept > 0 {
            text.push_str(&format!(" ({kept} locked value{} kept)", if kept == 1 { "" } else { "s" }));
        }
        let mut response = lines_response(vec![output_line("result", text)]);
        if also_selection {
            response.effects = Some(json!({ "selection": { "fixtureIds": [] } }));
        }
        Ok(response)
    }

    fn intensity(&self, level: Level, ctx: &Value) -> Result<surface::ExecResponse, String> {
        let selected = selection_of(ctx);
        if selected.is_empty() {
            return Err("nothing is selected — `fixture 1 thru 5` first".into());
        }
        self.apply_level(&selected, &cells_of(ctx), level)?;
        Ok(lines_response(vec![output_line(
            "result",
            format!(
                "{} fixture{} {}",
                selected.len(),
                if selected.len() == 1 { "" } else { "s" },
                said(level)
            ),
        )]))
    }

    /// Put the selection back where it rests.
    ///
    /// One write per fixture and no parameter named, so the station decides both what
    /// a fixture has and where each of its parameters rests. This plugin is granted
    /// no access to fixture types and needs none: home is a destination it never has
    /// to know, which is the same trick `at +10` plays with a level.
    fn home(&self, selected: &[String], cells: &[Value]) -> Result<surface::ExecResponse, String> {
        if selected.is_empty() {
            return Err("nothing is selected — `fixture 1 thru 5` first".into());
        }
        self.send_home(selected, cells)?;
        Ok(lines_response(vec![output_line(
            "result",
            format!(
                "{} fixture{} home",
                selected.len(),
                if selected.len() == 1 { "" } else { "s" }
            ),
        )]))
    }

    /// Whole fixtures go home in one write each, and the station enumerates what they
    /// have. Cut to cells, only those cells' own parameters go: a bar's master is the
    /// whole bar's, and sending three of its cells home is not asking for it.
    fn send_home(&self, fixtures: &[String], cells: &[Value]) -> Result<(), String> {
        if cells.is_empty() {
            for fixture_id in fixtures {
                host::set(&["programmer_values", "__home"], &json!({ "fixtureId": fixture_id }))?;
            }
            return Ok(());
        }
        for kind in per_cell_kinds(fixtures)? {
            for (fixture_id, kind) in targets(fixtures, cells, &kind)? {
                if kind.get("Cell").is_none() {
                    continue;
                }
                host::set(
                    &["programmer_values", "__home"],
                    &json!({ "fixtureId": fixture_id, "parameterKind": kind }),
                )?;
            }
        }
        Ok(())
    }

    /// Set the level, or move it — whichever the operator wrote.
    fn apply_level(&self, fixtures: &[String], cells: &[Value], level: Level) -> Result<(), String> {
        let landed = targets(fixtures, cells, &json!("Intensity"))?;
        match level {
            Level::To(percent) => self.hold_intensity(&landed, percent),
            Level::By(points) => self.nudge_intensity(&landed, points),
        }
    }

    /// Move these Intensities by so many percentage points.
    ///
    /// The station does the arithmetic, from what it is showing at the moment it
    /// applies the write. This plugin deliberately does not read a level and compute
    /// a destination: two operators nudging the same light would then read the same
    /// number and write the same answer, and one of the two nudges would be lost.
    /// It also does not need the fixture to be in the programmer already — the
    /// station takes the key, starting from what playback has it at.
    fn nudge_intensity(&self, landed: &[(String, Value)], points: f64) -> Result<(), String> {
        for (fixture_id, kind) in landed {
            host::set(
                &["programmer_values", "__by"],
                &json!({
                    "fixtureId": fixture_id,
                    "parameterKind": kind,
                    "by": points / 100.0,
                }),
            )?;
        }
        Ok(())
    }

    /// Put these Intensities into the programmer at a percentage.
    fn hold_intensity(&self, landed: &[(String, Value)], percent: f64) -> Result<(), String> {
        let level = (percent / 100.0).clamp(0.0, 1.0);
        let value = json!({ "type": "Float", "value": level });
        let held: Vec<String> = collection("programmer_values")?
            .iter()
            .filter_map(|e| e.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        for (fixture_id, kind) in landed {
            let key = key_of(kind)?;
            // The id is derived, not minted — the same derivation the values
            // panel uses, so the two write the same row and converge.
            let id = core::entry_id(fixture_id, &key);
            if held.contains(&id) {
                host::set(&["programmer_values", &id, "value"], &value)?;
            } else {
                let entry = json!({
                    "id": id,
                    "fixture_id": fixture_id,
                    "parameter_kind": kind,
                    "value": value,
                    "effect": null,
                    "locked": false,
                });
                host::set(&["programmer_values", "__create"], &entry)?;
            }
        }
        Ok(())
    }

    // ── Presets ───────────────────────────────────────────────────────────────

    /// Recall a preset into the programmer, over a selection or over everything it
    /// knows.
    ///
    /// Each entry carries the **reference and the value it resolved to** — the rule
    /// `crates/pult-schema/src/types/preset.rs` states and `applyPreset` in the
    /// browser follows: the reference is what makes a later edit reach the cue, and
    /// the value is what plays when the preset is gone.
    fn recall_preset(&self, target: &Target, selection: &[String]) -> Result<(String, usize), String> {
        let (id, name) = resolve("presets", target)?;
        let preset: Preset = serde_json::from_value(host::get(&["presets", &id])?)
            .map_err(|e| format!("that preset does not parse: {e}"))?;

        let held: Vec<String> = collection("programmer_values")?
            .iter()
            .filter_map(|e| e.get("id").and_then(Value::as_str).map(String::from))
            .collect();

        let mut written = 0;
        for value in &preset.values {
            let fixture_id = value.fixture_id.to_string();
            // Nothing selected means every fixture the preset knows: recalling *warm*
            // means warm. With a selection it is the intersection.
            if !selection.is_empty() && !selection.contains(&fixture_id) {
                continue;
            }
            let key = parameter_key(&value.parameter_kind);
            let entry_id = core::entry_id(&fixture_id, &key);
            let row = json!({
                "id": entry_id,
                "fixture_id": fixture_id,
                "parameter_kind": value.parameter_kind,
                "value": value.value,
                "effect": null,
                "preset": preset.id,
                "locked": false,
            });
            if held.contains(&entry_id) {
                // The whole row, because the reference and the value have to land
                // together — see the note on `flush` in `stores/programmer.ts`.
                host::set(&["programmer_values", &entry_id], &row)?;
            } else {
                host::set(&["programmer_values", "__create"], &row)?;
            }
            written += 1;
        }
        Ok((name, written))
    }

    /// The programmer as a preset: a new one, or merged into one that exists.
    ///
    /// An effect is left out, for the reason the browser leaves one out: a preset is a
    /// look, and a shape is an instruction with an anchor in it.
    fn store_preset(&self, target: Target) -> Result<surface::ExecResponse, String> {
        let entries = data::programmer_values().get()?;
        if entries.is_empty() {
            return Err("the programmer is empty — nothing to keep".into());
        }
        let values: Vec<PresetValue> = entries
            .into_iter()
            .filter(|entry| entry.effect.is_none())
            .map(|entry| PresetValue {
                fixture_id: entry.fixture_id,
                parameter_kind: entry.parameter_kind,
                value: entry.value,
            })
            .collect();
        if values.is_empty() {
            return Err("the programmer holds only shapes, and a preset is a look".into());
        }

        // An existing preset is merged into; anything else makes one.
        let existing = resolve("presets", &target).ok();
        match existing {
            Some((id, name)) => {
                let id = uuid(&id)?;
                let old = data::presets().by_id(id).get()?;
                let key = |v: &PresetValue| format!("{}/{}", v.fixture_id, parameter_key(&v.parameter_kind));
                let taken: Vec<String> = values.iter().map(&key).collect();
                let merged: Vec<PresetValue> = old
                    .values
                    .into_iter()
                    .filter(|v| !taken.contains(&key(v)))
                    .chain(values)
                    .collect();
                let n = merged.len();
                data::presets().by_id(id).values().set(merged)?;
                Ok(lines_response(vec![output_line(
                    "result",
                    format!("\"{name}\" now says {n} things — every cue using it followed"),
                )]))
            }
            None => {
                let name = match &target {
                    Target::Name(name) => name.clone(),
                    Target::Index(n) => format!("Preset {n}"),
                };
                let n = values.len();
                data::presets().create(&Preset { id: uuid::Uuid::new_v4(), name: name.clone(), values, color: None, folder: None })?;
                Ok(lines_response(vec![output_line(
                    "result",
                    format!("stored \"{name}\" — {n} values"),
                )]))
            }
        }
    }

    // ── Entities ──────────────────────────────────────────────────────────────

    fn entity_command(
        &self,
        table: &str,
        target: Target,
        command: &str,
        args: Vec<(String, Value)>,
    ) -> Result<surface::ExecResponse, String> {
        let (id, name) = resolve(table, &target)?;
        let entity = self
            .catalog
            .entities
            .iter()
            .find(|e| e.table == table)
            .ok_or_else(|| format!("no collection called {table}"))?;

        let mut payload = serde_json::Map::new();
        payload.insert(entity.id_arg(), Value::String(id.clone()));
        for (arg_name, value) in args {
            // `goto 3` names a cue by its place in this sequence; the command
            // wants its uuid. The one resolution words alone could not do.
            let value = if arg_name == "cueId" && value.is_u64() {
                let index = value.as_u64().unwrap() as usize;
                cue_id_in(&id, index)?
            } else {
                value
            };
            payload.insert(arg_name, value);
        }
        // A Go without a time is a Go now, same as pressing the button.
        let declares_at = self
            .catalog
            .command_for(table, command)
            .is_some_and(|c| c.args.iter().any(|a| a.name == "at"));
        if declares_at && !payload.contains_key("at") {
            payload.insert("at".into(), Value::from(now_ms()));
        }

        host::call(&format!("{table}.{command}"), &Value::Object(payload))?;
        Ok(lines_response(vec![output_line("result", format!("{name}: {command}"))]))
    }

    fn create(&self, table: &str, name: Option<String>) -> Result<surface::ExecResponse, String> {
        let word = singular(table);
        let name = name.unwrap_or_else(|| format!("New {word}"));
        let mut payload = json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "name": name,
        });
        // What a bare entity of each kind needs beyond a name. Kept small on
        // purpose: a type this table cannot default is better made in its own
        // panel, and the validation error below says so honestly.
        if table == "sequences" {
            payload["cue_ids"] = json!([]);
            payload["active_cue_index"] = Value::Null;
        }
        host::set(&[table, "__create"], &payload).map_err(|e| {
            format!("could not create a {word} from here ({e}) — its own panel can")
        })?;
        Ok(lines_response(vec![output_line("result", format!("created {word} \"{name}\""))]))
    }

    fn delete(&self, table: &str, target: Target) -> Result<surface::ExecResponse, String> {
        let (id, name) = resolve(table, &target)?;
        host::set(&[table, &id, "__delete"], &Value::Null)?;
        Ok(lines_response(vec![output_line(
            "result",
            format!("deleted {} \"{name}\"", singular(table)),
        )]))
    }

    fn set_field(
        &self,
        table: &str,
        target: Target,
        field: &str,
        value: Value,
    ) -> Result<surface::ExecResponse, String> {
        let (id, name) = resolve(table, &target)?;
        host::set(&[table, &id, field], &value)?;
        Ok(lines_response(vec![output_line("result", format!("{name}: {field} = {value}"))]))
    }

    /// The programmer into a cue: merge into an existing one, or make the next one.
    /// The station's Store (`programmer.store`), the one the Store dialog and the wing
    /// run, so a cue stored from here is the same cue. The programmer is kept, as the
    /// command line always has.
    fn store(&self, sequence: Target, cue: Target) -> Result<surface::ExecResponse, String> {
        let (sequence_id, sequence_name) = resolve("sequences", &sequence)?;
        let sequence_id = uuid(&sequence_id)?;
        let cue_ids = data::sequences().by_id(sequence_id).cue_ids().get()?;

        // An existing cue is merged into; a number one past the end (or a new
        // name) makes the next cue.
        let existing = match &cue {
            Target::Index(n) if *n <= cue_ids.len() => Some(cue_ids[n - 1]),
            Target::Index(_) => None,
            Target::Name(name) => {
                let cues = data::cues().get()?;
                cue_ids.iter().find(|id| {
                    cues.iter().any(|c| c.id == **id && c.name == *name)
                }).copied()
            }
        };
        if let (None, Target::Index(n)) = (existing, &cue) {
            if *n != cue_ids.len() + 1 {
                return Err(format!(
                    "\"{sequence_name}\" has {} cues — store into one of those, or {} for a new one",
                    cue_ids.len(),
                    cue_ids.len() + 1
                ));
            }
        }
        let mut args = json!({ "sequenceId": sequence_id, "keep": true });
        match (existing, &cue) {
            (Some(cue_id), _) => args["cueId"] = json!(cue_id),
            (None, Target::Name(name)) => args["name"] = json!(name),
            (None, Target::Index(_)) => {}
        }
        let stored = host::call("programmer.store", &args)?;
        let name = stored["cue"]["name"].as_str().unwrap_or("the cue");
        let text = if stored["created"].as_bool() == Some(true) {
            format!("stored as \"{name}\" in \"{sequence_name}\"")
        } else {
            format!("stored into \"{name}\" of \"{sequence_name}\"")
        };
        Ok(lines_response(vec![output_line("result", text)]))
    }

    /// `update` — every held value into the cue that is driving it now.
    ///
    /// The station's Update (`programmer.update`), the same act as the button in the
    /// top bar and UPDATE on the wing: a parameter being driven by a cue says which
    /// cue, so there is no target to name. The keys nothing is driving are reported
    /// rather than guessed at — a plugin cannot open the store dialog, so it says how
    /// many are left and leaves them in the programmer.
    fn update(&self) -> Result<surface::ExecResponse, String> {
        let answer = host::call("programmer.update", &json!({}))?;
        let updated = answer["updated"].as_u64().unwrap_or(0);
        let orphans = answer["orphans"].as_array().map_or(0, Vec::len);
        let cues = if updated == 1 { "cue" } else { "cues" };
        let text = match (&answer["edited"], updated, orphans) {
            (edited, _, _) if !edited.is_null() => {
                format!("updated \"{}\"", edited["name"].as_str().unwrap_or("the cue"))
            }
            (_, 0, _) => "nothing here is being driven by a cue".to_string(),
            (_, n, 0) => format!("updated {n} {cues}"),
            (_, n, left) => format!("updated {n} {cues} — {left} left, which no cue is driving"),
        };
        Ok(lines_response(vec![output_line("result", text)]))
    }

    // ── Completion ────────────────────────────────────────────────────────────

    /// Words the grammar can answer alone come straight from the core; what
    /// needs the show — names, how many entries there are — is filled in here.
    fn complete(&self, line: &str, cursor: usize) -> surface::CompleteResponse {
        let Completions { replace_from, prefix, expectations } =
            core::complete(&self.catalog, line, cursor);
        let prefix_lower = prefix.to_lowercase();
        let mut items = Vec::new();
        for expectation in expectations {
            match expectation {
                Expectation::Keyword { word, detail } => {
                    if word.to_lowercase().starts_with(&prefix_lower) {
                        items.push(surface::CompletionItem {
                            text: word,
                            detail: (!detail.is_empty()).then_some(detail),
                        });
                    }
                }
                Expectation::Value { hint } => {
                    // Nothing to insert; the hint rides along as a ghost item
                    // the surface shows dimmed and never inserts.
                    items.push(surface::CompletionItem {
                        text: String::new(),
                        detail: Some(hint),
                    });
                }
                Expectation::EntityRef { table } => {
                    let entries = collection(&table).unwrap_or_default();
                    // A fixture is typed by its own number; everything else by its place.
                    let numbers = (table == "fixtures").then(|| fixture_numbers(&entries));
                    for (i, entry) in entries.iter().enumerate() {
                        let number = match &numbers {
                            Some(numbers) => match numbers[i] {
                                Some(n) => n.to_string(),
                                None => continue,
                            },
                            None => (i + 1).to_string(),
                        };
                        let name = entry
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let matches = number.starts_with(&prefix_lower)
                            || name.to_lowercase().starts_with(&prefix_lower);
                        if matches {
                            items.push(surface::CompletionItem {
                                text: number,
                                detail: (!name.is_empty()).then_some(name),
                            });
                        }
                    }
                }
            }
        }
        surface::CompleteResponse { items, replace_from }
    }
}

// ── Small pieces ──────────────────────────────────────────────────────────────

/// How a level reads back: a destination arrived at, or a distance moved.
fn said(level: Level) -> String {
    match level {
        Level::To(percent) => format!("at {}%", percent.round()),
        Level::By(points) if points >= 0.0 => format!("{}% brighter", points.round()),
        Level::By(points) => format!("{}% darker", points.abs().round()),
    }
}

/// A collection as the ordered array the engine serves.
fn collection(table: &str) -> Result<Vec<Value>, String> {
    let mut path_get = host::get(&[table])?;
    match path_get.take() {
        Value::Array(entries) => Ok(entries),
        Value::Null => Ok(Vec::new()),
        _ => Err(format!("{table} did not come back as a collection")),
    }
}

/// `sequence 2` / `sequence "Songs"` → the entry's id and display name.
fn resolve(table: &str, target: &Target) -> Result<(String, String), String> {
    let entries = collection(table)?;
    let word = singular(table);
    let entry = match target {
        Target::Index(n) => entries.get(*n - 1).ok_or_else(|| match entries.len() {
            0 => format!("there are no {table} yet"),
            len => format!("there are {len} {table}; {n} is past the end"),
        })?,
        Target::Name(name) => entries
            .iter()
            .find(|e| e.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .or_else(|| {
                entries.iter().find(|e| {
                    e.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
            })
            .ok_or_else(|| format!("no {word} called {name:?}"))?,
    };
    let id = entry
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("that {word} has no id"))?
        .to_string();
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_string();
    Ok((id, name))
}

/// Cue number `n` of a sequence, as the uuid the command wants.
fn cue_id_in(sequence_id: &str, index: usize) -> Result<Value, String> {
    let seq = host::get(&["sequences", sequence_id])?;
    let ids = seq.get("cue_ids").and_then(Value::as_array).cloned().unwrap_or_default();
    if index == 0 || index > ids.len() {
        return Err(format!("that sequence has {} cues; {index} names none of them", ids.len()));
    }
    Ok(ids[index - 1].clone())
}

/// A uuid the show already gave out, back as one.
///
/// `resolve` answers what the station spelled, and the typed accessors take the
/// thing rather than the spelling of it — so this is where a string stops being one.
fn uuid(id: &str) -> Result<uuid::Uuid, String> {
    uuid::Uuid::parse_str(id).map_err(|e| format!("{id} is not an id: {e}"))
}

/// Each fixture's number, in patch order — `None` for one nobody numbered.
fn fixture_numbers(fixtures: &[Value]) -> Vec<Option<u32>> {
    fixtures
        .iter()
        .map(|f| f.get("fixture_number").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()))
        .collect()
}

fn selection_of(ctx: &Value) -> Vec<String> {
    ctx.get("selection")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// The cells the browser's selection is cut to: `[{ from, to }]`, or none.
fn cells_of(ctx: &Value) -> Vec<Value> {
    ctx.get("cells").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// Where a value of `kind` lands over these fixtures and cells, as `(fixture, kind)`.
/// The station's rule (`selection.targets`), never worked out here: the Programmer
/// asks the same rule in the browser, and a third copy would be the one that drifts.
fn targets(fixtures: &[String], cells: &[Value], kind: &Value) -> Result<Vec<(String, Value)>, String> {
    if fixtures.is_empty() {
        return Ok(Vec::new());
    }
    let answer = host::call(
        "selection.targets",
        &json!({ "fixtureIds": fixtures, "cells": cells, "parameterKind": kind }),
    )?;
    let rows = answer.as_array().cloned().unwrap_or_default();
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let fixture = row.get("fixtureId")?.as_str()?.to_string();
            let kind = row.get("parameterKind")?.clone();
            Some((fixture, kind))
        })
        .collect())
}

/// Every per-cell output parameter the types of these fixtures declare, once each.
fn per_cell_kinds(fixtures: &[String]) -> Result<Vec<Value>, String> {
    let rig = collection("fixtures")?;
    let types = collection("fixture_types")?;
    let mut kinds: Vec<Value> = Vec::new();
    for fixture in rig.iter().filter(|f| {
        f.get("id").and_then(Value::as_str).is_some_and(|id| fixtures.iter().any(|s| s == id))
    }) {
        let Some(type_id) = fixture.get("fixture_type_id").and_then(Value::as_str) else { continue };
        let Some(fixture_type) =
            types.iter().find(|t| t.get("id").and_then(Value::as_str) == Some(type_id))
        else {
            continue;
        };
        for p in fixture_type.get("parameters").and_then(Value::as_array).into_iter().flatten() {
            let per_cell = p.get("per_cell").and_then(Value::as_bool).unwrap_or(false);
            let output = p.get("direction").and_then(Value::as_str).unwrap_or("Output") == "Output";
            if let Some(kind) = p.get("kind").filter(|_| per_cell && output) {
                if !kinds.contains(kind) {
                    kinds.push(kind.clone());
                }
            }
        }
    }
    Ok(kinds)
}

/// The key a kind is held under: `Intensity`, `ColorRgb@7`.
fn key_of(kind: &Value) -> Result<String, String> {
    let kind: sdk::schema::ParameterKind =
        serde_json::from_value(kind.clone()).map_err(|e| format!("the station named a kind this cannot read: {e}"))?;
    Ok(parameter_key(&kind))
}

/// How a cut is said back: `cells 1–20`, `cells 5, 9`, `cells 20–1`.
fn said_cells(cells: &[core::Range]) -> Option<String> {
    if cells.is_empty() {
        return None;
    }
    let parts: Vec<String> = cells
        .iter()
        .map(|r| if r.from == r.to { r.from.to_string() } else { format!("{}–{}", r.from, r.to) })
        .collect();
    let word = if cells.len() == 1 && cells[0].from == cells[0].to { "cell" } else { "cells" };
    Some(format!("{word} {}", parts.join(", ")))
}

fn singular(table: &str) -> &str {
    table.strip_suffix('s').unwrap_or(table)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn lines_response(lines: Vec<surface::OutputLine>) -> surface::ExecResponse {
    surface::ExecResponse { lines, error: None, effects: None }
}

fn error_response(message: String) -> surface::ExecResponse {
    surface::ExecResponse {
        lines: Vec::new(),
        error: Some(surface::ExecError { message, span: None, expected: Vec::new() }),
        effects: None,
    }
}

fn parse_error_response(error: ParseError) -> surface::ExecResponse {
    surface::ExecResponse {
        lines: Vec::new(),
        error: Some(surface::ExecError {
            message: error.message,
            span: Some(surface::ErrorSpan { start: error.span.0, end: error.span.1 }),
            expected: error.expected,
        }),
        effects: None,
    }
}
