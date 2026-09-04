//! The catalogue's pieces, as bytes somebody else can open.
//!
//! Until now a stock piece was a shape the *browser* drew: `stock.ts` built cylinders
//! from the dimensions in [`crate::types::catalogue`], and that was the whole of what
//! an `f34-2m` looked like. Which meant an exported MVR carried an empty group where
//! the truss was — MVR has no primitive, its `GeometryNode` is a file or a symbol
//! instance — so a rig built here and opened in Vectorworks was a room full of
//! nothing. A from-scratch rig is *all* stock pieces, so that was the whole rig.
//!
//! So there is one implementation, and it is this one: the station generates a `.glb`
//! from the table, the browser loads it through `geometry.ts` like any other mesh, and
//! an export writes the same bytes into the archive. What somebody opens is what the
//! rig view drew.
//!
//! # A pure function, not an asset
//!
//! [`stock_glb`] takes an id and the properties and answers bytes. It is not put in
//! the asset store and it is not replicated, for three reasons: the store refuses a
//! write when no show is open and the welcome screen still draws a rig; a generated
//! mesh that outlived the code that generated it would be a stale asset nobody could
//! explain; and there is nothing to fetch from a peer that this station cannot make
//! for itself in a hundred microseconds. `GET /stock/{id}.glb` serves it with a strong
//! ETag over the bytes, so a browser asks once.
//!
//! # Deterministic, and that is a gate
//!
//! The same id and the same canonical properties must give the same bytes on every
//! station and on every run: the ETag is a hash of them, the MVR export names the file
//! after that hash, and the symdef uuid is a v5 over the same name. So no map is
//! iterated, no float is formatted, and every buffer is written in a fixed order.
//! `crates/pult-schema/tests` generates every piece twice and compares.

mod glb;
mod shapes;

use serde_json::Value;
use uuid::Uuid;

use crate::types::catalogue::{canonical_properties, piece};

pub use glb::{Mesh, GLB_MIME};

/// The bytes of one catalogue piece as a binary glTF, or `None` for an id this build
/// has never heard of.
///
/// `properties` is brought to its canonical form here, so a caller that has not done
/// so gets the same answer as one that has — which matters, because the two callers
/// are an HTTP route and an MVR export and they must not disagree about what a deck
/// with no legs said looks like.
pub fn stock_glb(id: &str, properties: &Value) -> Option<Vec<u8>> {
    let piece = piece(id)?;
    let canonical = canonical_properties(piece, properties);
    Some(glb::write(&shapes::build(piece, &canonical)))
}

/// What the archive calls the symdef a stock piece is exported as.
///
/// Carries the piece id and the canonical properties, so two decks at two leg heights
/// are two symbols and two decks at one height are one. It is read back on import by
/// [`parse_stock_symdef`], which is what makes a round trip give back a catalogue
/// piece rather than a mesh.
pub fn stock_symdef_name(id: &str, properties: &Value) -> String {
    let canonical = match piece(id) {
        Some(found) => canonical_properties(found, properties),
        None => Value::Object(serde_json::Map::new()),
    };
    // `serde_json` writes a `Map` in insertion order and `canonical_properties`
    // inserts in the piece's declared order, so this string is stable.
    format!("{PREFIX}{id}:{canonical}")
}

/// And the uuid that name always has.
///
/// A v5 rather than a fresh one, so exporting the same rig twice writes the same file
/// and a re-import matches rather than duplicating — the rule every other id in the
/// MVR path follows.
pub fn stock_symdef_uuid(name: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes())
}

/// The piece and the properties a symdef was written from, if this console wrote it.
///
/// **The uuid is checked against the name**, and that is the point of the pair rather
/// than tidiness: anybody may write a symdef called anything, and a drawing that
/// happened to name one `pult-stock:f34-2m:{}` would otherwise have its mesh thrown
/// away and replaced by this console's own idea of a two-metre truss. A file that
/// went through this console carries both halves and matches; anything else is an
/// ordinary symbol with an ordinary mesh, which is what it is.
pub fn parse_stock_symdef(name: &str, uuid: Uuid) -> Option<(String, Value)> {
    let rest = name.strip_prefix(PREFIX)?;
    let (id, json) = rest.split_once(':')?;
    if stock_symdef_uuid(name) != uuid {
        return None;
    }
    let found = piece(id)?;
    let given: Value = serde_json::from_str(json).ok()?;
    Some((found.id.to_string(), canonical_properties(found, &given)))
}

/// What every stock symdef's name begins with. Long enough that nothing else is
/// plausibly called it, and readable, because it is what somebody sees in a drawing.
const PREFIX: &str = "pult-stock:";

/// The name the archive entry gets: the piece, and eight characters of the digest of
/// its properties, so two decks at two heights are two files with readable names.
///
/// Not the sha of the bytes: the file name goes in the scene description, which is
/// written before the bytes are asked for.
pub fn stock_file_name(id: &str, properties: &Value) -> String {
    let name = stock_symdef_name(id, properties);
    let uuid = stock_symdef_uuid(&name);
    let hash = uuid.simple().to_string();
    format!("stock-{id}-{}.glb", &hash[..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::catalogue::CATALOGUE;

    /// The gate the ETag, the archive entry name and the symdef uuid all rest on.
    #[test]
    fn every_piece_generates_the_same_bytes_twice() {
        for piece in CATALOGUE {
            let once = stock_glb(piece.id, &Value::Null).expect("a listed piece draws");
            let again = stock_glb(piece.id, &Value::Null).expect("a listed piece draws");
            assert_eq!(once, again, "{} drew differently the second time", piece.id);
            assert!(once.len() > 100, "{} drew almost nothing", piece.id);
        }
    }

    #[test]
    fn a_name_this_build_does_not_know_draws_nothing() {
        assert!(stock_glb("f34-40m", &Value::Null).is_none());
    }

    /// The round trip the MVR import rests on.
    #[test]
    fn a_symdef_name_says_which_piece_it_was() {
        let properties = serde_json::json!({ "leg_height": 0.6 });
        let name = stock_symdef_name("deck-2x1", &properties);
        let uuid = stock_symdef_uuid(&name);

        let (id, back) = parse_stock_symdef(&name, uuid).expect("this console wrote it");
        assert_eq!(id, "deck-2x1");
        assert_eq!(back, serde_json::json!({ "leg_height": 0.6 }));
    }

    /// A name that says the right thing under a uuid that does not is somebody else's
    /// symbol, and its mesh is the truth about it.
    #[test]
    fn a_name_whose_uuid_is_wrong_is_not_trusted() {
        let name = stock_symdef_name("f34-2m", &Value::Null);
        assert!(parse_stock_symdef(&name, Uuid::nil()).is_none());
        assert!(parse_stock_symdef("Truss 2m", stock_symdef_uuid("Truss 2m")).is_none());
    }

    /// Two decks that were asked for the same thing in two ways are one symbol; two
    /// that were asked for different things are two.
    #[test]
    fn the_name_follows_what_was_asked_for_and_not_how_it_was_spelled() {
        let plain = stock_symdef_name("deck-2x1", &Value::Null);
        let spelled_out = stock_symdef_name("deck-2x1", &serde_json::json!({ "leg_height": 0.2 }));
        let noisy =
            stock_symdef_name("deck-2x1", &serde_json::json!({ "leg_height": 0.2, "hue": "red" }));
        assert_eq!(plain, spelled_out);
        assert_eq!(plain, noisy);

        let taller = stock_symdef_name("deck-2x1", &serde_json::json!({ "leg_height": 0.8 }));
        assert_ne!(plain, taller);
        assert_ne!(stock_file_name("deck-2x1", &Value::Null), stock_file_name("deck-2x1", &serde_json::json!({ "leg_height": 0.8 })));
    }
}
