//! What a parameter is doing, worked out from what is driving it and a moment.
//!
//! This crate is the console's one evaluator, and it exists as its own crate so that
//! it can be compiled twice: natively for the station, its output connectors and its
//! plugins, and to `wasm32-unknown-unknown` for the browser. `pult-schema` cannot
//! serve — its dependencies (sqlx, tokio, inventory) rule out a browser target — and
//! a second implementation in TypeScript would drift, which shows up as the screen
//! disagreeing with the lamps.
//!
//! So: nothing here touches an OS, a clock, or a filesystem. A moment is always an
//! argument. Everything is a pure function of what it is given, which is also what
//! makes two stations agree without exchanging a single value.

#![forbid(unsafe_code)]

pub mod color;
pub mod driving;
pub mod effect;
pub mod value;

pub use driving::{settles_at, value_at, Driving};
pub use effect::{
    blend, curve_level, cycle_position, ease, effect_value_at, fade_is_done, fade_progress,
    fade_value_at, step_value, Curve, Direction, Easing, EffectSource, RunningEffect, RunningFade,
    Shape, Step,
};
pub use color::{level_from, level_of, mix, Color, EmitterSpec};
pub use value::{interpolate, ParameterValue};
