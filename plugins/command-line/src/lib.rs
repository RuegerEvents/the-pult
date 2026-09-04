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
    schema::{Cue, EffectSpec, Easing, FollowMode, ParameterCapture},
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
            Command::Select { ops, at } => self.select(ops, at, ctx),
            Command::Clear { also_selection } => self.clear(also_selection),
            Command::Intensity { level } => self.intensity(level, ctx),
            Command::Home => self.home(&selection_of(ctx)),
            Command::EntityCommand { table, target, command, args } => {
                self.entity_command(&table, target, &command, args)
            }
            Command::Create { table, name } => self.create(&table, name),
            Command::Delete { table, target } => self.delete(&table, target),
            Command::SetField { table, target, field, value } => {
                self.set_field(&table, target, &field, value)
            }
            Command::Store { sequence, cue } => self.store(sequence, cue),
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
        at: Option<Then>,
        ctx: &Value,
    ) -> Result<surface::ExecResponse, String> {
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

        let fixtures = collection("fixtures")?;
        let mut selected: Vec<String> = selection_of(ctx);
        let mut only_group: Option<String> = None;
        for (op, target) in ops {
            let ids: Vec<String> = match target {
                SelectTarget::Fixtures(range) => {
                    if range.to > fixtures.len() {
                        return Err(format!(
                            "there are {} fixtures; {} is past the end",
                            fixtures.len(),
                            range.to
                        ));
                    }
                    fixtures[range.from - 1..range.to]
                        .iter()
                        .filter_map(|f| f.get("id").and_then(Value::as_str).map(String::from))
                        .collect()
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
        let mut text = match (count, live_query.is_some().then_some(only_group).flatten()) {
            (0, Some(name)) => format!("{name} is empty"),
            (0, None) => "nothing selected".to_string(),
            (1, Some(name)) => format!("{name}: 1 fixture"),
            (n, Some(name)) => format!("{name}: {n} fixtures"),
            (1, None) => "1 fixture selected".to_string(),
            (n, None) => format!("{n} fixtures selected"),
        };
        // The combined form: `fixture 1 thru 3 @ 80` sets the level on what it
        // just selected, not on whatever was selected before.
        match at {
            Some(_) if selected.is_empty() => {
                return Err("that selects nothing, so there is nothing to set".into());
            }
            Some(Then::At(level)) => {
                self.apply_level(&selected, level)?;
                text.push_str(&format!(", {}", said(level)));
            }
            Some(Then::Home) => {
                self.send_home(&selected)?;
                text.push_str(", home");
            }
            None => {}
        }
        let mut response = lines_response(vec![output_line("result", text)]);
        response.effects = Some(match live_query {
            Some(query) => json!({ "selection": { "query": query } }),
            None => json!({ "selection": { "fixtureIds": selected } }),
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
        self.apply_level(&selected, level)?;
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
    fn home(&self, selected: &[String]) -> Result<surface::ExecResponse, String> {
        if selected.is_empty() {
            return Err("nothing is selected — `fixture 1 thru 5` first".into());
        }
        self.send_home(selected)?;
        Ok(lines_response(vec![output_line(
            "result",
            format!(
                "{} fixture{} home",
                selected.len(),
                if selected.len() == 1 { "" } else { "s" }
            ),
        )]))
    }

    fn send_home(&self, fixtures: &[String]) -> Result<(), String> {
        for fixture_id in fixtures {
            host::set(&["programmer_values", "__home"], &json!({ "fixtureId": fixture_id }))?;
        }
        Ok(())
    }

    /// Set the level, or move it — whichever the operator wrote.
    fn apply_level(&self, fixtures: &[String], level: Level) -> Result<(), String> {
        match level {
            Level::To(percent) => self.hold_intensity(fixtures, percent),
            Level::By(points) => self.nudge_intensity(fixtures, points),
        }
    }

    /// Move these fixtures' Intensity by so many percentage points.
    ///
    /// The station does the arithmetic, from what it is showing at the moment it
    /// applies the write. This plugin deliberately does not read a level and compute
    /// a destination: two operators nudging the same light would then read the same
    /// number and write the same answer, and one of the two nudges would be lost.
    /// It also does not need the fixture to be in the programmer already — the
    /// station takes the key, starting from what playback has it at.
    fn nudge_intensity(&self, fixtures: &[String], points: f64) -> Result<(), String> {
        for fixture_id in fixtures {
            host::set(
                &["programmer_values", "__by"],
                &json!({
                    "fixtureId": fixture_id,
                    "parameterKind": "Intensity",
                    "by": points / 100.0,
                }),
            )?;
        }
        Ok(())
    }

    /// Put these fixtures' Intensity into the programmer at a percentage.
    fn hold_intensity(&self, fixtures: &[String], percent: f64) -> Result<(), String> {
        let level = (percent / 100.0).clamp(0.0, 1.0);
        let value = json!({ "type": "Float", "value": level });
        let held: Vec<String> = collection("programmer_values")?
            .iter()
            .filter_map(|e| e.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        for fixture_id in fixtures {
            // The id is derived, not minted — the same derivation the values
            // panel uses, so the two write the same row and converge.
            let id = core::entry_id(fixture_id, "Intensity");
            if held.contains(&id) {
                host::set(&["programmer_values", &id, "value"], &value)?;
            } else {
                let entry = json!({
                    "id": id,
                    "fixture_id": fixture_id,
                    "parameter_kind": "Intensity",
                    "value": value,
                    "effect": null,
                    "locked": false,
                });
                host::set(&["programmer_values", "__create"], &entry)?;
            }
        }
        Ok(())
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

    /// The programmer into a cue: merge into an existing one, or make the next
    /// one. The shape mirrors the store menu exactly — fades of zero on the
    /// captures, the cue's own fade at half a second, effects with their
    /// anchors dropped.
    fn store(&self, sequence: Target, cue: Target) -> Result<surface::ExecResponse, String> {
        let (sequence_id, sequence_name) = resolve("sequences", &sequence)?;
        let sequence_id = uuid(&sequence_id)?;
        let cue_ids = data::sequences().by_id(sequence_id).cue_ids().get()?;

        let entries = data::programmer_values().get()?;
        if entries.is_empty() {
            return Err("the programmer is empty — nothing to store".into());
        }
        let captures: Vec<ParameterCapture> = entries
            .into_iter()
            .map(|entry| ParameterCapture {
                fixture_id: entry.fixture_id,
                parameter_kind: entry.parameter_kind,
                value: entry.value,
                fade_in_ms: 0,
                fade_out_ms: 0,
                delay_in_ms: 0,
                // A stored effect drops its anchor: the cue's `went_at` is what it
                // is measured from on every Go.
                effect: entry.effect.map(|spec| EffectSpec { t0: None, ..spec }),
                easing: Easing::Linear,
            })
            .collect();

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

        match existing {
            Some(cue_id) => {
                let old = data::cues().by_id(cue_id).get()?;
                let merged = merge_captures(old.captures, captures);
                data::cues().by_id(cue_id).captures().set(merged)?;
                Ok(lines_response(vec![output_line(
                    "result",
                    format!("stored into \"{}\" of \"{sequence_name}\"", old.name),
                )]))
            }
            None => {
                if let Target::Index(n) = cue {
                    if n != cue_ids.len() + 1 {
                        return Err(format!(
                            "\"{sequence_name}\" has {} cues — store into one of those, or {} for a new one",
                            cue_ids.len(),
                            cue_ids.len() + 1
                        ));
                    }
                }
                let name = match &cue {
                    Target::Name(name) => name.clone(),
                    Target::Index(n) => format!("Cue {n}"),
                };
                let cue_id = uuid::Uuid::new_v4();
                data::cues().create(&Cue {
                    id: cue_id,
                    name: name.clone(),
                    number: (cue_ids.len() + 1) as f64,
                    captures,
                    follow_mode: FollowMode::Manual,
                    fade_in_ms: 500,
                    fade_out_ms: 500,
                    is_active: false,
                })?;
                let mut ids = cue_ids;
                ids.push(cue_id);
                data::sequences().by_id(sequence_id).cue_ids().set(ids)?;
                Ok(lines_response(vec![output_line(
                    "result",
                    format!("stored as \"{name}\" in \"{sequence_name}\""),
                )]))
            }
        }
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
                    for (i, entry) in entries.iter().enumerate() {
                        let number = (i + 1).to_string();
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

/// Merge new captures over old, one slot per fixture and parameter — the store
/// menu's merge mode.
fn merge_captures(
    existing: Vec<ParameterCapture>,
    stored: Vec<ParameterCapture>,
) -> Vec<ParameterCapture> {
    let key = |c: &ParameterCapture| format!("{}/{:?}", c.fixture_id, c.parameter_kind);
    let taken: Vec<String> = stored.iter().map(&key).collect();
    existing.into_iter().filter(|c| !taken.contains(&key(c))).chain(stored).collect()
}

/// A uuid the show already gave out, back as one.
///
/// `resolve` answers what the station spelled, and the typed accessors take the
/// thing rather than the spelling of it — so this is where a string stops being one.
fn uuid(id: &str) -> Result<uuid::Uuid, String> {
    uuid::Uuid::parse_str(id).map_err(|e| format!("{id} is not an id: {e}"))
}

fn selection_of(ctx: &Value) -> Vec<String> {
    ctx.get("selection")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
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
