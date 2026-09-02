//! The rig as a drawing: trusses, objects, layers, and where everything is.
//!
//! Before this, the only thing in the show that had a place was a fixture, and its
//! place was a point and perhaps a direction. That is enough to draw a beam and not
//! enough to draw a rig: a truss is somewhere, it is turned to face somewhere, things
//! hang off it, and moving it should move them.
//!
//! # Why a transform has a signed scale
//!
//! A drawing mirrors things. Twenty-one of the forty-three trusses in the first real
//! MVR file this console was pointed at have a basis whose determinant is −1, and no
//! rotation is a reflection — decomposed into position, rotation and an unsigned
//! scale, a mirrored truss comes back as some rotation that puts it nearly right,
//! with its bolt holes on the wrong side and nothing in the numbers saying so. So
//! [`Transform::scale`] may be negative, and the browser draws such an object with a
//! two-sided material because negative scale flips its normals.
//!
//! # Why this is worked out twice
//!
//! [`world_transform`] exists here and again in `frontend/src/lib/scene.ts`, for the
//! reason `SelectionQuery` is evaluated twice: dragging a truss re-composes every
//! child's place per frame and cannot be a round trip. The two are held together by
//! `testdata/transforms.json`, which both test suites read. A new rule about
//! composing transforms needs a case there or it is only half implemented.
//!
//! The arithmetic is small and deliberately not shared with `pult-mvr`'s: that crate
//! converts between MVR's millimetre Z-up space and this one, which is a different
//! job from composing two placements in this one. What holds them together is a case
//! in the same corpus that starts from a matrix as a file writes it.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::fixture::Vec3;
use crate::PultSchema;

/// Where something is, what it is turned to, and how big it is.
///
/// Metres, and XYZ Euler degrees — three.js's default order, and so the console's,
/// stated here and assumed everywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Transform {
    pub position: Vec3,
    /// XYZ Euler degrees.
    pub rotation: Vec3,
    /// One per axis, and **signed**: a negative component is a reflection, which is
    /// the only way to say that a drawing mirrored this object.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            position: Vec3::default(),
            rotation: Vec3::default(),
            scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        }
    }
}

impl Transform {
    /// At a point, turned to nothing, at its own size.
    pub fn at(position: Vec3) -> Self {
        Transform { position, ..Transform::default() }
    }

    /// At a point, aimed along a direction.
    ///
    /// Yaw and pitch and no roll, which is what aiming a light at something means:
    /// there is nothing in "point it over there" that says which way up it is.
    pub fn facing(position: Vec3, direction: Vec3) -> Self {
        let length = (direction.x.powi(2) + direction.y.powi(2) + direction.z.powi(2)).sqrt();
        if length < 1e-6 {
            return Transform::at(position);
        }
        let d = Vec3 {
            x: direction.x / length,
            y: direction.y / length,
            z: direction.z / length,
        };
        // A fixture's own axis is −Y, straight down. The rotation that takes −Y to `d`
        // tips away from straight down and *then* turns, and that order is not two of
        // the three XYZ angles: XYZ applies the tip first, and a light aimed sideways
        // loses its turn entirely. So the basis is built and decomposed instead.
        let pitch = (-d.y).clamp(-1.0, 1.0).acos();
        // Straight down and straight up have no bearing to speak of, and asking for
        // one gives `atan2(-0, -0)`, which is −π: a hanging light stored as turned all
        // the way round, and the epsilons that come back out of the angles then read
        // as a bearing of 45°.
        let yaw = if d.x.hypot(d.z) < 1e-9 { 0.0 } else { (-d.x).atan2(-d.z) };
        let (sp, cp) = pitch.sin_cos();
        let (sy, cy) = yaw.sin_cos();
        let basis = [
            [cy, sy * sp, sy * cp],
            [0.0, cp, -sp],
            [-sy, cy * sp, cy * cp],
        ];
        Transform {
            position,
            rotation: basis_to_euler_xyz_degrees(basis),
            ..Transform::default()
        }
    }

    /// The direction this transform points a fixture's own down axis.
    ///
    /// Negative zero is turned back into zero, and that is not tidiness: a fixture
    /// hanging straight down comes out as `(-0, -1, -0)`, and `atan2(0, -0)` is π —
    /// so a bearing taken off it is 180° out, and every beam in the rig points
    /// upstage. The browser's half does the same, for the same reason.
    pub fn facing_direction(&self) -> Vec3 {
        let basis = euler_xyz_degrees_to_basis(self.rotation);
        let zeroed = |n: f32| if n == 0.0 { 0.0 } else { n };
        // −Y through the rotation.
        Vec3 {
            x: zeroed(-basis[0][1]),
            y: zeroed(-basis[1][1]),
            z: zeroed(-basis[2][1]),
        }
    }
}

// ── The entities ──────────────────────────────────────────────────────────────

/// What an object in the rig is. MVR's own list, which is also the list an operator
/// would give: something to hang things off, something to hang, something to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SceneObjectKind {
    /// Anything that is not one of the others: a rostrum, a wall, a piece of set.
    #[default]
    Object,
    Truss,
    Support,
    VideoScreen,
    Projector,
    /// A point lights are aimed at.
    FocusPoint,
    /// Not a thing but a handle on several: what moving a whole truss and its lights
    /// at once takes hold of.
    Group,
}

/// One mesh, and where it sits relative to the object that carries it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GeometryRef {
    /// The sha256 the mesh is stored under in the asset store.
    pub asset: String,
    /// What the file that carried it called it. Kept so an export writes the archive
    /// back with names its own meshes still resolve — a `.3ds` names its texture by a
    /// bare file name, and a content-addressed store has no names at all.
    pub file_name: String,
    pub transform: Transform,
}

/// A truss, a rostrum, a screen: something in the rig that is not a light.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "scene_objects")]
pub struct SceneObject {
    /// The MVR uuid where it came from one, so a re-import matches rather than
    /// duplicates.
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub kind: SceneObjectKind,
    /// Where it is **relative to its parent**. Composing the chain is
    /// [`world_transform`].
    #[pult(lifecycle = PERSISTED)]
    pub transform: Transform,
    /// The object this hangs off, if any.
    #[pult(lifecycle = PERSISTED)]
    pub parent: Option<Uuid>,
    #[pult(lifecycle = PERSISTED)]
    pub layer: Option<Uuid>,
    #[pult(lifecycle = PERSISTED)]
    pub class: Option<Uuid>,
    /// Meshes of its own.
    #[pult(lifecycle = PERSISTED)]
    pub geometry: Vec<GeometryRef>,
    /// Or a shared one, which is how a rig of forty identical truss sections holds
    /// one mesh rather than forty.
    #[pult(lifecycle = PERSISTED)]
    pub symbol: Option<Uuid>,
}

/// A drawing's layer: a name to show, hide and lock a part of the rig by.
///
/// Whether *this browser* is showing it is a Svelte store, not a field — two people
/// looking at one show should be able to look at different parts of it. Whether it is
/// locked is the show's, because that is a decision about the rig rather than about a
/// screen.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "layers")]
pub struct Layer {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub locked: bool,
    /// Where it sits in the layers panel.
    ///
    /// Not `order`, which is a SQL keyword and which the generated `CREATE TABLE`
    /// does not quote — a column called that fails to open the show.
    #[pult(lifecycle = PERSISTED)]
    pub sort_order: u32,
}

/// A piece of geometry drawn once and placed many times.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "symbols")]
pub struct Symbol {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub geometry: Vec<GeometryRef>,
}

/// A tag that cuts across layers: "house rig", "touring", "practicals".
///
/// MVR's `Class`, kept rather than dropped for two reasons. It is data the file
/// carries that nothing else in the show could reconstruct, and an export that lost it
/// would not round-trip — every real file in the corpus classes every object it has.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "classes")]
pub struct SceneClass {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
}

/// The name a file gave an asset, against the sha it is stored under.
///
/// The asset store is content-addressed and has no names in it, and a mesh does have
/// names in it: a `.3ds` asks for `tx603.jpg` by that string and nothing else. This is
/// the bridge, in both directions — the browser resolves a texture name through it,
/// and an export writes each asset back under a name its own meshes still find.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "named_assets")]
pub struct NamedAsset {
    /// UUIDv5 of the name, so two imports of one archive write one row.
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    /// The name as the file spelled it.
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// The sha256 it is stored under.
    #[pult(lifecycle = PERSISTED)]
    pub asset: String,
    /// What kind of file it is, as it was stored.
    #[pult(lifecycle = PERSISTED)]
    pub mime: String,
}

/// The namespace names are hashed in, so an id follows from a name.
pub const NAMED_ASSET_NAMESPACE: Uuid = Uuid::from_u128(0x7075_6c74_6e61_6d65_6461_7373_6574_7331);

impl NamedAsset {
    /// The id a given name always has.
    pub fn id_for(name: &str) -> Uuid {
        Uuid::new_v5(&NAMED_ASSET_NAMESPACE, name.as_bytes())
    }
}

// ── Composing a chain ─────────────────────────────────────────────────────────

/// How deep a parent chain may go before this stops walking it.
///
/// A cycle is not supposed to be possible — an import builds the chain from a tree —
/// but a peer, a plugin or an undo could write one, and a rig view that hangs is
/// worse than one that draws a truss in the wrong place.
pub const MAX_DEPTH: usize = 64;

/// Where something actually is, with every parent's placement applied.
///
/// Objects are keyed by id; anything naming a parent that is not there is treated as
/// having none, because a truss somebody deleted should leave its lights where they
/// were rather than move them to the origin.
pub fn world_transform(
    local: &Transform,
    parent: Option<Uuid>,
    objects: &HashMap<Uuid, &SceneObject>,
) -> Transform {
    let mut matrix = to_matrix(local);
    let mut next = parent;
    let mut seen = 0;
    while let Some(id) = next {
        seen += 1;
        if seen > MAX_DEPTH {
            break;
        }
        let Some(object) = objects.get(&id) else { break };
        matrix = multiply(to_matrix(&object.transform), matrix);
        next = object.parent;
    }
    from_matrix(matrix)
}

/// The same, for a whole collection, so the caller builds the map once.
pub fn by_id(objects: &[SceneObject]) -> HashMap<Uuid, &SceneObject> {
    objects.iter().map(|object| (object.id, object)).collect()
}

// ── The arithmetic ────────────────────────────────────────────────────────────

/// A transform as a 4x4, column-vector convention: `world = M · local`.
type Matrix4 = [[f32; 4]; 4];

fn to_matrix(transform: &Transform) -> Matrix4 {
    let basis = euler_xyz_degrees_to_basis(transform.rotation);
    let scale = [transform.scale.x, transform.scale.y, transform.scale.z];
    let mut out = [[0.0f32; 4]; 4];
    for (row, cells) in basis.iter().enumerate() {
        for (col, cell) in cells.iter().enumerate() {
            out[row][col] = cell * scale[col];
        }
    }
    out[0][3] = transform.position.x;
    out[1][3] = transform.position.y;
    out[2][3] = transform.position.z;
    out[3][3] = 1.0;
    out
}

fn from_matrix(matrix: Matrix4) -> Transform {
    let mut basis = [[0.0f32; 3]; 3];
    for (row, cells) in basis.iter_mut().enumerate() {
        for (col, cell) in cells.iter_mut().enumerate() {
            *cell = matrix[row][col];
        }
    }

    let mut scale = [0.0f32; 3];
    for (axis, length) in scale.iter_mut().enumerate() {
        *length = (0..3).map(|row| basis[row][axis].powi(2)).sum::<f32>().sqrt();
        if *length < 1e-6 {
            *length = 1.0;
        }
    }
    if determinant(basis) < 0.0 {
        scale[0] = -scale[0];
    }
    for cells in basis.iter_mut() {
        for (axis, cell) in cells.iter_mut().enumerate() {
            *cell /= scale[axis];
        }
    }

    Transform {
        position: Vec3 { x: matrix[0][3], y: matrix[1][3], z: matrix[2][3] },
        rotation: basis_to_euler_xyz_degrees(basis),
        scale: Vec3 { x: scale[0], y: scale[1], z: scale[2] },
    }
}

fn multiply(a: Matrix4, b: Matrix4) -> Matrix4 {
    let mut out = [[0.0f32; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn determinant(m: [[f32; 3]; 3]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Three degrees to a rotation matrix, XYZ order: `R = Rx · Ry · Rz`.
pub fn euler_xyz_degrees_to_basis(euler: Vec3) -> [[f32; 3]; 3] {
    let (sx, cx) = euler.x.to_radians().sin_cos();
    let (sy, cy) = euler.y.to_radians().sin_cos();
    let (sz, cz) = euler.z.to_radians().sin_cos();
    [
        [cy * cz, -cy * sz, sy],
        [cx * sz + sx * sy * cz, cx * cz - sx * sy * sz, -sx * cy],
        [sx * sz - cx * sy * cz, sx * cz + cx * sy * sz, cx * cy],
    ]
}

/// And back.
pub fn basis_to_euler_xyz_degrees(basis: [[f32; 3]; 3]) -> Vec3 {
    let sy = basis[0][2].clamp(-1.0, 1.0);
    let y = sy.asin();
    let (x, z) = if sy.abs() < 0.999_999 {
        ((-basis[1][2]).atan2(basis[2][2]), (-basis[0][1]).atan2(basis[0][0]))
    } else {
        // Gimbal lock: roll and yaw turn about the same axis, so it all goes in x.
        (basis[2][1].atan2(basis[1][1]), 0.0)
    };
    Vec3 { x: x.to_degrees(), y: y.to_degrees(), z: z.to_degrees() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: Vec3, want: Vec3, what: &str) {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(
            near(got.x, want.x) && near(got.y, want.y) && near(got.z, want.z),
            "{what}: {got:?} against {want:?}",
        );
    }

    #[test]
    fn a_fixture_nothing_has_turned_hangs_straight_down() {
        close(Transform::default().facing_direction(), Vec3 { x: 0.0, y: -1.0, z: 0.0 }, "at rest");
    }

    /// The pair that matters: aiming somewhere and then asking where it is aimed.
    ///
    /// A sideways aim is the case that catches the wrong Euler order — pitch and yaw
    /// as two of the three XYZ angles applies the pitch first, and the yaw then turns
    /// about an axis the pitch has already moved.
    #[test]
    fn aiming_somewhere_and_asking_where_it_is_aimed_agree() {
        for direction in [
            Vec3 { x: 0.0, y: -1.0, z: 0.0 },
            Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            Vec3 { x: 0.0, y: 0.0, z: -1.0 },
            Vec3 { x: 0.5, y: -0.5, z: 0.7 },
            Vec3 { x: -2.0, y: -3.0, z: 1.0 },
        ] {
            let length = (direction.x.powi(2) + direction.y.powi(2) + direction.z.powi(2)).sqrt();
            let unit = Vec3 {
                x: direction.x / length,
                y: direction.y / length,
                z: direction.z / length,
            };

            let aimed = Transform::facing(Vec3::default(), direction);

            close(aimed.facing_direction(), unit, &format!("aimed at {direction:?}"));
        }
    }

    /// A placement that has been through a matrix and back is the placement it was.
    #[test]
    fn a_transform_survives_becoming_a_matrix() {
        let original = Transform {
            position: Vec3 { x: 1.0, y: -2.0, z: 3.5 },
            rotation: Vec3 { x: 20.0, y: -35.0, z: 10.0 },
            scale: Vec3 { x: -1.0, y: 2.0, z: 0.5 },
        };

        let back = from_matrix(to_matrix(&original));

        close(back.position, original.position, "position");
        close(back.rotation, original.rotation, "rotation");
        close(back.scale, original.scale, "scale");
    }
}
