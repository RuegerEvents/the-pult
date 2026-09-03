//! The pieces of a room the console can draw without being given a mesh.
//!
//! A `SceneObject` points at its geometry by sha, which is right for a drawing that
//! came out of somebody's Vectorworks file and useless for everything else. A console
//! that has never imported an MVR has no meshes at all, so a truss it makes for
//! itself is an empty group in the rig view and its lights hang in the air.
//!
//! So there is a small catalogue of standard pieces, named rather than modelled. A
//! `SceneObject` carrying `catalogue = "f34-2m"` is a two-metre length of the box
//! truss most of Europe hangs its rigs on, and the browser draws one — procedurally,
//! from the dimensions here, so it costs no download and no asset store.
//!
//! # What is here and what is not
//!
//! Dimensions and names. Not geometry: turning a `BoxTruss` 2 m long into chords and
//! bracing is a fact about drawing rather than about the show, and it lives in
//! `frontend/src/lib/stock.ts`. What this file guarantees is that the two ends agree
//! about what `f34-2m` *is* — the table is emitted to TypeScript by `pult-codegen`,
//! so there is one of it.
//!
//! An imported mesh always wins, and the MVR importer never guesses one of these: a
//! drawing's object says what it is with its mesh, and when the mesh did not come
//! with the file the honest answer is that this console does not know how long that
//! truss was. What the catalogue is for is a rig the console made itself — and for
//! anything that later wants to *say* which piece an object is, which is a thing a
//! scene editor would do and there is not one yet.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{fixture::Vec3, scene::SceneObjectKind};

/// How a piece is drawn. The browser switches on this rather than on the id, so a
/// new length of truss is a row in the table and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum StockShape {
    /// Four chords and bracing: a straight length of box truss.
    BoxTruss,
    /// The block that turns one run of truss into another. Drawn as a cube of
    /// chords, because that is what a two-way corner is.
    TrussCorner,
    /// A rostrum: a flat top on legs.
    Deck,
    /// A flat panel standing on its bottom edge — a wall, or a piece of scenery.
    Panel,
}

/// One piece the console can draw.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StockPiece {
    /// What a `SceneObject::catalogue` holds. Stable: it is written into showfiles.
    pub id: &'static str,
    /// What to call it in a list.
    pub title: &'static str,
    pub shape: StockShape,
    /// What kind of scene object one of these is, so a console making one does not
    /// have to decide separately and get it wrong.
    pub kind: SceneObjectKind,
    /// Its size in metres: along X, up Y, and along Z. A piece is drawn at this size
    /// and then scaled by the object's own transform like anything else.
    pub size: Vec3,
}

/// The width of an F34 chord square, in metres.
///
/// 290 mm, which is the figure the whole family is named for and the reason a light
/// clamped to one sits where it does. Kept as a constant because three of the
/// entries below are the same square at different lengths.
pub const F34: f32 = 0.29;

const fn metres(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

/// Everything the console can draw for itself.
///
/// Deliberately short. This is not a hire stock list — it is the handful of shapes
/// that make a rig read as a rig, and anything more particular belongs in an MVR
/// somebody drew.
pub const CATALOGUE: &[StockPiece] = &[
    StockPiece {
        id: "f34-1m",
        title: "F34 truss 1 m",
        shape: StockShape::BoxTruss,
        kind: SceneObjectKind::Truss,
        size: metres(1.0, F34, F34),
    },
    StockPiece {
        id: "f34-2m",
        title: "F34 truss 2 m",
        shape: StockShape::BoxTruss,
        kind: SceneObjectKind::Truss,
        size: metres(2.0, F34, F34),
    },
    StockPiece {
        id: "f34-3m",
        title: "F34 truss 3 m",
        shape: StockShape::BoxTruss,
        kind: SceneObjectKind::Truss,
        size: metres(3.0, F34, F34),
    },
    StockPiece {
        id: "f34-corner",
        title: "F34 corner",
        shape: StockShape::TrussCorner,
        kind: SceneObjectKind::Truss,
        size: metres(F34, F34, F34),
    },
    StockPiece {
        id: "deck-2x1",
        title: "Stage deck 2 × 1 m",
        shape: StockShape::Deck,
        kind: SceneObjectKind::Object,
        // 200 mm, a deck's own thickness. How high it stands is its legs, which is
        // the object's position — a deck is not a taller deck for being on a riser.
        size: metres(2.0, 0.2, 1.0),
    },
    StockPiece {
        id: "deck-1x1",
        title: "Stage deck 1 × 1 m",
        shape: StockShape::Deck,
        kind: SceneObjectKind::Object,
        size: metres(1.0, 0.2, 1.0),
    },
    StockPiece {
        id: "wall-2x1",
        title: "Wall panel 2 × 1 m",
        shape: StockShape::Panel,
        kind: SceneObjectKind::Object,
        size: metres(2.0, 1.0, 0.05),
    },
    StockPiece {
        id: "flat-1x24",
        title: "Flat 1 × 2.4 m",
        shape: StockShape::Panel,
        kind: SceneObjectKind::Object,
        // The standard flat: a metre wide and eight foot tall in metres.
        size: metres(1.0, 2.4, 0.05),
    },
];

/// One piece by the id a `SceneObject` carries, or `None` for a name this build has
/// never heard of — which is drawn as nothing rather than refused, the rule a layout
/// already follows for a panel id it does not know.
pub fn piece(id: &str) -> Option<&'static StockPiece> {
    CATALOGUE.iter().find(|piece| piece.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_piece_can_be_found_by_the_id_a_showfile_would_carry() {
        for piece in CATALOGUE {
            assert_eq!(self::piece(piece.id).map(|found| found.id), Some(piece.id));
        }
        assert!(self::piece("nothing-like-it").is_none());
    }

    #[test]
    fn the_ids_are_unique_and_the_sizes_are_real() {
        // The id goes into showfiles, so two entries sharing one would be two
        // different things drawn as whichever the search found first.
        let mut seen: Vec<&str> = CATALOGUE.iter().map(|piece| piece.id).collect();
        seen.sort_unstable();
        let unique = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), unique, "two pieces share an id");

        for piece in CATALOGUE {
            let size = piece.size;
            assert!(
                size.x > 0.0 && size.y > 0.0 && size.z > 0.0,
                "{} has no size to be drawn at",
                piece.id,
            );
        }
    }

    #[test]
    fn a_truss_is_the_square_it_is_named_for() {
        // F34 is 290 mm, which is why a clamp sits where it does. A length that
        // quietly stopped being that would put every light in the rig somewhere else.
        for id in ["f34-1m", "f34-2m", "f34-3m"] {
            let truss = piece(id).expect("a listed truss");
            assert_eq!((truss.size.y, truss.size.z), (F34, F34), "{id}");
            assert_eq!(truss.kind, SceneObjectKind::Truss);
        }
    }
}
