// Sequence commands (go_next, go_to_cue) live in pult-schema/src/types/sequence.rs
// because Rust's orphan rule requires inherent impls to be in the same crate as the type.
// They are auto-registered via #[pult_commands] + inventory and dispatched here by the engine.
