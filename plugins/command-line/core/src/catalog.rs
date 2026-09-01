//! What the console said it can do, parsed once from introspection.

use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub entities: Vec<EntityInfo>,
    pub commands: Vec<CommandInfo>,
    pub rpcs: Vec<RpcInfo>,
}

#[derive(Debug, Clone)]
pub struct EntityInfo {
    /// The table name, which is also the path key: `"sequences"`.
    pub table: String,
    /// The Rust type name (`"Sequence"`), which names the id argument of the
    /// entity's commands: `sequenceId`.
    pub entity_name: String,
    pub is_singleton: bool,
    /// Field names, for `set` and its completion.
    pub fields: Vec<String>,
}

impl EntityInfo {
    /// `Sequence` → `sequenceId`: the argument a registered command reads its
    /// target from.
    pub fn id_arg(&self) -> String {
        let mut chars = self.entity_name.chars();
        let first = chars.next().map(|c| c.to_ascii_lowercase()).unwrap_or_default();
        format!("{first}{}Id", chars.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub table: String,
    /// The registered camelCase name: `"goNext"`.
    pub name: String,
    pub args: Vec<ArgInfo>,
    pub doc: String,
}

#[derive(Debug, Clone)]
pub struct RpcInfo {
    /// `"session.join"`.
    pub method: String,
    pub args: Vec<ArgInfo>,
    pub doc: String,
}

#[derive(Debug, Clone)]
pub struct ArgInfo {
    pub name: String,
    /// TypeScript's word for it, kept as text: `"string"`, `"number"`.
    pub ty: String,
    pub optional: bool,
}

impl Catalog {
    /// Build from the three introspection answers, exactly as the host serves
    /// them. Anything malformed is skipped rather than fatal: a command line
    /// with one verb missing beats no command line.
    pub fn from_introspection(entities: &Value, commands: &Value, rpcs: &Value) -> Catalog {
        let entities = entities
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some(EntityInfo {
                    table: e.get("tableName")?.as_str()?.to_string(),
                    entity_name: e
                        .get("entityName")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    is_singleton: e.get("isSingleton").and_then(Value::as_bool).unwrap_or(false),
                    fields: e
                        .get("fields")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|f| f.get("name")?.as_str().map(String::from))
                        .collect(),
                })
            })
            .collect();
        let commands = commands
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| {
                Some(CommandInfo {
                    table: c.get("table")?.as_str()?.to_string(),
                    name: c.get("name")?.as_str()?.to_string(),
                    args: parse_args(c.get("argsSchema")),
                    doc: c.get("doc").and_then(Value::as_str).unwrap_or("").to_string(),
                })
            })
            .collect();
        let rpcs = rpcs
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                Some(RpcInfo {
                    method: r.get("method")?.as_str()?.to_string(),
                    args: parse_args(r.get("argsSchema")),
                    doc: r.get("doc").and_then(Value::as_str).unwrap_or("").to_string(),
                })
            })
            .collect();
        Catalog { entities, commands, rpcs }
    }

    /// The entity a word names: the table itself, or its natural singular.
    /// `sequence`, `sequences` and `speed_master` all land where you expect.
    pub fn table_for(&self, word: &str) -> Option<&EntityInfo> {
        let wanted = normal(word);
        self.entities.iter().find(|e| {
            let table = normal(&e.table);
            let singular = table.strip_suffix('s').unwrap_or(&table);
            wanted == table || wanted == singular
        })
    }

    /// Every collection's friendly (singular) spelling, for completion.
    pub fn entity_words(&self) -> Vec<String> {
        self.entities
            .iter()
            .filter(|e| !e.is_singleton)
            .map(|e| singular_of(&e.table))
            .collect()
    }

    pub fn commands_of<'a>(&'a self, table: &str) -> impl Iterator<Item = &'a CommandInfo> + 'a {
        let table = table.to_string();
        self.commands.iter().filter(move |c| c.table == table)
    }

    /// The registered command a typed word means, on this table.
    ///
    /// Case and underscores are the operator's business (`gonext`, `go_next`),
    /// and the two verbs every console has get their classic spellings: `go`
    /// for `goNext`, `goto` for `goToCue` — but only where the table actually
    /// registers those, so the aliases can never shadow a real command.
    pub fn command_for(&self, table: &str, word: &str) -> Option<&CommandInfo> {
        let wanted = normal(word);
        self.commands_of(table)
            .find(|c| normal(&c.name) == wanted)
            .or_else(|| match wanted.as_str() {
                "go" => self.commands_of(table).find(|c| c.name == "goNext"),
                "goto" => self.commands_of(table).find(|c| c.name == "goToCue"),
                _ => None,
            })
    }

    /// First words of RPC methods: `session`, `device`.
    pub fn rpc_prefixes(&self) -> Vec<String> {
        let mut prefixes: Vec<String> = self
            .rpcs
            .iter()
            .filter_map(|r| r.method.split('.').next().map(String::from))
            .collect();
        prefixes.sort();
        prefixes.dedup();
        prefixes
    }

    pub fn rpc_for(&self, prefix: &str, word: &str) -> Option<&RpcInfo> {
        let method = format!("{prefix}.{word}");
        self.rpcs.iter().find(|r| r.method == method)
    }
}

/// Lowercased, underscores dropped: how words are compared throughout.
pub(crate) fn normal(word: &str) -> String {
    word.chars()
        .filter(|c| *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// The word an operator types for a table: one trailing `s` off.
pub(crate) fn singular_of(table: &str) -> String {
    table.strip_suffix('s').unwrap_or(table).to_string()
}

fn parse_args(schema: Option<&Value>) -> Vec<ArgInfo> {
    // argsSchema arrives either as a JSON array or as that array in a string,
    // depending on which registry served it; both read the same here.
    let owned;
    let arr = match schema {
        Some(Value::Array(a)) => Some(a),
        Some(Value::String(s)) => {
            owned = serde_json::from_str::<Value>(s).ok();
            owned.as_ref().and_then(Value::as_array)
        }
        _ => None,
    };
    arr.into_iter()
        .flatten()
        .filter_map(|a| {
            Some(ArgInfo {
                name: a.get("name")?.as_str()?.to_string(),
                ty: a.get("type").and_then(Value::as_str).unwrap_or("string").to_string(),
                optional: a.get("optional").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn test_catalog() -> Catalog {
    // A miniature of what introspection actually serves, used by the parser
    // and completion tests.
    let entities = serde_json::json!([
        { "entityName": "Fixture", "tableName": "fixtures", "isSingleton": false,
          "fields": [{"name": "id"}, {"name": "name"}, {"name": "live_values"}] },
        { "entityName": "Group", "tableName": "groups", "isSingleton": false,
          "fields": [{"name": "id"}, {"name": "name"}, {"name": "query"}] },
        { "entityName": "Sequence", "tableName": "sequences", "isSingleton": false,
          "fields": [{"name": "id"}, {"name": "name"}, {"name": "cue_ids"}] },
        { "entityName": "Cue", "tableName": "cues", "isSingleton": false,
          "fields": [{"name": "id"}, {"name": "name"}, {"name": "fade_time"}] },
        { "entityName": "SpeedMaster", "tableName": "speed_masters", "isSingleton": false,
          "fields": [{"name": "id"}, {"name": "bpm"}] },
        { "entityName": "Show", "tableName": "show", "isSingleton": true,
          "fields": [{"name": "name"}] }
    ]);
    let commands = serde_json::json!([
        { "table": "sequences", "name": "goNext", "doc": "Take the next cue.",
          "argsSchema": [{"name": "at", "type": "number", "optional": true}] },
        { "table": "sequences", "name": "goToCue", "doc": "Jump to a cue.",
          "argsSchema": [{"name": "cueId", "type": "string", "optional": false},
                          {"name": "at", "type": "number", "optional": true}] }
    ]);
    let rpcs = serde_json::json!([
        { "method": "session.join", "doc": "Join a session.",
          "argsSchema": [{"name": "sessionId", "type": "string", "optional": false}] },
        { "method": "device.adopt", "doc": "Adopt a device.",
          "argsSchema": [{"name": "serial", "type": "string", "optional": false}] }
    ]);
    Catalog::from_introspection(&entities, &commands, &rpcs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_finds_its_table_in_any_reasonable_spelling() {
        let catalog = test_catalog();
        for word in ["sequence", "sequences", "Sequence"] {
            assert_eq!(catalog.table_for(word).unwrap().table, "sequences");
        }
        for word in ["speedmaster", "speed_master", "speed_masters"] {
            assert_eq!(catalog.table_for(word).unwrap().table, "speed_masters");
        }
        assert!(catalog.table_for("banana").is_none());
    }

    #[test]
    fn the_classic_verbs_reach_the_registered_commands() {
        let catalog = test_catalog();
        assert_eq!(catalog.command_for("sequences", "go").unwrap().name, "goNext");
        assert_eq!(catalog.command_for("sequences", "goto").unwrap().name, "goToCue");
        assert_eq!(catalog.command_for("sequences", "GoNext").unwrap().name, "goNext");
        assert!(catalog.command_for("cues", "go").is_none());
    }
}
