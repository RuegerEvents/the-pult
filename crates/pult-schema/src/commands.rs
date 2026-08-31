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
    /// The same arguments as data: a JSON array of `{ "name", "type", "optional" }`,
    /// derived from `args_ts` at compile time. What a command line completes and
    /// validates against. Empty string where `args_ts` was too clever to parse —
    /// never hand-written, so `args_ts` stays the single source.
    pub args_schema: &'static str,
    /// The method's `///` comment, for help text. Empty where there is none.
    pub doc: &'static str,
    pub handler: fn(serde_json::Value, serde_json::Value) -> anyhow::Result<serde_json::Value>,
}

inventory::collect!(CommandRegistration);

/// The registration for a command name, if there is one.
///
/// Used to tell an edit from a button press: a path whose last segment names a
/// command is somebody pressing Go, and a path that does not is somebody changing
/// the show. Undo cares about the difference.
pub fn registered_command(name: &str) -> Option<&'static CommandRegistration> {
    inventory::iter::<CommandRegistration>
        .into_iter()
        .find(|registration| registration.command_name == name)
}
