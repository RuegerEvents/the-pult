/// A named command on a model entity, auto-submitted by #[pult_commands] + #[pult_command].
///
/// handler(entity_json, args_json) deserializes the entity, calls the annotated method,
/// and returns the full updated entity as JSON. The engine applies the result.
pub struct CommandRegistration {
    /// Fn returning the entity's table name (called at startup, not in static context).
    pub entity_table: fn() -> &'static str,
    /// camelCase path key used in Set paths: ["sequences", id, "goNext"].
    pub command_name: &'static str,
    pub is_public: bool,
    /// TypeScript argument type literal for codegen (empty string = no args → `(): Promise<void>`).
    /// Example: `"{ cueId: string }"` → `(args: { cueId: string }): Promise<void>`.
    pub args_ts: &'static str,
    pub handler: fn(serde_json::Value, serde_json::Value) -> anyhow::Result<serde_json::Value>,
}

inventory::collect!(CommandRegistration);
