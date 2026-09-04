//! The show, reached by name instead of by string.
//!
//! Introspection is the right wire and a poor thing to program against: a plugin
//! learns the schema as JSON, navigates it by hand, and spells every path as
//! `&["cues", id, "fade_in_ms"]` with nothing checking any of it. This module is the
//! other half — the same split the frontend has, where `data.cues[3].fadeInMs.set(4)`
//! rides an unchanged generic `set(path, json)`.
//!
//! ```ignore
//! use pult_plugin_sdk::data;
//!
//! let cue = data::cues().nth(3);
//! cue.fade_in_ms().set(4000)?;
//! cue.name().set("Blackout".to_string())?;
//! ```
//!
//! # What is here and what is generated
//!
//! This file is the runtime: [`Field`], [`Entity`], [`Collection`] and [`Singleton`]
//! know how to build a path and how to hand it to the host. The accessors over them —
//! `data::cues()`, `CueEntity::fade_in_ms` — are generated from the `EntityMeta` and
//! `CommandRegistration` inventories by `pult-codegen`, and re-exported at the bottom.
//!
//! # What this does not do
//!
//! It does not replace [`crate::host::entities`]. Typed accessors are what is known at
//! *build* time; introspection answers what this station has *now*, including a
//! collection this SDK never heard of. A command-line plugin building its grammar out
//! of the registry and a plugin walking unknown tables both still want the JSON.
//!
//! And it is not a promise about the station. The wire is generic, so a plugin built
//! against a newer schema compiles happily and fails at the one call that names a path
//! this station has not got — with the path and the type in the message, which is the
//! whole of what an author gets to debug with.

use std::borrow::Borrow;
use std::marker::PhantomData;

use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::host;

// ── Reading and writing one path ──────────────────────────────────────────────

fn refs(path: &[String]) -> Vec<&str> {
    path.iter().map(String::as_str).collect()
}

/// Read a path and deserialize it.
///
/// The station answers `null` for a path that holds nothing, so a missing path and a
/// path holding the wrong shape fail the same way — with the path, the type wanted and
/// serde's own complaint. That message is the failure mode of the whole typed layer:
/// a plugin built against a schema this station does not have gets it at one call
/// rather than at load time, deliberately, because the alternative is a bundle that
/// refuses to run on a console one version behind.
fn read<T: DeserializeOwned>(path: &[String]) -> Result<T, String> {
    let value = host::get(&refs(path))?;
    serde_json::from_value(value).map_err(|e| {
        format!("{}: the station holds nothing that reads as {} here ({e})", path.join("/"), std::any::type_name::<T>())
    })
}

fn write<T: Serialize>(path: &[String], value: &T) -> Result<(), String> {
    let json = serde_json::to_value(value).map_err(|e| e.to_string())?;
    host::set(&refs(path), &json)
}

fn extend(path: &[String], segment: impl Into<String>) -> Vec<String> {
    let mut next = Vec::with_capacity(path.len() + 1);
    next.extend_from_slice(path);
    next.push(segment.into());
    next
}

// ── Field ─────────────────────────────────────────────────────────────────────

/// One field of one row.
pub struct Field<T> {
    path: Vec<String>,
    _t: PhantomData<T>,
}

impl<T> Field<T> {
    fn new(path: Vec<String>) -> Self {
        Self { path, _t: PhantomData }
    }

    /// The path this field writes, as the station spells it.
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// Be told when this field changes; updates arrive at
    /// [`crate::PultPlugin::on_update`] carrying the returned token.
    pub fn subscribe(&self) -> u64 {
        host::subscribe(&self.path.join("/"))
    }
}

impl<T: DeserializeOwned> Field<T> {
    /// What the station holds here.
    pub fn get(&self) -> Result<T, String> {
        read(&self.path)
    }
}

impl<T: Serialize> Field<T> {
    /// Write it. Takes the value or a reference to one.
    pub fn set(&self, value: impl Borrow<T>) -> Result<(), String> {
        write(&self.path, value.borrow())
    }
}

impl<T: Nudgeable> Field<T> {
    /// Move it by `delta` rather than saying what it should become.
    ///
    /// Relative to what the station holds at the moment it applies the write, not to
    /// what this plugin last read — which is the point: two callers nudging one value
    /// at once both get their nudge. The station resolves it to an absolute write
    /// before anything records or replicates it, so the history, the showfile and
    /// every peer only ever see a destination.
    ///
    /// This is what `at +10` in the command line is, and it is why a plugin can answer
    /// "a bit darker" without being able to read the rig.
    pub fn by(&self, delta: f64) -> Result<(), String> {
        let path = extend(&self.path, "__by");
        host::set(&refs(&path), &serde_json::json!(delta))
    }
}

/// What a relative write means something for.
///
/// The station refuses `__by` on anything else, naming the path. Saying so in the type
/// system instead costs one trait and makes the refusal a compile error.
pub trait Nudgeable {}

macro_rules! nudgeable {
    ($($t:ty),*) => { $(impl Nudgeable for $t {})* };
}
nudgeable!(f32, f64, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
impl Nudgeable for pult_render::ParameterValue {}
impl<T: Nudgeable> Nudgeable for Option<T> {}

// ── Entity, Collection, Singleton ─────────────────────────────────────────────

/// One row, at a path. The generated per-entity accessors wrap one of these.
pub struct Entity {
    path: Vec<String>,
}

impl Entity {
    pub fn path(&self) -> &[String] {
        &self.path
    }

    pub fn get<T: DeserializeOwned>(&self) -> Result<T, String> {
        read(&self.path)
    }

    pub fn set<T: Serialize>(&self, value: &T) -> Result<(), String> {
        write(&self.path, value)
    }

    pub fn delete(&self) -> Result<(), String> {
        write(&extend(&self.path, "__delete"), &serde_json::Value::Null)
    }

    pub fn field<T>(&self, name: &str) -> Field<T> {
        Field::new(extend(&self.path, name))
    }

    /// Invoke a registered command on this row. A command is a write to a path whose
    /// last segment is the command's name, which is why it undoes and replicates like
    /// one.
    pub fn command<A: Serialize>(&self, name: &str, args: &A) -> Result<(), String> {
        write(&extend(&self.path, name), args)
    }

    pub fn subscribe_deep(&self) -> u64 {
        host::subscribe(&format!("{}/**", self.path.join("/")))
    }
}

/// A collection, at a path.
pub struct Collection {
    path: Vec<String>,
}

impl Collection {
    pub fn at(table: &str) -> Self {
        Self { path: vec![table.to_string()] }
    }

    pub fn path(&self) -> &[String] {
        &self.path
    }

    pub fn get<T: DeserializeOwned>(&self) -> Result<Vec<T>, String> {
        read(&self.path)
    }

    pub fn by_id(&self, id: Uuid) -> Entity {
        Entity { path: extend(&self.path, id.to_string()) }
    }

    pub fn nth(&self, index: usize) -> Entity {
        Entity { path: extend(&self.path, index.to_string()) }
    }

    pub fn create<T: Serialize>(&self, value: &T) -> Result<(), String> {
        write(&extend(&self.path, "__create"), value)
    }

    /// One of the collection verbs — `__home`, `__set_home`, `__checkpoint`.
    pub fn verb<A: Serialize>(&self, verb: &str, args: &A) -> Result<(), String> {
        write(&extend(&self.path, verb), args)
    }

    pub fn subscribe(&self) -> u64 {
        host::subscribe(&self.path.join("/"))
    }

    pub fn subscribe_deep(&self) -> u64 {
        host::subscribe(&format!("{}/**", self.path.join("/")))
    }
}

/// A singleton, at a path. `show` is the only one with a table.
pub struct Singleton {
    path: Vec<String>,
}

impl Singleton {
    pub fn at(table: &str) -> Self {
        Self { path: vec![table.to_string()] }
    }

    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// `None` where the path holds nothing — a console with no show open is a real
    /// state, not an error.
    pub fn get<T: DeserializeOwned>(&self) -> Result<Option<T>, String> {
        read(&self.path)
    }

    pub fn set<T: Serialize>(&self, value: &T) -> Result<(), String> {
        write(&self.path, value)
    }

    pub fn field<T>(&self, name: &str) -> Field<T> {
        Field::new(extend(&self.path, name))
    }

    pub fn subscribe(&self) -> u64 {
        host::subscribe(&format!("{}/**", self.path.join("/")))
    }
}

pub use crate::generated::data::*;

#[cfg(test)]
mod tests {
    use super::*;

    // Paths only. Anything that calls the host is a WASM import with no
    // implementation on this side of the boundary, so what is testable here is
    // exactly what is worth testing: that a typed accessor spells the path the
    // station parses.

    #[test]
    fn a_field_is_the_path_the_station_parses() {
        let id = Uuid::parse_str("2f6b535b-0000-4000-8000-000000000000").unwrap();
        let field = cues().by_id(id).fade_in_ms();
        assert_eq!(field.path(), ["cues", &id.to_string(), "fade_in_ms"]);
    }

    #[test]
    fn nth_is_a_position_and_the_host_reads_digits_as_one() {
        assert_eq!(sequences().nth(3).name().path(), ["sequences", "3", "name"]);
    }

    #[test]
    fn a_command_is_a_write_to_its_own_name() {
        let id = Uuid::nil();
        let entity = sequences().by_id(id);
        assert_eq!(entity.path(), ["sequences", &id.to_string()]);
        // `goNext` stays camelCase on the wire; only the method is snake_case.
        assert_eq!(
            extend(entity.path(), "goNext"),
            vec!["sequences".to_string(), id.to_string(), "goNext".to_string()]
        );
    }

    #[test]
    fn the_singleton_has_no_id_between_it_and_its_fields() {
        assert_eq!(show().haze_density().path(), ["show", "haze_density"]);
    }

    #[test]
    fn command_arguments_are_camel_case_and_leave_out_what_was_not_given() {
        let args = SequenceGoToCueArgs { cue_id: Uuid::nil().to_string(), at: None };
        assert_eq!(
            serde_json::to_value(&args).unwrap(),
            serde_json::json!({ "cueId": Uuid::nil().to_string() })
        );
    }
}
