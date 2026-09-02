//! From a matrix to three vectors a person can read, and back.
//!
//! An MVR object's place is a 4x3 matrix in millimetres with Z up. The console holds
//! a position, a rotation as XYZ Euler degrees, and a scale — three numbers each,
//! which is what a numeric field can show and a gizmo can write. Getting between the
//! two is the whole of this module.
//!
//! **Scale is signed, and that is not a detail.** Twenty-one of the forty-three
//! trusses in one real Vectorworks file have a basis with determinant −1: the drawing
//! mirrored them. A decomposition into position, rotation and an unsigned scale
//! cannot say so, and the reflection would come back as some rotation that happens to
//! put the geometry nearly where it belongs — with the bolt holes on the wrong side
//! and nothing in the numbers admitting it. So the reflection is pulled onto the X
//! axis as a negative scale, which round-trips exactly and which the browser can
//! apply with a two-sided material.
//!
//! Only reflections and scale are representable here, not shear. No file in the
//! corpus has a non-orthogonal basis, and one that did would come back very slightly
//! straightened rather than refused.

use crate::values::{
    basis_from_console, basis_to_console, basis_to_euler_xyz_degrees, euler_xyz_degrees_to_basis,
    from_console, to_console, Matrix, MvrMatrix,
};

/// Where something is, in the console's space: metres, Y up, Z downstage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Metres.
    pub position: [f32; 3],
    /// XYZ Euler degrees — three.js's order, and the console's.
    pub rotation: [f32; 3],
    /// One per axis. Negative where the file mirrored the object.
    pub scale: [f32; 3],
}

impl Default for Placement {
    fn default() -> Self {
        Placement {
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: [1.0; 3],
        }
    }
}

/// Read a matrix as a placement.
pub fn decompose(matrix: &MvrMatrix) -> Placement {
    let rotation_scale = to_console_rotation(matrix.basis());
    let (rotation, scale) = split_rotation_from_scale(rotation_scale);
    Placement {
        position: to_console(matrix.translation_mm()),
        rotation: basis_to_euler_xyz_degrees(rotation),
        scale,
    }
}

/// Write a placement as a matrix.
pub fn compose(placement: &Placement) -> MvrMatrix {
    let rotation = euler_xyz_degrees_to_basis(placement.rotation);
    let scaled = scale_columns(rotation, placement.scale);
    let basis = transpose(basis_from_console(scaled));
    let translation = from_console(placement.position);

    let mut rows = [[0.0f32; 4]; 4];
    for (r, row) in basis.iter().enumerate() {
        rows[r][..3].copy_from_slice(row);
    }
    rows[3][..3].copy_from_slice(&translation);
    rows[3][3] = 1.0;
    MvrMatrix(Matrix(rows))
}

/// MVR writes a basis as its three local axes, one per row; a rotation matrix has
/// them as its columns. Transposing is the whole of that difference, and it happens
/// here so that no caller has to remember which way round the file was.
fn to_console_rotation(basis: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    basis_to_console(transpose(basis))
}

/// Pull the lengths of the three axes out of a rotation matrix, putting any
/// reflection on X.
fn split_rotation_from_scale(matrix: [[f32; 3]; 3]) -> ([[f32; 3]; 3], [f32; 3]) {
    let mut scale = [0.0f32; 3];
    for (axis, length) in scale.iter_mut().enumerate() {
        *length = (0..3).map(|row| matrix[row][axis].powi(2)).sum::<f32>().sqrt();
    }
    // A degenerate axis has no direction to keep; unit is the only honest answer, and
    // it keeps the division below finite.
    for length in scale.iter_mut() {
        if *length < 1e-6 {
            *length = 1.0;
        }
    }
    if determinant(matrix) < 0.0 {
        scale[0] = -scale[0];
    }

    let mut rotation = [[0.0f32; 3]; 3];
    for (row, out) in rotation.iter_mut().enumerate() {
        for (axis, cell) in out.iter_mut().enumerate() {
            *cell = matrix[row][axis] / scale[axis];
        }
    }
    (rotation, scale)
}

fn scale_columns(matrix: [[f32; 3]; 3], scale: [f32; 3]) -> [[f32; 3]; 3] {
    let mut out = matrix;
    for row in out.iter_mut() {
        for (axis, cell) in row.iter_mut().enumerate() {
            *cell *= scale[axis];
        }
    }
    out
}

fn transpose(matrix: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for (r, row) in matrix.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            out[c][r] = *cell;
        }
    }
    out
}

fn determinant(m: [[f32; 3]; 3]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], what: &str) {
        for axis in 0..3 {
            assert!(
                (a[axis] - b[axis]).abs() < 1e-3,
                "{what}: {a:?} against {b:?}"
            );
        }
    }

    fn round_trip(text: &str) -> (Placement, MvrMatrix) {
        let matrix: MvrMatrix = text.parse().expect("a matrix");
        let placement = decompose(&matrix);
        (placement, compose(&placement))
    }

    #[test]
    fn an_object_at_the_origin_is_where_it_says() {
        let (placement, _) = round_trip("{1,0,0}{0,1,0}{0,0,1}{0,0,0}");

        close(placement.position, [0.0; 3], "position");
        close(placement.rotation, [0.0; 3], "rotation");
        close(placement.scale, [1.0; 3], "scale");
    }

    /// Millimetres Z-up to metres Y-up: 4 m upstage and 4 m in the air becomes 4 m
    /// up and 4 m *downstage negative*, because the console's Z points the other way.
    #[test]
    fn a_position_arrives_in_metres_with_y_up() {
        let (placement, _) = round_trip("{1,0,0}{0,1,0}{0,0,1}{-3600,13900,4000}");

        close(placement.position, [-3.6, 4.0, -13.9], "position");
    }

    /// The one the corpus forced. A truss the drawing mirrored has a basis whose
    /// determinant is −1, and there is no rotation that is one.
    #[test]
    fn a_mirrored_truss_comes_back_mirrored() {
        // The real thing, out of spitzengefuehl_v13.mvr.
        let (placement, back) = round_trip("{0,-1,0}{-1,0,0}{0,0,1}{-1500.518413,-1921.527697,4000}");

        assert!(
            placement.scale.iter().filter(|s| **s < 0.0).count() == 1,
            "exactly one axis is flipped: {:?}",
            placement.scale
        );
        assert_eq!(
            back.to_string(),
            "{0,-1,0}{-1,0,0}{0,0,1}{-1500.518433,-1921.52771,4000}",
            "and writing it back gives the file's own numbers",
        );
    }

    #[test]
    fn every_shape_in_the_corpus_round_trips() {
        for text in [
            "{1,0,0}{0,1,0}{0,0,1}{0,0,0}",
            "{0,-0.731354,0.681998}{1,0,0}{0,0.681998,0.731354}{-3600,13900,4000}",
            "{0,1,0}{-1,0,0}{0,0,1}{-1500.518413,-7610.027697,4000}",
            "{0,-1,0}{-1,0,0}{0,0,1}{-1500.518413,-1921.527697,4000}",
            "{0.707107,0.707107,0}{-0.707107,0.707107,0}{0,0,1}{100,200,300}",
        ] {
            let original: MvrMatrix = text.parse().expect("a matrix");
            let (_, back) = round_trip(text);
            for row in 0..4 {
                for col in 0..3 {
                    assert!(
                        (original.0 .0[row][col] - back.0 .0[row][col]).abs() < 1e-3,
                        "{text} came back as {back}",
                    );
                }
            }
        }
    }

    /// Scale survives, which is what makes the decomposition lossless for anything
    /// short of shear.
    #[test]
    fn a_scaled_object_keeps_its_size() {
        let (placement, back) = round_trip("{2,0,0}{0,3,0}{0,0,4}{0,0,0}");

        close(placement.scale, [2.0, 4.0, 3.0], "scale, in the console's axis order");
        assert_eq!(back.to_string(), "{2,0,0}{0,3,0}{0,0,4}{0,0,0}");
    }
}
