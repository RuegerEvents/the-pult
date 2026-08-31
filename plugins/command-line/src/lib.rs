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
use core::{Catalog, Command, Completions, Expectation, ParseError, SelOp, Target};
use pult_plugin_sdk::{self as sdk, host, output_line, surface, PultPlugin};
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
            Command::Intensity { percent } => self.intensity(percent, ctx),
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
        ops: Vec<(SelOp, core::Range)>,
        at: Option<f64>,
        ctx: &Value,
    ) -> Result<surface::ExecResponse, String> {
        let fixtures = collection("fixtures")?;
        let mut selected: Vec<String> = selection_of(ctx);
        for (op, range) in ops {
            if range.to > fixtures.len() {
                return Err(format!(
                    "there are {} fixtures; {} is past the end",
                    fixtures.len(),
                    range.to
                ));
            }
            let ids = fixtures[range.from - 1..range.to]
                .iter()
                .filter_map(|f| f.get("id").and_then(Value::as_str).map(String::from));
            match op {
                SelOp::Replace => selected = ids.collect(),
                SelOp::Add => {
                    for id in ids {
                        if !selected.contains(&id) {
                            selected.push(id);
                        }
                    }
                }
                SelOp::Remove => {
                    let drop: Vec<String> = ids.collect();
                    selected.retain(|id| !drop.contains(id));
                }
            }
        }
        let count = selected.len();
        let mut text = match count {
            0 => "nothing selected".to_string(),
            1 => "1 fixture selected".to_string(),
            n => format!("{n} fixtures selected"),
        };
        // The combined form: `fixture 1 thru 3 @ 80` sets the level on what it
        // just selected, not on whatever was selected before.
        if let Some(percent) = at {
            if selected.is_empty() {
                return Err("that selects nothing, so there is nothing to set".into());
            }
            self.hold_intensity(&selected, percent)?;
            text.push_str(&format!(", at {}%", percent.round()));
        }
        let mut response = lines_response(vec![output_line("result", text)]);
        response.effects = Some(json!({ "selection": { "fixtureIds": selected } }));
        Ok(response)
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

    fn intensity(&self, percent: f64, ctx: &Value) -> Result<surface::ExecResponse, String> {
        let selected = selection_of(ctx);
        if selected.is_empty() {
            return Err("nothing is selected — `fixture 1 thru 5` first".into());
        }
        self.hold_intensity(&selected, percent)?;
        Ok(lines_response(vec![output_line(
            "result",
            format!(
                "{} fixture{} at {}%",
                selected.len(),
                if selected.len() == 1 { "" } else { "s" },
                percent.round()
            ),
        )]))
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
        let seq = host::get(&["sequences", &sequence_id])?;
        let cue_ids: Vec<String> = seq
            .get("cue_ids")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let entries = collection("programmer_values")?;
        if entries.is_empty() {
            return Err("the programmer is empty — nothing to store".into());
        }
        let captures: Vec<Value> = entries
            .iter()
            .map(|entry| {
                let mut effect = entry.get("effect").cloned().unwrap_or(Value::Null);
                if let Some(spec) = effect.as_object_mut() {
                    // A stored effect drops its anchor: the cue's `went_at` is
                    // what it is measured from on every Go.
                    spec.insert("t0".into(), Value::Null);
                }
                json!({
                    "fixture_id": entry.get("fixture_id"),
                    "parameter_kind": entry.get("parameter_kind"),
                    "value": entry.get("value"),
                    "fade_in_ms": 0,
                    "fade_out_ms": 0,
                    "delay_in_ms": 0,
                    "effect": effect,
                    "easing": "Linear",
                })
            })
            .collect();

        // An existing cue is merged into; a number one past the end (or a new
        // name) makes the next cue.
        let existing = match &cue {
            Target::Index(n) if *n <= cue_ids.len() => Some(cue_ids[n - 1].clone()),
            Target::Index(_) => None,
            Target::Name(name) => {
                let cues = collection("cues")?;
                cue_ids.iter().find_map(|id| {
                    let c = cues.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(id))?;
                    (c.get("name").and_then(Value::as_str) == Some(name.as_str()))
                        .then(|| id.clone())
                })
            }
        };

        match existing {
            Some(cue_id) => {
                let old = host::get(&["cues", &cue_id])?;
                let merged = merge_captures(
                    old.get("captures").and_then(Value::as_array).cloned().unwrap_or_default(),
                    captures,
                );
                host::set(&["cues", &cue_id, "captures"], &Value::Array(merged))?;
                let name = old.get("name").and_then(Value::as_str).unwrap_or("cue");
                Ok(lines_response(vec![output_line(
                    "result",
                    format!("stored into \"{name}\" of \"{sequence_name}\""),
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
                let cue_id = uuid::Uuid::new_v4().to_string();
                let payload = json!({
                    "id": cue_id,
                    "name": name,
                    "number": (cue_ids.len() + 1) as f64,
                    "captures": captures,
                    "follow_mode": "Manual",
                    "fade_in_ms": 500,
                    "fade_out_ms": 500,
                    "is_active": false,
                });
                host::set(&["cues", "__create"], &payload)?;
                let mut ids = cue_ids;
                ids.push(cue_id);
                host::set(&["sequences", &sequence_id, "cue_ids"], &json!(ids))?;
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
fn merge_captures(existing: Vec<Value>, stored: Vec<Value>) -> Vec<Value> {
    let key = |c: &Value| {
        format!(
            "{}/{}",
            c.get("fixture_id").and_then(Value::as_str).unwrap_or(""),
            c.get("parameter_kind").map(|k| k.to_string()).unwrap_or_default()
        )
    };
    let taken: Vec<String> = stored.iter().map(&key).collect();
    existing
        .into_iter()
        .filter(|c| !taken.contains(&key(c)))
        .chain(stored)
        .collect()
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
