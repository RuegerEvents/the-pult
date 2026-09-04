//! Natural language in, command lines out.
//!
//! The model never touches the show. It is handed the command line's own
//! grammar and a short summary of what exists, and asked to answer in command
//! lines; whatever comes back is executed through the command-line plugin with
//! the caller's own context — same selection, same user, same undo history.
//! The command line is the safety boundary: anything the model invents that
//! is not grammatical fails loudly there, with a span.

mod http;

use pult_plugin_sdk::{self as sdk, data, host, output_line, surface, PultPlugin};
use serde_json::{json, Value};

const SYSTEM_PROMPT: &str = "You operate a lighting console through its command line. \
Answer with ONLY the command lines to run, one per line, no prose, no code fences. \
Use the reference below for what exists. If the request cannot be done with these \
commands, answer with one line starting with `!` explaining why, briefly.";

struct NaturalLanguage {
    provider: Provider,
    /// The command line's grammar, fetched once at init — the model's manual.
    grammar: String,
    /// Configuration as the layers composed it, before this machine's own
    /// choice was laid over the top. Kept so that choosing again starts from
    /// the same place rather than from the last answer.
    config: Value,
}

/// Where this console remembers the operator's choice of model.
///
/// Station-scoped, and the reason is the whole distinction: which model is
/// installed on *this* machine, or which provider this operator prefers at this
/// desk, is not a fact about the show. Put it in a show-scoped store and it
/// replicates to a console that has no Ollama on it, travels in the showfile,
/// and lands in every backup.
///
/// Deliberately not a cache of anything derived. A cache would be the easier
/// example and the wrong one: derived data is cheap to rebuild and a stale copy
/// is worse than none, so an example built on it argues against its own feature.
/// This is state — nothing can recompute what the operator picked.
const PREFS: &str = "prefs";

/// The composed configuration with this machine's remembered choice over it.
///
/// The most specific layer wins, and a choice made at this desk a moment ago is
/// as specific as it gets — more so than the station's `preferences.toml`, which
/// is what this console was set up with rather than what the operator just
/// asked for.
fn with_remembered_choice(config: &Value) -> Value {
    let mut config = config.clone();
    let Some(table) = config.as_object_mut() else { return config };
    for key in ["provider", "model", "base_url"] {
        match sdk::store::get::<String>(PREFS, key) {
            Ok(Some(value)) if !value.is_empty() => {
                table.insert(key.to_string(), Value::String(value));
            }
            Ok(_) => {}
            // A store that will not answer is not a reason to refuse to start:
            // the configured provider is a perfectly good answer.
            Err(e) => sdk::log_warn!("could not read the remembered {key}: {e}"),
        }
    }
    config
}

struct Provider {
    base_url: String,
    model: String,
    api_key: Option<String>,
    label: String,
}

impl Provider {
    fn from_config(config: &Value) -> Result<Provider, String> {
        let name = config.get("provider").and_then(Value::as_str).unwrap_or("ollama");
        let (default_base, default_model, wants_key) = match name {
            "ollama" => ("http://localhost:11434/v1", "qwen3:4b", false),
            "openrouter" => ("https://openrouter.ai/api/v1", "anthropic/claude-haiku-4.5", true),
            "openai" => ("https://api.openai.com/v1", "gpt-4.1-mini", true),
            other => {
                return Err(format!(
                    "unknown provider {other:?} — ollama, openrouter or openai (or set base_url)"
                ))
            }
        };
        let base_url = match config.get("base_url").and_then(Value::as_str) {
            Some(url) if !url.is_empty() => url.to_string(),
            _ => default_base.to_string(),
        };
        let model = match config.get("model").and_then(Value::as_str) {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => default_model.to_string(),
        };
        let api_key = config
            .get("api_key_env")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .and_then(|name| std::env::var(name).ok())
            .filter(|key| !key.is_empty());
        if wants_key && api_key.is_none() {
            sdk::log_warn!(
                "{name} usually needs an API key; set the env var named by api_key_env"
            );
        }
        Ok(Provider {
            base_url,
            label: format!("{name} · {model}"),
            model,
            api_key,
        })
    }
}

impl PultPlugin for NaturalLanguage {
    fn init(config: Value) -> Result<Self, String> {
        let provider = Provider::from_config(&with_remembered_choice(&config))?;
        // The dependency is running — the manager loads it first or not us.
        let grammar = host::call_plugin("command-line", "grammar", &json!({}))?;
        let grammar = grammar
            .get("text")
            .and_then(Value::as_str)
            .ok_or("the command line answered `grammar` with no text")?
            .to_string();
        sdk::log_info!("natural language ready, speaking to {}", provider.label);
        Ok(NaturalLanguage { provider, grammar, config })
    }

    fn handle(&mut self, method: &str, args: Value, _ctx: Value) -> Result<Value, String> {
        match method {
            "surface.exec" | "exec" => {
                let text = args
                    .get("line")
                    .and_then(Value::as_str)
                    .ok_or("exec takes { \"line\": \"...\" }")?;
                let response = self.run(text);
                serde_json::to_value(&response).map_err(|e| e.to_string())
            }
            // A bar has no completions worth offering; the whole point is
            // typing whatever you mean.
            "surface.complete" => Ok(json!({ "items": [], "replaceFrom": 0 })),
            "surface.help" => Ok(json!({ "text": self.help_text() })),
            // Which model this console talks to, and a way to change it that
            // outlives the session. The answer is remembered on this machine,
            // so the next start speaks to the same one without being told.
            "provider" => Ok(json!({ "label": self.provider.label })),
            "use" => self.use_provider(args),
            _ => Err(format!("natural language has no method called {method:?}")),
        }
    }
}

sdk::plugin_main!(NaturalLanguage);

impl NaturalLanguage {
    /// Point this console at a different model, and remember that it was asked.
    ///
    /// The provider is built before anything is written, so a name that is not
    /// a provider is refused with the same message it would get at startup and
    /// nothing is remembered — a console cannot be left unable to start by one
    /// mistyped word.
    fn use_provider(&mut self, args: Value) -> Result<Value, String> {
        let mut asked = self.config.clone();
        let Some(table) = asked.as_object_mut() else {
            return Err("this plugin's configuration is not a table".into());
        };
        for key in ["provider", "model", "base_url"] {
            if let Some(value) = args.get(key).and_then(Value::as_str) {
                table.insert(key.to_string(), Value::String(value.to_string()));
            }
        }

        let provider = Provider::from_config(&asked)?;
        for key in ["provider", "model", "base_url"] {
            if let Some(value) = args.get(key).and_then(Value::as_str) {
                sdk::store::set(PREFS, key, &value)?;
            }
        }
        sdk::log_info!("now speaking to {}", provider.label);
        self.provider = provider;
        Ok(json!({ "label": self.provider.label }))
    }

    fn run(&self, text: &str) -> surface::ExecResponse {
        match self.commands_for(text) {
            Ok(commands) => self.execute(commands),
            Err(message) => surface::ExecResponse {
                lines: Vec::new(),
                error: Some(surface::ExecError { message, span: None, expected: Vec::new() }),
                effects: None,
            },
        }
    }

    /// One round trip to the model, and its reply read as command lines.
    fn commands_for(&self, text: &str) -> Result<Vec<String>, String> {
        let user = format!(
            "# Command reference\n\n{}\n\n# What the show holds right now\n\n{}\n\n# Request\n\n{}",
            self.grammar,
            show_summary(),
            text
        );
        let reply = http::chat(&http::ChatRequest {
            base_url: &self.provider.base_url,
            model: &self.provider.model,
            api_key: self.provider.api_key.as_deref(),
            system: SYSTEM_PROMPT,
            user: &user,
        })?;

        let mut commands = Vec::new();
        for line in reply.lines() {
            let line = line.trim().trim_start_matches("```").trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(reason) = line.strip_prefix('!') {
                return Err(format!("the model declined: {}", reason.trim()));
            }
            commands.push(line.to_string());
        }
        if commands.is_empty() {
            return Err("the model answered with no commands".into());
        }
        Ok(commands)
    }

    /// Run each command through the command line, stopping at the first error:
    /// a half-applied interpretation that carried on would be worse than one
    /// that stopped where it went wrong.
    fn execute(&self, commands: Vec<String>) -> surface::ExecResponse {
        let mut lines = Vec::new();
        let mut effects: Option<Value> = None;
        for command in commands {
            lines.push(output_line("info", format!("> {command}")));
            match host::call_plugin("command-line", "exec", &json!({ "line": command })) {
                Ok(result) => {
                    for line in result.get("lines").and_then(Value::as_array).into_iter().flatten() {
                        lines.push(output_line(
                            line.get("kind").and_then(Value::as_str).unwrap_or("result"),
                            line.get("text").and_then(Value::as_str).unwrap_or(""),
                        ));
                    }
                    if let Some(error) = result.get("error").and_then(|e| e.get("message")) {
                        lines.push(output_line(
                            "error",
                            error.as_str().unwrap_or("failed").to_string(),
                        ));
                        break;
                    }
                    // The last selection change wins, same as typing the
                    // commands by hand.
                    if let Some(e) = result.get("effects") {
                        if !e.is_null() {
                            effects = Some(e.clone());
                        }
                    }
                }
                Err(e) => {
                    lines.push(output_line("error", e));
                    break;
                }
            }
        }
        surface::ExecResponse { lines, error: None, effects }
    }

    fn help_text(&self) -> String {
        format!(
            "Type what you want in plain language — \"take the first five \
fixtures to 80 percent\", \"next cue on the main sequence\".\n\n\
A model turns it into command lines and runs them through the Command \
Line plugin; the transcript shows exactly what ran, and undo takes it \
back like anything else.\n\n\
Speaking to: {}\nConfigured in the plugin's config.toml.",
            self.provider.label
        )
    }
}

/// What exists, briefly — enough for the model to name things, small enough
/// to cost nothing.
///
/// Read through the typed accessors rather than by hand: `f.get("name")` on a
/// `Value` is three fallbacks deep and answers `"?"` for a field this build spelled
/// wrong, which is exactly the kind of quiet wrongness a prompt made of it carries
/// into the model.
fn show_summary() -> String {
    let mut out = String::new();
    if let Ok(fixtures) = data::fixtures().get() {
        let names: Vec<String> = fixtures
            .iter()
            .take(60)
            .enumerate()
            .map(|(i, f)| format!("{}:{}", i + 1, f.name))
            .collect();
        out.push_str(&format!("Fixtures ({}): {}\n", names.len(), names.join(", ")));
    }
    if let Ok(sequences) = data::sequences().get() {
        let rows: Vec<String> = sequences
            .iter()
            .take(30)
            .enumerate()
            .map(|(i, s)| format!("{}:{} ({} cues)", i + 1, s.name, s.cue_ids.len()))
            .collect();
        out.push_str(&format!("Sequences: {}\n", rows.join(", ")));
    }
    if out.is_empty() {
        out.push_str("The show is empty.\n");
    }
    out
}
