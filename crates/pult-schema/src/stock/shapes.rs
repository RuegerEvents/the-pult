//! What each catalogue shape is made of.
//!
//! The arithmetic that used to be in `frontend/src/lib/stock.ts`, moved here so that
//! there is one of it: the bytes an operator opens in Vectorworks are the bytes the
//! rig view drew. Turning "two metres of F34" into chords and bracing is still a fact
//! about drawing rather than about the show — it is just that the drawing now has to
//! be the same on both sides of the wire.
//!
//! Every piece is centred on its own origin along X, because a fixture hangs at an
//! offset from the middle of the bar it is on and a piece whose origin was at one end
//! would make every one of those offsets depend on how long the section happened to
//! be. The exceptions are the two that stand on something: a deck's origin is its
//! **top surface**, which is the number anybody cares about, and a panel's is its
//! bottom edge, because a wall is placed on the floor.

use serde_json::Value;

use super::glb::{Material, Mesh};
use crate::types::catalogue::{number, StockPiece, StockShape, F34, PIPE_DIAMETER, PLATE_STEEL};

/// A chord tube: 50 mm on F34, which is what the ladder is welded out of.
const CHORD: f32 = 0.05;
/// And the bracing between them, 20 mm.
const BRACE: f32 = 0.02;

/// How round a chord is drawn, and how round a brace is.
///
/// Eight and four. A brace is 20 mm across and is a line on the screen at any camera
/// distance somebody works at; making it as round as a chord would double the
/// triangles in a truss to no visible end, and a festival rig is a hundred of them.
const CHORD_SEGMENTS: usize = 8;
const BRACE_SEGMENTS: usize = 4;

/// Aluminium, and not shiny: a truss under stage light is a matte grey thing, and a
/// mirror-finish one reads as a prop.
const ALUMINIUM: Material = Material { colour: [0.6, 0.63, 0.65], roughness: 0.65, metalness: 0.85 };
/// Decks and walls are painted, not extruded aluminium.
const PAINTED: Material = Material { colour: [0.18, 0.19, 0.21], roughness: 0.9, metalness: 0.05 };

/// One piece, as triangles.
///
/// `properties` must already be canonical — [`super::stock_glb`] is the only caller
/// and it does that first, which is what keeps one deck from having two meshes.
pub fn build(piece: &StockPiece, properties: &Value) -> Mesh {
    let mut mesh = Mesh {
        material: match piece.shape {
            StockShape::Deck | StockShape::Panel => PAINTED,
            _ => ALUMINIUM,
        },
        ..Mesh::default()
    };

    match piece.shape {
        StockShape::BoxTruss => box_truss(&mut mesh, piece.size.x, piece.size.y),
        StockShape::TrussCorner => truss_corner(&mut mesh, piece.size.x),
        StockShape::Pipe => {
            mesh.add_tube(
                [-piece.size.x / 2.0, 0.0, 0.0],
                [piece.size.x / 2.0, 0.0, 0.0],
                PIPE_DIAMETER / 2.0,
                CHORD_SEGMENTS,
            );
        }
        StockShape::Deck => deck(
            &mut mesh,
            piece.size.x,
            piece.size.y,
            piece.size.z,
            number(piece, properties, "leg_height"),
        ),
        StockShape::Panel => {
            let (w, h, d) = (piece.size.x, piece.size.y, piece.size.z);
            mesh.add_box([-w / 2.0, 0.0, -d / 2.0], [w / 2.0, h, d / 2.0]);
        }
        StockShape::BasePlate => plate(&mut mesh, piece.size.x, piece.size.y, piece.size.z, true),
        StockShape::TopPlate => plate(&mut mesh, piece.size.x, piece.size.y, piece.size.z, false),
    }
    mesh
}

/// A straight length of box truss, centred on its own origin and running along X.
fn box_truss(mesh: &mut Mesh, length: f32, square: f32) {
    let half = square / 2.0 - CHORD / 2.0;

    // The four chords, along X.
    for y in [half, -half] {
        for z in [half, -half] {
            mesh.add_tube(
                [-length / 2.0, y, z],
                [length / 2.0, y, z],
                CHORD / 2.0,
                CHORD_SEGMENTS,
            );
        }
    }

    // Bracing, as a zig-zag down each of the four faces. One bay every ~250 mm, which
    // is close enough to the real spacing to read correctly and coarse enough that a
    // three-metre section is a few dozen tubes rather than a few hundred.
    //
    // Held back half a brace from each end, because a brace runs at an angle and its
    // end cap is square to *its own* axis: a zig-zag starting exactly at the end of
    // the truss puts 7 mm of tube past it, and then a "one metre" section measures
    // 1.014 m. Which is not cosmetic — the length is what a rig is set out from.
    let inset = BRACE / 2.0;
    let braced = length - BRACE;
    let bays = ((braced / 0.25).round() as usize).max(2);
    let step = braced / bays as f32;
    for bay in 0..bays {
        let x0 = -length / 2.0 + inset + bay as f32 * step;
        let x1 = x0 + step;
        let up = bay % 2 == 0;
        let (from, to) = if up { (half, -half) } else { (-half, half) };
        // The two vertical faces, then the two horizontal ones.
        for z in [half, -half] {
            mesh.add_tube([x0, from, z], [x1, to, z], BRACE / 2.0, BRACE_SEGMENTS);
        }
        for y in [half, -half] {
            mesh.add_tube([x0, y, from], [x1, y, to], BRACE / 2.0, BRACE_SEGMENTS);
        }
    }
}

/// The block that turns one run of truss into another: a cube of chords.
///
/// Twelve bars, one along each edge of the cube, so a corner reads as a corner from
/// any of the six directions a run can come into it from.
fn truss_corner(mesh: &mut Mesh, square: f32) {
    let half = square / 2.0 - CHORD / 2.0;
    let end = square / 2.0;
    for a in [half, -half] {
        for b in [half, -half] {
            mesh.add_tube([-end, a, b], [end, a, b], CHORD / 2.0, CHORD_SEGMENTS);
            mesh.add_tube([a, -end, b], [a, end, b], CHORD / 2.0, CHORD_SEGMENTS);
            mesh.add_tube([a, b, -end], [a, b, end], CHORD / 2.0, CHORD_SEGMENTS);
        }
    }
}

/// A rostrum: a top slab with legs under it.
///
/// The origin is the **top surface**, so a deck is placed at the height its top is at
/// — which is the number anybody cares about and the only one anybody measures. The
/// slab hangs below that, and `leg_height` is how far the whole thing reaches down
/// from the top: at the default it is exactly the slab and the deck sits on the floor,
/// and anything more is legs.
fn deck(mesh: &mut Mesh, length: f32, thickness: f32, depth: f32, leg_height: f32) {
    let (hx, hz) = (length / 2.0, depth / 2.0);
    mesh.add_box([-hx, -thickness, -hz], [hx, 0.0, hz]);

    let floor = -leg_height.max(thickness);
    if floor >= -thickness {
        return;
    }
    let leg = 0.06;
    for x in [hx - leg, -(hx - leg)] {
        for z in [hz - leg, -(hz - leg)] {
            mesh.add_box(
                [x - leg / 2.0, floor, z - leg / 2.0],
                [x + leg / 2.0, -thickness, z + leg / 2.0],
            );
        }
    }
}

/// What a tower stands on, or what caps it.
///
/// A plate and a short stub of truss end, so that the piece reads as a fitting rather
/// than as a sheet of steel. `upward` puts the plate's face at the origin and the stub
/// above it; the top plate is the same thing the other way up. Which face the truss
/// bolts to is the one connector the piece declares, so nothing has to guess.
fn plate(mesh: &mut Mesh, width: f32, height: f32, depth: f32, upward: bool) {
    let (hx, hz) = (width / 2.0, depth / 2.0);
    let sign = if upward { 1.0 } else { -1.0 };
    let steel = PLATE_STEEL * sign;
    let (low, high) = if upward { (0.0, steel) } else { (steel, 0.0) };
    mesh.add_box([-hx, low, -hz], [hx, high, hz]);

    // Four short stubs where the truss's own chords land, which is what makes a plate
    // look like a thing a tower stands on rather than a sheet of steel.
    let square = F34 / 2.0 - CHORD / 2.0;
    let stub = (height - PLATE_STEEL) * sign;
    for x in [square, -square] {
        for z in [square, -square] {
            mesh.add_tube([x, steel, z], [x, steel + stub, z], CHORD / 2.0, CHORD_SEGMENTS);
        }
    }
}
