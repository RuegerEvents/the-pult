//! The pieces of a room the console can draw without being given a mesh.
//!
//! A `SceneObject` points at its geometry by sha, which is right for a drawing that
//! came out of somebody's Vectorworks file and useless for everything else. A console
//! that has never imported an MVR has no meshes at all, so a truss it makes for
//! itself is an empty group in the rig view and its lights hang in the air.
//!
//! So there is a small catalogue of standard pieces, named rather than modelled. A
//! `SceneObject` carrying `catalogue = "f34-2m"` is a two-metre length of the box
//! truss most of Europe hangs its rigs on.
//!
//! # What is here
//!
//! Four things, and each of them is a fact about the piece rather than about a screen:
//!
//! - **Dimensions and names**, which is what the table has always carried.
//! - **Connectors** — where one piece bolts to another, and what kind of joint that
//!   is. A `TrussEnd` mates a `TrussEnd`, never a `DeckEdge`, and the mating puts the
//!   two points together facing opposite ways. That is the whole of the snapping rule.
//! - **Chords** — the lines a clamp can go round, so a light dragged near a bar knows
//!   what it would be clamped to. See [`crate::types::mount`].
//! - **Properties** — the questions a piece asks about itself. A deck has legs and the
//!   operator says how long they are; a truss asks nothing. Declared here so the sheet
//!   that offers them and the geometry that reads them cannot disagree.
//!
//! The **geometry** is here too now, in [`crate::stock`], and it was not before: the
//! browser used to draw cylinders from these dimensions, which meant an exported MVR
//! carried an empty group where the truss was. One implementation, generated on the
//! station and loaded by the browser, so the bytes somebody opens in Vectorworks are
//! the bytes the rig view drew.
//!
//! The table is emitted to TypeScript by `pult-codegen`, so there is one of it.
//!
//! An imported mesh always wins, and the MVR importer never guesses one of these: a
//! drawing's object says what it is with its mesh, and when the mesh did not come
//! with the file the honest answer is that this console does not know how long that
//! truss was.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use super::mount::Chord;
use super::{fixture::Vec3, scene::SceneObjectKind};

/// How a piece is drawn. The browser switches on this rather than on the id, so a
/// new length of truss is a row in the table and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum StockShape {
    /// Four chords and bracing: a straight length of box truss.
    BoxTruss,
    /// The block that turns one run of truss into another. A cube with a truss end on
    /// every face, so one corner does every angle rather than there being a left one
    /// and a right one and an up one.
    TrussCorner,
    /// A rostrum: a flat top on legs.
    Deck,
    /// A flat panel standing on its bottom edge — a wall, or a piece of scenery.
    Panel,
    /// A single tube: a scaff bar, a lighting pipe.
    Pipe,
    /// What a tower stands on: a plate with one truss end pointing up.
    BasePlate,
    /// And what caps it: a plate with one truss end pointing down.
    TopPlate,
}

/// What one piece bolts to another with.
///
/// Like mates like and nothing else. A truss end will not go on a deck edge however
/// close somebody drags it, which is what stops a snap radius from turning into a
/// guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ConnectorKind {
    /// The bolted end of a length of box truss.
    TrussEnd,
    /// A coupler on the end of a pipe.
    PipeEnd,
    /// The edge of a deck, where the next one goes.
    DeckEdge,
}

/// Where a piece joins another, in its own frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Connector {
    /// The point the joint is at, in metres.
    pub at: Vec3,
    /// Which way it faces — outwards, away from the piece. Two connectors mate when
    /// their points meet and their facings are opposite.
    pub facing: Vec3,
    pub kind: ConnectorKind,
}

impl Connector {
    const fn new(at: Vec3, facing: Vec3, kind: ConnectorKind) -> Self {
        Connector { at, facing, kind }
    }
}

/// What kind of answer a property wants, and what an editor should offer for it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub enum PropertyKind {
    Number { min: f32, max: f32, step: f32, unit: &'static str },
    Choice { options: &'static [&'static str] },
    Bool,
}

/// One question a piece asks about itself.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Property {
    /// What the key is called in `SceneObject::properties`. Stable: it goes into
    /// showfiles and into the name a symdef is exported under.
    pub key: &'static str,
    pub title: &'static str,
    pub kind: PropertyKind,
    /// What it is when nobody said. Filled in by [`canonical_properties`], so the
    /// geometry never has to cope with a missing key.
    pub default: f32,
}

/// One piece the console can draw.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
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
    /// Where it bolts to its neighbours.
    pub connectors: &'static [Connector],
    /// The lines a clamp can go round. Empty for anything nothing hangs off.
    pub chords: &'static [Chord],
    /// What it asks about itself.
    pub properties: &'static [Property],
}

/// The width of an F34 chord square, in metres.
///
/// 290 mm, which is the figure the whole family is named for and the reason a light
/// clamped to one sits where it does. Kept as a constant because three of the
/// entries below are the same square at different lengths.
pub const F34: f32 = 0.29;

/// A scaff bar: 48.3 mm, the diameter every hook clamp is cut for.
pub const PIPE_DIAMETER: f32 = 0.0483;

/// How thick a base or top plate's steel is, in metres.
pub const PLATE_STEEL: f32 = 0.03;

/// And how tall the whole fitting is: the plate plus the stubs the truss bolts to.
///
/// A piece's `size` is what it measures, so the stubs are in the figure — a base
/// plate whose size stopped at the steel would put a tower 60 mm into the floor.
pub const PLATE_HEIGHT: f32 = PLATE_STEEL + 0.06;

const fn metres(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

const HALF: f32 = F34 / 2.0;

/// The four chords of a box truss, bottom pair first.
///
/// Bottom first because chord 0 is what a mount defaults to, and a default that put
/// every light on top of the bar would be a rig hung upside down.
const BOX_CHORDS: &[Chord] = &[
    Chord::at(-HALF, -HALF),
    Chord::at(-HALF, HALF),
    Chord::at(HALF, -HALF),
    Chord::at(HALF, HALF),
];

/// A pipe's one chord: the tube itself.
const PIPE_CHORDS: &[Chord] = &[Chord::at(0.0, 0.0)];

/// The two ends of a one-metre length of box truss, and so on for the others.
macro_rules! truss_ends {
    ($half:expr) => {
        &[
            Connector::new(
                metres(-$half, 0.0, 0.0),
                metres(-1.0, 0.0, 0.0),
                ConnectorKind::TrussEnd,
            ),
            Connector::new(
                metres($half, 0.0, 0.0),
                metres(1.0, 0.0, 0.0),
                ConnectorKind::TrussEnd,
            ),
        ]
    };
}

macro_rules! pipe_ends {
    ($half:expr) => {
        &[
            Connector::new(
                metres(-$half, 0.0, 0.0),
                metres(-1.0, 0.0, 0.0),
                ConnectorKind::PipeEnd,
            ),
            Connector::new(metres($half, 0.0, 0.0), metres(1.0, 0.0, 0.0), ConnectorKind::PipeEnd),
        ]
    };
}

/// Six truss ends, one per face of the corner block.
///
/// Six rather than the two a two-way corner has, and that is the decision the whole
/// piece rests on: a run that turns, a run that goes up, and a tower that meets a bar
/// are all one part, so there is nothing to choose between a left corner and a right
/// one. It also makes a spigot kind unnecessary — a base plate is one truss end
/// pointing up, and a top plate is one pointing down.
const CORNER_ENDS: &[Connector] = &[
    Connector::new(metres(-HALF, 0.0, 0.0), metres(-1.0, 0.0, 0.0), ConnectorKind::TrussEnd),
    Connector::new(metres(HALF, 0.0, 0.0), metres(1.0, 0.0, 0.0), ConnectorKind::TrussEnd),
    Connector::new(metres(0.0, -HALF, 0.0), metres(0.0, -1.0, 0.0), ConnectorKind::TrussEnd),
    Connector::new(metres(0.0, HALF, 0.0), metres(0.0, 1.0, 0.0), ConnectorKind::TrussEnd),
    Connector::new(metres(0.0, 0.0, -HALF), metres(0.0, 0.0, -1.0), ConnectorKind::TrussEnd),
    Connector::new(metres(0.0, 0.0, HALF), metres(0.0, 0.0, 1.0), ConnectorKind::TrussEnd),
];

/// The four edges of a deck, which is what makes two of them a stage rather than two
/// decks.
macro_rules! deck_edges {
    ($x:expr, $z:expr) => {
        &[
            Connector::new(metres(-$x, 0.0, 0.0), metres(-1.0, 0.0, 0.0), ConnectorKind::DeckEdge),
            Connector::new(metres($x, 0.0, 0.0), metres(1.0, 0.0, 0.0), ConnectorKind::DeckEdge),
            Connector::new(metres(0.0, 0.0, -$z), metres(0.0, 0.0, -1.0), ConnectorKind::DeckEdge),
            Connector::new(metres(0.0, 0.0, $z), metres(0.0, 0.0, 1.0), ConnectorKind::DeckEdge),
        ]
    };
}

/// How long a deck's legs are. The one property anything in this catalogue has, and
/// it earns its place: a stage is decks at two heights, and a deck whose legs could
/// not be said would be a stage that is all one level.
const LEG_HEIGHT: &[Property] = &[Property {
    key: "leg_height",
    title: "Leg height",
    kind: PropertyKind::Number { min: 0.0, max: 1.2, step: 0.2, unit: "m" },
    default: 0.2,
}];

const NOTHING: &[Property] = &[];
const NO_CONNECTORS: &[Connector] = &[];
const NO_CHORDS: &[Chord] = &[];

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
        connectors: truss_ends!(0.5),
        chords: BOX_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "f34-2m",
        title: "F34 truss 2 m",
        shape: StockShape::BoxTruss,
        kind: SceneObjectKind::Truss,
        size: metres(2.0, F34, F34),
        connectors: truss_ends!(1.0),
        chords: BOX_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "f34-3m",
        title: "F34 truss 3 m",
        shape: StockShape::BoxTruss,
        kind: SceneObjectKind::Truss,
        size: metres(3.0, F34, F34),
        connectors: truss_ends!(1.5),
        chords: BOX_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "f34-corner",
        title: "F34 corner",
        shape: StockShape::TrussCorner,
        kind: SceneObjectKind::Truss,
        size: metres(F34, F34, F34),
        connectors: CORNER_ENDS,
        // Nothing hangs off a corner: it is 290 mm of block, and a clamp on it would
        // be a light bolted to a joint.
        chords: NO_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "f34-base",
        title: "F34 base plate",
        shape: StockShape::BasePlate,
        kind: SceneObjectKind::Support,
        size: metres(0.6, PLATE_HEIGHT, 0.6),
        connectors: &[Connector::new(
            metres(0.0, PLATE_HEIGHT, 0.0),
            metres(0.0, 1.0, 0.0),
            ConnectorKind::TrussEnd,
        )],
        chords: NO_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "f34-top",
        title: "F34 top plate",
        shape: StockShape::TopPlate,
        kind: SceneObjectKind::Support,
        size: metres(0.6, PLATE_HEIGHT, 0.6),
        connectors: &[Connector::new(
            metres(0.0, -PLATE_HEIGHT, 0.0),
            metres(0.0, -1.0, 0.0),
            ConnectorKind::TrussEnd,
        )],
        chords: NO_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "pipe-1m",
        title: "Pipe 1 m",
        shape: StockShape::Pipe,
        kind: SceneObjectKind::Truss,
        size: metres(1.0, PIPE_DIAMETER, PIPE_DIAMETER),
        connectors: pipe_ends!(0.5),
        chords: PIPE_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "pipe-2m",
        title: "Pipe 2 m",
        shape: StockShape::Pipe,
        kind: SceneObjectKind::Truss,
        size: metres(2.0, PIPE_DIAMETER, PIPE_DIAMETER),
        connectors: pipe_ends!(1.0),
        chords: PIPE_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "pipe-3m",
        title: "Pipe 3 m",
        shape: StockShape::Pipe,
        kind: SceneObjectKind::Truss,
        size: metres(3.0, PIPE_DIAMETER, PIPE_DIAMETER),
        connectors: pipe_ends!(1.5),
        chords: PIPE_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "pipe-6m",
        title: "Pipe 6 m",
        shape: StockShape::Pipe,
        kind: SceneObjectKind::Truss,
        size: metres(6.0, PIPE_DIAMETER, PIPE_DIAMETER),
        connectors: pipe_ends!(3.0),
        chords: PIPE_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "deck-2x1",
        title: "Stage deck 2 × 1 m",
        shape: StockShape::Deck,
        kind: SceneObjectKind::Object,
        // 200 mm, a deck's own thickness. How high it stands is its legs, which is
        // a property — a deck is not a taller deck for being on a riser.
        size: metres(2.0, 0.2, 1.0),
        connectors: deck_edges!(1.0, 0.5),
        chords: NO_CHORDS,
        properties: LEG_HEIGHT,
    },
    StockPiece {
        id: "deck-1x1",
        title: "Stage deck 1 × 1 m",
        shape: StockShape::Deck,
        kind: SceneObjectKind::Object,
        size: metres(1.0, 0.2, 1.0),
        connectors: deck_edges!(0.5, 0.5),
        chords: NO_CHORDS,
        properties: LEG_HEIGHT,
    },
    StockPiece {
        id: "wall-2x1",
        title: "Wall panel 2 × 1 m",
        shape: StockShape::Panel,
        kind: SceneObjectKind::Object,
        size: metres(2.0, 1.0, 0.05),
        connectors: NO_CONNECTORS,
        chords: NO_CHORDS,
        properties: NOTHING,
    },
    StockPiece {
        id: "flat-1x24",
        title: "Flat 1 × 2.4 m",
        shape: StockShape::Panel,
        kind: SceneObjectKind::Object,
        // The standard flat: a metre wide and eight foot tall in metres.
        size: metres(1.0, 2.4, 0.05),
        connectors: NO_CONNECTORS,
        chords: NO_CHORDS,
        properties: NOTHING,
    },
];

/// One piece by the id a `SceneObject` carries, or `None` for a name this build has
/// never heard of — which is drawn as nothing rather than refused, the rule a layout
/// already follows for a panel id it does not know.
pub fn piece(id: &str) -> Option<&'static StockPiece> {
    CATALOGUE.iter().find(|piece| piece.id == id)
}

/// One spelling of what a piece was asked for.
///
/// The declared keys and no others, each either what the object said or the piece's
/// own default, in the order the piece declares them. Three things read this and they
/// must agree to the byte: the geometry cache key, the HTTP ETag, and the name a
/// symdef is exported under. A map with a stray key in it, or the same map with its
/// keys in the other order, would be a second mesh for one piece — and on the way
/// back in through MVR, a second symbol.
///
/// A number that arrives outside the property's own range is brought inside it rather
/// than refused: a showfile from a later version of this console may know about legs
/// this one does not offer, and a deck drawn at the nearest height it can manage is
/// better than a deck that is not drawn.
pub fn canonical_properties(piece: &StockPiece, given: &Value) -> Value {
    let said = given.as_object();
    let mut out = Map::new();
    for property in piece.properties {
        let value = said.and_then(|map| map.get(property.key));
        out.insert(property.key.to_string(), canonical_one(property, value));
    }
    Value::Object(out)
}

/// The same, for an id rather than a piece. An unknown id has nothing to declare, so
/// the answer is the empty map.
pub fn canonical_properties_of(id: &str, given: &Value) -> Value {
    match piece(id) {
        Some(found) => canonical_properties(found, given),
        None => Value::Object(Map::new()),
    }
}

fn canonical_one(property: &Property, given: Option<&Value>) -> Value {
    match property.kind {
        PropertyKind::Number { min, max, step, .. } => {
            let raw = given.and_then(Value::as_f64).map(|n| n as f32).unwrap_or(property.default);
            let clamped = raw.clamp(min, max);
            // Snapped to the step, so 0.2000001 and 0.2 are one mesh rather than two.
            let stepped =
                if step > 0.0 { (clamped / step).round() * step } else { clamped };
            // Through six decimals, because the number goes into a cache key as text
            // and a float's own last bits are not a fact about the deck.
            Value::from((stepped as f64 * 1e6).round() / 1e6)
        }
        PropertyKind::Choice { options } => {
            let chosen = given
                .and_then(Value::as_str)
                .filter(|said| options.contains(said))
                .unwrap_or_else(|| options.first().copied().unwrap_or(""));
            Value::from(chosen)
        }
        PropertyKind::Bool => Value::from(
            given.and_then(Value::as_bool).unwrap_or(property.default != 0.0),
        ),
    }
}

/// One property of a piece, read back as a number.
///
/// What the geometry asks. It goes through [`canonical_properties`] first, so a piece
/// that was never given a value still gets its default and the answer is never
/// missing.
pub fn number(piece: &StockPiece, properties: &Value, key: &str) -> f32 {
    canonical_properties(piece, properties)
        .get(key)
        .and_then(Value::as_f64)
        .map(|n| n as f32)
        .unwrap_or(0.0)
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

    /// A connector's point is on the piece and its facing is a unit vector pointing
    /// away from it — the two things the mating arithmetic assumes and neither of
    /// which the type can enforce.
    #[test]
    fn every_connector_points_out_of_its_own_piece() {
        for piece in CATALOGUE {
            for connector in piece.connectors {
                let f = connector.facing;
                let length = (f.x * f.x + f.y * f.y + f.z * f.z).sqrt();
                assert!(
                    (length - 1.0).abs() < 1e-5,
                    "{}: a facing of {f:?} is not a direction",
                    piece.id,
                );
                // Pointing outwards: the dot of the facing with the point it is at is
                // never negative, or the joint would be on the far side of the piece
                // from the way it faces.
                let at = connector.at;
                let dot = at.x * f.x + at.y * f.y + at.z * f.z;
                assert!(dot >= -1e-6, "{}: a connector at {at:?} faces {f:?}", piece.id);
            }
        }
    }

    /// A chord is a line along X, so its own `at.x` says nothing and must be zero —
    /// `Mount::along` is measured from the piece's origin, and a chord that carried an
    /// offset of its own would move every light on it.
    #[test]
    fn every_chord_is_a_line_through_x_nought() {
        for piece in CATALOGUE {
            for chord in piece.chords {
                assert_eq!(chord.at.x, 0.0, "{}: a chord starts somewhere", piece.id);
            }
        }
    }

    /// The canonical form fills in what was not said, drops what was not asked for,
    /// and is the same map whichever way round the given one was written.
    #[test]
    fn canonical_properties_are_one_spelling() {
        let deck = piece("deck-2x1").expect("a deck");
        let empty = canonical_properties(deck, &Value::Null);
        assert_eq!(empty, serde_json::json!({ "leg_height": 0.2 }));

        let noisy = canonical_properties(
            deck,
            &serde_json::json!({ "leg_height": 0.6, "colour": "blue", "extra": 1 }),
        );
        assert_eq!(noisy, serde_json::json!({ "leg_height": 0.6 }));

        // Out of range, and off the step.
        let wild = canonical_properties(deck, &serde_json::json!({ "leg_height": 9.0 }));
        assert_eq!(wild, serde_json::json!({ "leg_height": 1.2 }));
        let off = canonical_properties(deck, &serde_json::json!({ "leg_height": 0.31 }));
        assert_eq!(off, serde_json::json!({ "leg_height": 0.4 }));

        // A piece with nothing to ask says nothing, whatever it is handed.
        let truss = piece("f34-2m").expect("a truss");
        assert_eq!(
            canonical_properties(truss, &serde_json::json!({ "leg_height": 1.0 })),
            serde_json::json!({}),
        );
    }
}
