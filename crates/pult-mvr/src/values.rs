//! The scalar types MVR spells as strings.
//!
//! Almost all of them are GDTF's, because MVR was written beside GDTF and shares its
//! conventions: a colour is CIE xyY, a matrix is brace-wrapped rows, geometry is
//! millimetres with Z up. Those come straight from [`pult_gdtf::values`] and are
//! re-exported here so a reader of this crate has one place to look — a second
//! implementation of the space conversion is the one bug that would show up as the
//! screen disagreeing with the lamps.
//!
//! What is genuinely MVR's own is one thing: how a matrix is *written*.

use std::fmt;
use std::str::FromStr;

pub use pult_gdtf::values::{
    basis_from_console, basis_to_console, basis_to_euler_xyz_degrees, de_from_str_opt,
    de_number_opt, euler_xyz_degrees_to_basis, from_console, num, ser_display_opt, to_console,
    ColorCie, Matrix, ParseError,
};

/// A matrix as MVR writes one: four brace-wrapped rows of **three**.
///
/// GDTF writes four columns per row and MVR drops the homogeneous one, so the same
/// numbers print differently in the two formats. [`Matrix`] already reads both — this
/// exists so that writing an MVR gives back the shape it was read from rather than a
/// GDTF-shaped matrix that every other tool would still accept but no other tool
/// would have written.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MvrMatrix(pub Matrix);

impl MvrMatrix {
    /// The translation part, in the file's own millimetres.
    pub fn translation_mm(&self) -> [f32; 3] {
        self.0.translation_mm()
    }

    /// The 3x3 rotation/scale part, row-major and still in the file's own space.
    pub fn basis(&self) -> [[f32; 3]; 3] {
        self.0.basis()
    }
}

impl From<Matrix> for MvrMatrix {
    fn from(matrix: Matrix) -> Self {
        MvrMatrix(matrix)
    }
}

impl FromStr for MvrMatrix {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Matrix::from_str(s).map(MvrMatrix)
    }
}

impl fmt::Display for MvrMatrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in &self.0 .0 {
            write!(f, "{{{},{},{}}}", num(row[0]), num(row[1]), num(row[2]))?;
        }
        Ok(())
    }
}

impl serde::Serialize for MvrMatrix {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for MvrMatrix {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let text = String::deserialize(deserializer)?;
        MvrMatrix::from_str(&text).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matrix_reads_the_rows_of_three_mvr_writes_and_writes_them_back() {
        let text = "{1,0,0}{0,1,0}{0,0,1}{-3600,13900,4000}";
        let matrix: MvrMatrix = text.parse().expect("four rows of three");

        assert_eq!(matrix.translation_mm(), [-3600.0, 13900.0, 4000.0]);
        assert_eq!(matrix.to_string(), text, "and prints the way it was written");
    }

    /// The other spelling: a file that writes GDTF's four columns is still an MVR
    /// this console opens. It exports in MVR's own shape, which is the only thing
    /// that changes.
    #[test]
    fn a_matrix_written_with_four_columns_is_read_too() {
        let matrix: MvrMatrix = "{1,0,0,0}{0,1,0,0}{0,0,1,0}{1,2,3,1}"
            .parse()
            .expect("four rows of four");

        assert_eq!(matrix.translation_mm(), [1.0, 2.0, 3.0]);
        assert_eq!(matrix.to_string(), "{1,0,0}{0,1,0}{0,0,1}{1,2,3}");
    }
}
