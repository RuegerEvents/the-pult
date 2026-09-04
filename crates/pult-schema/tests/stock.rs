//! The catalogue's own geometry: is it the size it says it is, and does it bolt
//! together?
//!
//! Both are gates rather than curiosities. The size is what a rig is measured in — a
//! truss whose mesh is not the length the table says is a rig where every light is
//! somewhere else — and the mating is what the whole editor rests on: dragging one
//! section against another has to put them end to end, and it does that by asking two
//! connectors to meet.
//!
//! The reader below is deliberately tiny. A glTF crate would be a dependency this
//! crate has no other use for, and what is being checked is exactly the two numbers a
//! reader has to get out of the file anyway.

use pult_schema::stock::stock_glb;
use pult_schema::types::catalogue::{
    canonical_properties, number, Connector, ConnectorKind, StockPiece, StockShape, CATALOGUE,
};
use pult_schema::types::fixture::Vec3;
use pult_schema::types::scene::{euler_xyz_degrees_to_basis, Transform};
use serde_json::{json, Value};

/// What the POSITION accessor says the mesh spans.
///
/// Read out of the JSON chunk rather than by walking the buffer: the accessor's `min`
/// and `max` are what every loader frames on, so if they are wrong the fact that the
/// vertices are right helps nobody.
fn bounds_of(glb: &[u8]) -> ([f32; 3], [f32; 3]) {
    assert_eq!(&glb[0..4], b"glTF", "not a glb");
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    assert_eq!(&glb[16..20], b"JSON");
    let document: Value = serde_json::from_slice(&glb[20..20 + json_len]).expect("the JSON chunk");

    // The primitive's POSITION attribute names the accessor to read.
    let which = document["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
        .as_u64()
        .expect("a POSITION attribute") as usize;
    let accessor = &document["accessors"][which];
    let axis = |key: &str| {
        let list = accessor[key].as_array().expect("bounds on the POSITION accessor");
        [
            list[0].as_f64().unwrap() as f32,
            list[1].as_f64().unwrap() as f32,
            list[2].as_f64().unwrap() as f32,
        ]
    };
    (axis("min"), axis("max"))
}

/// How tall a piece's mesh should be, which is its declared size except for a deck,
/// whose legs are a property.
fn wanted_height(piece: &StockPiece, properties: &Value) -> f32 {
    match piece.shape {
        StockShape::Deck => piece.size.y.max(number(piece, properties, "leg_height")),
        _ => piece.size.y,
    }
}

#[test]
fn every_piece_is_the_size_the_table_says_it_is() {
    for piece in CATALOGUE {
        let glb = stock_glb(piece.id, &Value::Null).expect("a listed piece draws");
        let (min, max) = bounds_of(&glb);
        let span = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        let wanted = [piece.size.x, wanted_height(piece, &Value::Null), piece.size.z];
        for (axis, (got, want)) in span.iter().zip(wanted.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-3,
                "{}: axis {axis} spans {got} and the table says {want}",
                piece.id,
            );
        }
    }
}

/// A deck's legs are the one property in the catalogue, and the whole of what it is
/// for is that a stage can be at two heights.
#[test]
fn a_decks_legs_reach_as_far_as_it_was_asked_for() {
    let properties = json!({ "leg_height": 0.8 });
    let glb = stock_glb("deck-2x1", &properties).expect("a deck draws");
    let (min, max) = bounds_of(&glb);

    // Its origin is the top surface, so the top is at nought and the legs go down.
    assert!(max[1].abs() < 1e-3, "the top of a deck is its origin, not {}", max[1]);
    assert!((min[1] + 0.8).abs() < 1e-3, "its legs reach {} rather than −0.8", min[1]);

    // And a deck with no legs to speak of is the slab it always was.
    let flat = stock_glb("deck-2x1", &Value::Null).expect("a deck draws");
    let (low, _) = bounds_of(&flat);
    assert!((low[1] + 0.2).abs() < 1e-3, "a deck on the floor reaches {}", low[1]);
}

/// A deck at two leg heights is two meshes, which is the whole reason the properties
/// are in the cache key.
#[test]
fn two_leg_heights_are_two_meshes() {
    let low = stock_glb("deck-2x1", &json!({ "leg_height": 0.2 })).unwrap();
    let high = stock_glb("deck-2x1", &json!({ "leg_height": 0.8 })).unwrap();
    assert_ne!(low, high);
    // And two spellings of one answer are one mesh.
    assert_eq!(low, stock_glb("deck-2x1", &json!({ "leg_height": 0.21, "hue": 4 })).unwrap());
}

// ── The joints ────────────────────────────────────────────────────────────────

/// The placement that brings `moving`'s connector onto `fixed`'s, mated.
///
/// The rule the whole editor's snapping is: the two points meet, and the facings end
/// up opposite. Written here as the plainest possible statement of it, so that what
/// the browser implements has something to be checked against.
fn mating(fixed_at: Vec3, fixed_facing: Vec3, moving: &Connector) -> Transform {
    // Turn the moving connector's facing onto the opposite of the fixed one.
    let wanted = Vec3 { x: -fixed_facing.x, y: -fixed_facing.y, z: -fixed_facing.z };
    let turn = rotation_taking(moving.facing, wanted);
    let basis = euler_xyz_degrees_to_basis(turn);
    let turned = Vec3 {
        x: basis[0][0] * moving.at.x + basis[0][1] * moving.at.y + basis[0][2] * moving.at.z,
        y: basis[1][0] * moving.at.x + basis[1][1] * moving.at.y + basis[1][2] * moving.at.z,
        z: basis[2][0] * moving.at.x + basis[2][1] * moving.at.y + basis[2][2] * moving.at.z,
    };
    Transform {
        position: Vec3 {
            x: fixed_at.x - turned.x,
            y: fixed_at.y - turned.y,
            z: fixed_at.z - turned.z,
        },
        rotation: turn,
        ..Transform::default()
    }
}

/// XYZ Euler degrees taking one unit vector to another, going the short way.
fn rotation_taking(from: Vec3, to: Vec3) -> Vec3 {
    let dot = (from.x * to.x + from.y * to.y + from.z * to.z).clamp(-1.0, 1.0);
    // Already there, or exactly opposite — in which case any perpendicular axis will
    // do, and this picks one the same way every time.
    let axis = if dot > 0.999_999 {
        return Vec3::default();
    } else if dot < -0.999_999 {
        let helper = if from.x.abs() < 0.9 {
            Vec3 { x: 1.0, y: 0.0, z: 0.0 }
        } else {
            Vec3 { x: 0.0, y: 1.0, z: 0.0 }
        };
        cross(from, helper)
    } else {
        cross(from, to)
    };
    let angle = dot.acos();
    axis_angle_to_euler(normalise(axis), angle)
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn normalise(v: Vec3) -> Vec3 {
    let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if length < 1e-9 {
        Vec3 { x: 1.0, y: 0.0, z: 0.0 }
    } else {
        Vec3 { x: v.x / length, y: v.y / length, z: v.z / length }
    }
}

fn axis_angle_to_euler(axis: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    let t = 1.0 - cos;
    let basis = [
        [
            t * axis.x * axis.x + cos,
            t * axis.x * axis.y - sin * axis.z,
            t * axis.x * axis.z + sin * axis.y,
        ],
        [
            t * axis.x * axis.y + sin * axis.z,
            t * axis.y * axis.y + cos,
            t * axis.y * axis.z - sin * axis.x,
        ],
        [
            t * axis.x * axis.z - sin * axis.y,
            t * axis.y * axis.z + sin * axis.x,
            t * axis.z * axis.z + cos,
        ],
    ];
    pult_schema::types::scene::basis_to_euler_xyz_degrees(basis)
}

/// Every connector of like kind mates with every other: the two points end up
/// together and the two facings end up opposite.
///
/// The rule is what stops a snap radius from being a guess, and stating it as a test
/// over the *whole* catalogue is what stops a new piece from being added with a
/// connector facing into its own middle.
#[test]
fn like_connectors_mate() {
    let all: Vec<(&StockPiece, &Connector)> = CATALOGUE
        .iter()
        .flat_map(|piece| piece.connectors.iter().map(move |c| (piece, c)))
        .collect();

    for (fixed_piece, fixed) in &all {
        for (moving_piece, moving) in &all {
            if fixed.kind != moving.kind {
                continue;
            }
            let placed = mating(fixed.at, fixed.facing, moving);
            let basis = euler_xyz_degrees_to_basis(placed.rotation);
            let through = |v: Vec3| Vec3 {
                x: basis[0][0] * v.x + basis[0][1] * v.y + basis[0][2] * v.z,
                y: basis[1][0] * v.x + basis[1][1] * v.y + basis[1][2] * v.z,
                z: basis[2][0] * v.x + basis[2][1] * v.y + basis[2][2] * v.z,
            };
            let at = through(moving.at);
            let landed = Vec3 {
                x: at.x + placed.position.x,
                y: at.y + placed.position.y,
                z: at.z + placed.position.z,
            };
            let facing = through(moving.facing);

            let what = format!("{} onto {}", moving_piece.id, fixed_piece.id);
            for (axis, (got, want)) in [
                (landed.x, fixed.at.x),
                (landed.y, fixed.at.y),
                (landed.z, fixed.at.z),
            ]
            .iter()
            .enumerate()
            {
                assert!((got - want).abs() < 1e-4, "{what}: axis {axis} landed at {got}, not {want}");
            }
            let dot = facing.x * fixed.facing.x + facing.y * fixed.facing.y + facing.z * fixed.facing.z;
            assert!((dot + 1.0).abs() < 1e-4, "{what}: the joints face the same way, not opposite");
        }
    }
}

/// Nothing bolts to a different kind of joint, which is what stops a deck from
/// snapping onto a truss end because somebody dragged it close.
#[test]
fn a_truss_end_is_not_a_deck_edge() {
    let truss = pult_schema::types::catalogue::piece("f34-2m").unwrap();
    let deck = pult_schema::types::catalogue::piece("deck-2x1").unwrap();
    assert!(truss.connectors.iter().all(|c| c.kind == ConnectorKind::TrussEnd));
    assert!(deck.connectors.iter().all(|c| c.kind == ConnectorKind::DeckEdge));
    let pipe = pult_schema::types::catalogue::piece("pipe-2m").unwrap();
    assert!(pipe.connectors.iter().all(|c| c.kind == ConnectorKind::PipeEnd));
}

/// A base plate is one truss end pointing up and a top plate is one pointing down,
/// which is what makes a spigot kind unnecessary — and what lets a tower stand.
#[test]
fn the_plates_are_a_truss_end_each_way_up() {
    let base = pult_schema::types::catalogue::piece("f34-base").unwrap();
    let top = pult_schema::types::catalogue::piece("f34-top").unwrap();
    assert_eq!(base.connectors.len(), 1);
    assert_eq!(top.connectors.len(), 1);
    assert!(base.connectors[0].facing.y > 0.9, "a base plate takes a truss upwards");
    assert!(top.connectors[0].facing.y < -0.9, "a top plate takes one downwards");
}

/// The canonical properties are what everything else keys off, so an object carrying
/// nothing and an object carrying the defaults are one piece.
#[test]
fn a_piece_asked_for_nothing_is_a_piece_asked_for_its_defaults() {
    for piece in CATALOGUE {
        let empty = canonical_properties(piece, &Value::Null);
        let spelled = canonical_properties(piece, &empty);
        assert_eq!(empty, spelled, "{}", piece.id);
        assert_eq!(empty.as_object().map(|m| m.len()), Some(piece.properties.len()));
    }
}
