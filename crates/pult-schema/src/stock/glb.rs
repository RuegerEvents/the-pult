//! A minimal binary glTF writer.
//!
//! Enough of glTF 2.0 to say "here are some triangles with normals and one material",
//! which is the whole of what a truss is. Not a library: a `.glb` this console writes
//! is read by three.js's `GLTFLoader` and by whatever somebody opens the exported MVR
//! in, and both of those want the common case done properly rather than the general
//! case done at all.
//!
//! # Deterministic by construction
//!
//! One buffer, three views in a fixed order, one accessor each, and the JSON built
//! from a `serde_json::Map` — whose keys are ordered — so the same mesh writes the
//! same bytes on every station. That is not tidiness: the ETag on `/stock/{id}.glb`
//! is a digest of these bytes, and the MVR export names its archive entry after the
//! piece's own hash and would otherwise write two files for one truss.
//!
//! Every position and normal is rounded to a micrometre before it is written, so an
//! `f32` whose last bits differ between two builds of the same arithmetic cannot
//! reach the buffer.

use serde_json::{json, Map, Value};

/// What a `.glb` is served and stored as.
pub const GLB_MIME: &str = "model/gltf-binary";

/// Triangles, with a normal per vertex.
///
/// Flat-shaded where the shape wants it and smooth round a tube, which is why the
/// normals are given rather than worked out: a truss chord is a cylinder and a deck
/// is a box, and one rule for both would make one of them wrong.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// The colour, roughness and metalness the piece is drawn in when nobody says
    /// otherwise. The browser puts its own shared materials on instead, so that a
    /// hundred truss sections cost one shader; this is for everybody else.
    pub material: Material,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub colour: [f32; 3],
    pub roughness: f32,
    pub metalness: f32,
}

impl Default for Material {
    fn default() -> Self {
        Material { colour: [0.6, 0.63, 0.65], roughness: 0.65, metalness: 0.85 }
    }
}

impl Mesh {
    /// Add a triangle fan's worth of geometry, offsetting the indices for what is
    /// already here.
    fn push(&mut self, positions: &[[f32; 3]], normals: &[[f32; 3]], indices: &[u32]) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(positions);
        self.normals.extend_from_slice(normals);
        self.indices.extend(indices.iter().map(|n| n + base));
    }

    /// A box between two corners, flat-shaded: six faces, four vertices each.
    ///
    /// Its own vertices per face rather than eight shared ones, because a box with
    /// shared corners has one normal per corner and shades like a ball.
    pub fn add_box(&mut self, min: [f32; 3], max: [f32; 3]) {
        // (axis, sign) for each of the six faces, in a fixed order.
        for axis in 0..3usize {
            for sign in [-1.0f32, 1.0] {
                let mut normal = [0.0f32; 3];
                normal[axis] = sign;
                // The two axes the face spans, in the order that keeps it wound
                // anticlockwise seen from outside.
                let (u, v) = if sign > 0.0 {
                    ((axis + 1) % 3, (axis + 2) % 3)
                } else {
                    ((axis + 2) % 3, (axis + 1) % 3)
                };
                let corner = |du: bool, dv: bool| {
                    let mut point = [0.0f32; 3];
                    point[axis] = if sign > 0.0 { max[axis] } else { min[axis] };
                    point[u] = if du { max[u] } else { min[u] };
                    point[v] = if dv { max[v] } else { min[v] };
                    point
                };
                self.push(
                    &[corner(false, false), corner(true, false), corner(true, true), corner(false, true)],
                    &[normal; 4],
                    &[0, 1, 2, 0, 2, 3],
                );
            }
        }
    }

    /// A tube between two points: a cylinder with flat ends.
    ///
    /// `segments` decides how round it is. Eight for a chord, which is what somebody
    /// looks at; four for bracing, which at 20 mm across is a line on the screen and
    /// whose only job is to be there.
    pub fn add_tube(&mut self, from: [f32; 3], to: [f32; 3], radius: f32, segments: usize) {
        let axis = sub(to, from);
        let length = norm(axis);
        if length < 1e-6 || segments < 3 {
            return;
        }
        let along = scale(axis, 1.0 / length);
        // Any two directions across the tube. Picked off whichever world axis the
        // tube is least aligned with, so the cross product is never near zero — and
        // picked *by index* rather than by a comparison on floats, so a tube exactly
        // along an axis always gets the same pair.
        let least = (0..3)
            .min_by(|a, b| along[*a].abs().total_cmp(&along[*b].abs()))
            .unwrap_or(0);
        let mut helper = [0.0f32; 3];
        helper[least] = 1.0;
        let across = normalise(cross(along, helper));
        let other = cross(along, across);

        let ring = |turn: usize| -> [f32; 3] {
            let angle = std::f32::consts::TAU * (turn as f32) / (segments as f32);
            let (sin, cos) = angle.sin_cos();
            add(scale(across, cos), scale(other, sin))
        };

        // The side, as one quad per segment with its own vertices so the seam is not
        // a shared-normal artefact.
        for turn in 0..segments {
            let a = ring(turn);
            let b = ring((turn + 1) % segments);
            let pa = scale(a, radius);
            let pb = scale(b, radius);
            self.push(
                &[add(from, pa), add(from, pb), add(to, pb), add(to, pa)],
                &[a, b, b, a],
                &[0, 1, 2, 0, 2, 3],
            );
        }

        // And the two ends, as fans.
        for (centre, normal, forward) in
            [(from, scale(along, -1.0), false), (to, along, true)]
        {
            let mut positions = Vec::with_capacity(segments);
            for turn in 0..segments {
                positions.push(add(centre, scale(ring(turn), radius)));
            }
            let mut indices = Vec::with_capacity((segments - 2) * 3);
            for triangle in 1..segments - 1 {
                if forward {
                    indices.extend([0, triangle as u32, triangle as u32 + 1]);
                } else {
                    indices.extend([0, triangle as u32 + 1, triangle as u32]);
                }
            }
            let normals = vec![normal; segments];
            self.push(&positions, &normals, &indices);
        }
    }

    /// The box every vertex is inside, which is what the tests measure and what the
    /// accessor has to declare.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for point in &self.positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        if self.positions.is_empty() {
            return ([0.0; 3], [0.0; 3]);
        }
        (min, max)
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f32; 3], by: f32) -> [f32; 3] {
    [a[0] * by, a[1] * by, a[2] * by]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn normalise(a: [f32; 3]) -> [f32; 3] {
    let length = norm(a);
    if length < 1e-9 {
        [1.0, 0.0, 0.0]
    } else {
        scale(a, 1.0 / length)
    }
}

/// To the nearest micrometre.
///
/// A truss is measured in millimetres and nobody is asking for more, and the rounding
/// is what keeps two builds of the same arithmetic writing the same bytes.
fn micrometre(value: f32) -> f32 {
    (value as f64 * 1e6).round() as f32 / 1e6
}

/// Write a mesh as a binary glTF.
pub fn write(mesh: &Mesh) -> Vec<u8> {
    // ── The buffer: positions, then normals, then indices ────────────────────
    let mut binary: Vec<u8> = Vec::with_capacity(mesh.positions.len() * 24 + mesh.indices.len() * 4);
    for point in &mesh.positions {
        for axis in point {
            binary.extend_from_slice(&micrometre(*axis).to_le_bytes());
        }
    }
    let normals_at = binary.len();
    for normal in &mesh.normals {
        for axis in normal {
            binary.extend_from_slice(&micrometre(*axis).to_le_bytes());
        }
    }
    let indices_at = binary.len();
    for index in &mesh.indices {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    let indices_length = binary.len() - indices_at;
    // Every chunk is four-byte aligned, and so is every view inside it.
    while binary.len() % 4 != 0 {
        binary.push(0);
    }

    let (min, max) = mesh.bounds();
    let rounded = |v: [f32; 3]| json!([micrometre(v[0]), micrometre(v[1]), micrometre(v[2])]);

    let count = mesh.positions.len();
    let document = json!({
        "asset": { "version": "2.0", "generator": "the-pult stock catalogue" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{
            "primitives": [{
                "attributes": { "POSITION": 0, "NORMAL": 1 },
                "indices": 2,
                "material": 0,
                "mode": 4
            }]
        }],
        "materials": [{
            "pbrMetallicRoughness": {
                "baseColorFactor": [
                    mesh.material.colour[0], mesh.material.colour[1], mesh.material.colour[2], 1.0
                ],
                "metallicFactor": mesh.material.metalness,
                "roughnessFactor": mesh.material.roughness
            },
            "doubleSided": false
        }],
        "accessors": [
            {
                "bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
                "min": rounded(min), "max": rounded(max)
            },
            { "bufferView": 1, "componentType": 5126, "count": count, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": normals_at, "target": 34962 },
            { "buffer": 0, "byteOffset": normals_at, "byteLength": indices_at - normals_at, "target": 34962 },
            { "buffer": 0, "byteOffset": indices_at, "byteLength": indices_length, "target": 34963 }
        ],
        "buffers": [{ "byteLength": binary.len() }]
    });

    let mut json_bytes = serde_json::to_vec(&sorted(document)).expect("a document we built");
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }

    const HEADER: usize = 12;
    const CHUNK_HEADER: usize = 8;
    let total = HEADER + CHUNK_HEADER + json_bytes.len() + CHUNK_HEADER + binary.len();

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    out.extend_from_slice(&[b'B', b'I', b'N', 0]);
    out.extend_from_slice(&binary);
    out
}

/// The document with every object's keys in one order.
///
/// `serde_json`'s map is ordered already, but only because of how this crate happens
/// to be built; saying so here means the determinism does not rest on a feature flag
/// somebody could turn on for an unrelated reason.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), sorted(map[&key].clone()));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_is_twelve_triangles_and_its_bounds_are_what_it_was_asked_for() {
        let mut mesh = Mesh::default();
        mesh.add_box([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]);
        assert_eq!(mesh.indices.len(), 36);
        assert_eq!(mesh.positions.len(), 24);
        let (min, max) = mesh.bounds();
        assert_eq!(min, [-1.0, -2.0, -3.0]);
        assert_eq!(max, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_tube_is_as_wide_as_it_was_asked_for() {
        let mut mesh = Mesh::default();
        mesh.add_tube([-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.05, 16);
        let (min, max) = mesh.bounds();
        assert!((min[0] + 1.0).abs() < 1e-5 && (max[0] - 1.0).abs() < 1e-5);
        assert!((max[1] - 0.05).abs() < 1e-3, "{max:?}");
        assert!((max[2] - 0.05).abs() < 1e-3, "{max:?}");
    }

    /// The header and the two chunks, which is the whole of the container.
    #[test]
    fn the_bytes_are_a_glb() {
        let mut mesh = Mesh::default();
        mesh.add_box([0.0; 3], [1.0; 3]);
        let bytes = write(&mesh);

        assert_eq!(&bytes[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize, bytes.len());
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(&bytes[16..20], b"JSON");
        assert_eq!(json_len % 4, 0, "the JSON chunk is padded to four bytes");
        let document: Value = serde_json::from_slice(&bytes[20..20 + json_len]).expect("it parses");
        assert_eq!(document["asset"]["version"], "2.0");
        assert_eq!(&bytes[20 + json_len + 4..20 + json_len + 8], &[b'B', b'I', b'N', 0]);
    }
}
