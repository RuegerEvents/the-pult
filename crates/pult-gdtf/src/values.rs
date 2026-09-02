//! The scalar types GDTF spells as strings.
//!
//! DIN SPEC 15800 gives most of its attributes a syntax rather than a type: a DMX
//! value is `255/1`, a colour is three comma-separated numbers in CIE xyY, a
//! position is four brace-wrapped rows of four. Each of them gets a `FromStr` and a
//! `Display` here, and serde goes through those, so the object model can hold a
//! parsed value and still write back exactly the shape the schema expects.

use std::fmt;
use std::str::FromStr;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

/// `Serialize`/`Deserialize` for a type whose wire form is its `Display`.
///
/// Every value below is an attribute in the XML, so serde has to see a string; going
/// through `FromStr` and `Display` means the parse and the print stay one pair
/// instead of four.
macro_rules! serde_via_string {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                raw.trim().parse().map_err(D::Error::custom)
            }
        }
    };
}

/// What went wrong reading one of the string-shaped values below.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{what}: {input:?}")]
pub struct ParseError {
    pub what: &'static str,
    pub input: String,
}

impl ParseError {
    fn new(what: &'static str, input: &str) -> Self {
        Self {
            what,
            input: input.to_string(),
        }
    }
}

/// Serde helpers for an *optional* string-shaped attribute.
///
/// The non-optional case never comes up: every value below that is written as an
/// attribute is optional in the spec, and the required ones are plain strings.
///
/// Public rather than private to this crate because `pult-mvr` reads the same shapes
/// out of the same kind of file, and a second copy of the leniency below is a second
/// place for it to be wrong.
pub fn de_from_str_opt<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        // Two spellings of "nothing here", and both are the spec's own. An attribute
        // written as `Color=""` is one; the literal `None` is the other, and it is what
        // a real file writes for `Highlight` on a channel that has no highlight value
        // and for `Offset` on a channel that occupies no DMX slot. Rejecting either
        // fails a whole fixture over an attribute that was saying it had nothing to
        // say — which is how the first file this reader was pointed at from the Share
        // failed to import.
        Some(text) if text.trim().is_empty() || text.trim() == "None" => Ok(None),
        Some(text) => T::from_str(text.trim()).map(Some).map_err(D::Error::custom),
    }
}

/// A *value* an attribute might not really have.
///
/// [`de_from_str_opt`] with the last step softened: where that one fails the whole
/// fixture over an attribute it cannot parse, this reads it as absent. For a colour
/// that is the right answer and the strict version is the wrong one — a real Robe
/// file writes `Color="nan,nan,nan"` on a black slot, which says the slot has no
/// colour rather than that the file is broken.
pub fn de_value_opt<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else { return Ok(None) };
    let text = raw.trim();
    if text.is_empty() || text == "None" {
        return Ok(None);
    }
    Ok(T::from_str(text).ok())
}

/// A number an attribute might not really have.
///
/// Read leniently, and that is not politeness — it is what other people's files
/// require. A real Share file writes `Highlight="None"` for a channel with no
/// highlight, `-2147483648` in an *unsigned* field for a value nobody set, and `1.0`
/// where an integer belongs. A reader that failed on any of them would refuse a
/// fixture somebody has to patch tonight over an attribute that was saying it had
/// nothing to say. Anything unreadable becomes absent, which is what it meant.
pub fn de_number_opt<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else { return Ok(None) };
    let text = raw.trim();
    if text.is_empty() || text == "None" {
        return Ok(None);
    }
    if let Ok(value) = T::from_str(text) {
        return Ok(Some(value));
    }
    // `1.0` in a field the spec calls an integer. Truncating rather than rejecting,
    // because a count of `3.0` is three.
    match text.parse::<f64>() {
        Ok(number) if number.fract() == 0.0 => Ok(T::from_str(&format!("{}", number as i64)).ok()),
        _ => Ok(None),
    }
}

pub fn ser_display_opt<S, T>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: fmt::Display,
{
    match value {
        Some(value) => serializer.collect_str(value),
        None => serializer.serialize_none(),
    }
}

// ── DMX values ───────────────────────────────────────────────────────

/// A DMX value with the byte count it was written at: `255/1`, `65535/2`, `128/1s`.
///
/// The byte count is not decoration. `Default="128/1"` on a 16-bit channel means the
/// coarse byte is 128 and the fine byte is whatever the shift rule gives, so a reader
/// that dropped the `/1` would put 128 into a 16-bit channel and land the fixture at
/// half a percent instead of half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmxValue {
    pub value: u32,
    pub byte_count: u8,
    /// The `s` suffix: rescale by shifting rather than by the value range, which is
    /// what makes `255/1s` become `65535` rather than `65280` at two bytes.
    pub shifting: bool,
}

impl DmxValue {
    pub fn new(value: u32, byte_count: u8) -> Self {
        Self {
            value,
            byte_count,
            shifting: false,
        }
    }

    /// The largest value a channel of `byte_count` bytes can hold.
    pub fn max_for(byte_count: u8) -> u32 {
        match byte_count {
            0 => 0,
            n if n >= 4 => u32::MAX,
            n => (1u32 << (8 * n as u32)) - 1,
        }
    }

    /// This value expressed at a different width.
    ///
    /// Widening a non-shifting value scales by the ratio of the ranges (so full stays
    /// full); a shifting one moves the bits up and leaves zeros behind, which is what
    /// the spec's `s` asks for. Narrowing takes the high bytes either way.
    pub fn rescale(&self, byte_count: u8) -> u32 {
        if byte_count == self.byte_count || byte_count == 0 {
            return self.value;
        }
        if byte_count > self.byte_count {
            let shift = 8 * (byte_count - self.byte_count) as u32;
            if self.shifting {
                return self.value << shift;
            }
            let from_max = Self::max_for(self.byte_count) as u64;
            if from_max == 0 {
                return 0;
            }
            let to_max = Self::max_for(byte_count) as u64;
            ((self.value as u64 * to_max + from_max / 2) / from_max) as u32
        } else {
            self.value >> (8 * (self.byte_count - byte_count) as u32)
        }
    }
}

impl FromStr for DmxValue {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // The spec's grammar is `value/bytes`, and real files write a bare number:
        // three of the first five downloaded from the Share do. One byte is what a
        // bare one means, and refusing it refuses the whole fixture.
        let Some((value, rest)) = s.split_once('/') else {
            return Ok(DmxValue {
                value: s.trim().parse().map_err(|_| ParseError::new("DMX value", s))?,
                byte_count: 1,
                shifting: false,
            });
        };
        let shifting = rest.ends_with('s') || rest.ends_with('S');
        let digits = if shifting {
            &rest[..rest.len() - 1]
        } else {
            rest
        };
        Ok(DmxValue {
            value: value
                .trim()
                .parse()
                .map_err(|_| ParseError::new("DMX value", s))?,
            byte_count: digits
                .trim()
                .parse()
                .map_err(|_| ParseError::new("DMX value", s))?,
            shifting,
        })
    }
}

impl fmt::Display for DmxValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}{}",
            self.value,
            self.byte_count,
            if self.shifting { "s" } else { "" }
        )
    }
}

serde_via_string!(DmxValue);

// ── Colour ───────────────────────────────────────────────────────────

/// A colour in CIE 1931 xyY, which is how GDTF says every colour it knows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCie {
    pub x: f32,
    pub y: f32,
    /// Luminance, 0..100 in the spec's own units.
    pub luminance: f32,
}

impl FromStr for ColorCie {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(',').map(str::trim);
        let mut next = || -> Result<f32, ParseError> {
            parts
                .next()
                .ok_or_else(|| ParseError::new("CIE colour", s))?
                .parse()
                .map_err(|_| ParseError::new("CIE colour", s))
        };
        let (x, y, luminance) = (next()?, next()?, next()?);
        // `Color="nan,nan,nan"` is in a real Robe file, on a colour wheel's black
        // slot, and Rust parses "nan" into a number quite happily. What follows is
        // worse than a refusal: a NaN reaches the schema, `serde_json` writes it as
        // `null`, and the row is stored and can never be read back — silent loss with
        // no bad data to blame. A colour that is not a number is not a colour.
        if !x.is_finite() || !y.is_finite() || !luminance.is_finite() {
            return Err(ParseError::new("CIE colour", s));
        }
        Ok(ColorCie { x, y, luminance })
    }
}

impl fmt::Display for ColorCie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{}", num(self.x), num(self.y), num(self.luminance))
    }
}

serde_via_string!(ColorCie);

impl ColorCie {
    /// This colour as linear sRGB in 0..1, ignoring luminance.
    ///
    /// What an emitter's colour is for: mixing needs a direction in RGB space, and
    /// the console has no colour management to do anything cleverer with xyY.
    pub fn to_linear_rgb(self) -> [f32; 3] {
        if self.y.abs() < 1e-6 {
            return [0.0, 0.0, 0.0];
        }
        let big_y = 1.0_f32;
        let big_x = self.x * big_y / self.y;
        let big_z = (1.0 - self.x - self.y) * big_y / self.y;
        // sRGB D65 matrix.
        let r = 3.240454 * big_x - 1.537139 * big_y - 0.498531 * big_z;
        let g = -0.969266 * big_x + 1.876011 * big_y + 0.041556 * big_z;
        let b = 0.055643 * big_x - 0.204026 * big_y + 1.057225 * big_z;
        let peak = r.max(g).max(b).max(1e-6);
        [
            (r / peak).clamp(0.0, 1.0),
            (g / peak).clamp(0.0, 1.0),
            (b / peak).clamp(0.0, 1.0),
        ]
    }
}

// ── Node paths ───────────────────────────────────────────────────────

/// A dot-separated reference to something else in the file: `Yoke.Head`,
/// `ColorAdd_R`, `Wheel1.Slot 3`.
///
/// Kept as its parts rather than as the string, because every consumer of one wants
/// to walk it, and splitting at each use is where an off-by-one in the separator
/// lives.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Node(pub Vec<String>);

impl Node {
    pub fn last(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromStr for Node {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(Node(Vec::new()));
        }
        Ok(Node(s.split('.').map(str::to_string).collect()))
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("."))
    }
}

serde_via_string!(Node);

// ── Matrices ─────────────────────────────────────────────────────────

/// A GDTF `Position`: four brace-wrapped rows of four numbers, row-major, with the
/// translation in the last row and in **millimetres**.
///
/// Held row-major exactly as written. Converting to the console's space is
/// [`to_console`], and it is the only place that knows about the axis swap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix(pub [[f32; 4]; 4]);

impl Default for Matrix {
    fn default() -> Self {
        Matrix([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }
}

impl Matrix {
    /// The translation part, in the file's own millimetres.
    pub fn translation_mm(&self) -> [f32; 3] {
        [self.0[3][0], self.0[3][1], self.0[3][2]]
    }

    /// The 3x3 rotation/scale part, row-major.
    pub fn basis(&self) -> [[f32; 3]; 3] {
        [
            [self.0[0][0], self.0[0][1], self.0[0][2]],
            [self.0[1][0], self.0[1][1], self.0[1][2]],
            [self.0[2][0], self.0[2][1], self.0[2][2]],
        ]
    }
}

impl FromStr for Matrix {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut rows = [[0.0f32; 4]; 4];
        let mut seen = 0usize;
        for chunk in s.split('}') {
            let Some((_, body)) = chunk.split_once('{') else {
                continue;
            };
            if seen >= 4 {
                return Err(ParseError::new("matrix", s));
            }
            let mut cols = 0usize;
            for (index, part) in body.split(',').map(str::trim).enumerate() {
                if index >= 4 {
                    return Err(ParseError::new("matrix", s));
                }
                rows[seen][index] = part.parse().map_err(|_| ParseError::new("matrix", s))?;
                cols = index + 1;
            }
            // MVR writes 4x3 in places; the missing column is the homogeneous one.
            if cols == 3 {
                rows[seen][3] = if seen == 3 { 1.0 } else { 0.0 };
            } else if cols != 4 {
                return Err(ParseError::new("matrix", s));
            }
            seen += 1;
        }
        if seen != 4 {
            return Err(ParseError::new("matrix", s));
        }
        Ok(Matrix(rows))
    }
}

impl fmt::Display for Matrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in &self.0 {
            write!(
                f,
                "{{{},{},{},{}}}",
                num(row[0]),
                num(row[1]),
                num(row[2]),
                num(row[3])
            )?;
        }
        Ok(())
    }
}

serde_via_string!(Matrix);

/// A GDTF `Rotation`: three brace-wrapped rows of three.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation(pub [[f32; 3]; 3]);

impl Default for Rotation {
    fn default() -> Self {
        Rotation([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
    }
}

impl FromStr for Rotation {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut rows = [[0.0f32; 3]; 3];
        let mut seen = 0usize;
        for chunk in s.split('}') {
            let Some((_, body)) = chunk.split_once('{') else {
                continue;
            };
            if seen >= 3 {
                return Err(ParseError::new("rotation", s));
            }
            for (index, part) in body.split(',').map(str::trim).enumerate() {
                if index >= 3 {
                    return Err(ParseError::new("rotation", s));
                }
                rows[seen][index] = part.parse().map_err(|_| ParseError::new("rotation", s))?;
            }
            seen += 1;
        }
        if seen != 3 {
            return Err(ParseError::new("rotation", s));
        }
        Ok(Rotation(rows))
    }
}

impl fmt::Display for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in &self.0 {
            write!(f, "{{{},{},{}}}", num(row[0]), num(row[1]), num(row[2]))?;
        }
        Ok(())
    }
}

serde_via_string!(Rotation);

// ── Space conversion ─────────────────────────────────────────────────

/// GDTF and MVR are millimetres, X right, Y upstage, Z up. The console is metres,
/// Y up, Z downstage.
///
/// `(x, z, −y) / 1000` and nothing else. The tempting `(x, z, y)` swap has
/// determinant −1: it converts positions correctly and mirrors every rotation, so a
/// yoke imported through it tilts the wrong way and nothing about the numbers says
/// so.
pub fn to_console(mm: [f32; 3]) -> [f32; 3] {
    [mm[0] / 1000.0, mm[2] / 1000.0, -mm[1] / 1000.0]
}

/// The inverse of [`to_console`], for export.
pub fn from_console(metres: [f32; 3]) -> [f32; 3] {
    [metres[0] * 1000.0, -metres[2] * 1000.0, metres[1] * 1000.0]
}

/// The same change of basis applied to a rotation, so an imported yoke turns the way
/// the file drew it.
pub fn basis_to_console(basis: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    // C = P · B · Pᵀ with P the permutation (x, z, −y), written out because a 3x3
    // matrix library for one multiplication is a dependency for nothing.
    const P: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
    mul3(mul3(P, basis), transpose3(P))
}

/// And back, for export.
pub fn basis_from_console(basis: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    const P: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]];
    mul3(mul3(P, basis), transpose3(P))
}

fn mul3(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn transpose3(a: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for (i, row) in a.iter().enumerate() {
        for (j, cell) in row.iter().enumerate() {
            out[j][i] = *cell;
        }
    }
    out
}

/// Euler angles in degrees, XYZ order — three.js's default, and the console's.
///
/// Extracted from a rotation basis so the schema can hold three numbers a person can
/// type instead of nine they cannot.
pub fn basis_to_euler_xyz_degrees(basis: [[f32; 3]; 3]) -> [f32; 3] {
    // R = Rx·Ry·Rz, so basis[0][2] is sin(y).
    let sy = basis[0][2].clamp(-1.0, 1.0);
    let y = sy.asin();
    let (x, z) = if sy.abs() < 0.999_999 {
        (
            (-basis[1][2]).atan2(basis[2][2]),
            (-basis[0][1]).atan2(basis[0][0]),
        )
    } else {
        // Gimbal lock: roll and yaw are the same axis, so put it all in x.
        (basis[2][1].atan2(basis[1][1]), 0.0)
    };
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

/// The inverse: three degrees back to a basis, XYZ order.
pub fn euler_xyz_degrees_to_basis(euler: [f32; 3]) -> [[f32; 3]; 3] {
    let [x, y, z] = [
        euler[0].to_radians(),
        euler[1].to_radians(),
        euler[2].to_radians(),
    ];
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    [
        [cy * cz, -cy * sz, sy],
        [cx * sz + sx * sy * cz, cx * cz - sx * sy * sz, -sx * cy],
        [sx * sz - cx * sy * cz, sx * cz + cx * sy * sz, cx * cy],
    ]
}

// ── Number formatting ────────────────────────────────────────────────

/// A float the way the spec writes one: no exponent, no trailing `.0` noise, and
/// enough digits that a round trip does not move it.
pub fn num(value: f32) -> String {
    if value == value.trunc() && value.abs() < 1e9 {
        return format!("{}", value as i64);
    }
    let mut text = format!("{value:.6}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    // A value a hair under zero rounds to "-0.000000" and strips to "-0", which is a
    // number no spec asks for and which makes two writes of the same rig differ over
    // a rounding direction.
    if text == "-0" {
        return "0".into();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Robe file writes this on a colour wheel's black slot, and Rust parses
    /// "nan" into a number without complaint. What follows is worse than a refusal: a
    /// NaN reaches the schema, serialises as `null`, and the row can never be read
    /// back — a fixture type stored and lost with nothing to blame.
    #[test]
    fn a_colour_that_is_not_a_number_is_not_a_colour() {
        assert!("nan,nan,nan".parse::<ColorCie>().is_err());
        assert!("0.31,nan,100".parse::<ColorCie>().is_err());
        assert!("inf,0.3,100".parse::<ColorCie>().is_err());
        assert!("0.3127,0.329,100".parse::<ColorCie>().is_ok(), "and a real one still reads");
    }

    #[test]
    fn a_bare_number_is_one_byte_because_that_is_what_real_files_write() {
        let bare: DmxValue = "255".parse().unwrap();
        assert_eq!(bare, DmxValue::new(255, 1));
        assert_eq!(bare.rescale(2), 65535, "and widens like any other one-byte value");
    }

    #[test]
    fn dmx_values_carry_their_width() {
        let coarse: DmxValue = "128/1".parse().unwrap();
        assert_eq!(coarse.rescale(2), 32896, "128/255 of full, at two bytes");
        let shifting: DmxValue = "128/1s".parse().unwrap();
        assert_eq!(shifting.rescale(2), 32768, "shifted, not scaled");
        assert_eq!(coarse.to_string(), "128/1");
        assert_eq!(shifting.to_string(), "128/1s");
    }

    #[test]
    fn a_matrix_round_trips() {
        let text = "{1,0,0,0}{0,1,0,0}{0,0,1,0}{100,200,300,1}";
        let matrix: Matrix = text.parse().unwrap();
        assert_eq!(matrix.translation_mm(), [100.0, 200.0, 300.0]);
        assert_eq!(matrix.to_string(), text);
    }

    #[test]
    fn a_four_by_three_matrix_gets_its_fourth_column() {
        let matrix: Matrix = "{1,0,0}{0,1,0}{0,0,1}{0,0,1000}".parse().unwrap();
        assert_eq!(matrix.0[3][3], 1.0);
        assert_eq!(matrix.translation_mm(), [0.0, 0.0, 1000.0]);
    }

    #[test]
    fn the_space_conversion_is_a_rotation_not_a_mirror() {
        assert_eq!(to_console([1000.0, 2000.0, 3000.0]), [1.0, 3.0, -2.0]);
        assert_eq!(from_console([1.0, 3.0, -2.0]), [1000.0, 2000.0, 3000.0]);

        // A mirror would flip the sign of the determinant; a rotation keeps it at +1.
        let basis = euler_xyz_degrees_to_basis([20.0, 35.0, -50.0]);
        let converted = basis_to_console(basis);
        assert!((determinant(converted) - 1.0).abs() < 1e-4, "{converted:?}");
        let back = basis_from_console(converted);
        for (row, original) in back.iter().zip(basis.iter()) {
            for (got, want) in row.iter().zip(original.iter()) {
                assert!((got - want).abs() < 1e-4, "{back:?} vs {basis:?}");
            }
        }
    }

    #[test]
    fn euler_angles_round_trip_through_a_basis() {
        for angles in [
            [0.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            [10.0, -20.0, 45.0],
            [0.0, 89.0, 12.0],
        ] {
            let back = basis_to_euler_xyz_degrees(euler_xyz_degrees_to_basis(angles));
            for (got, want) in back.iter().zip(angles.iter()) {
                assert!((got - want).abs() < 1e-2, "{back:?} vs {angles:?}");
            }
        }
    }

    #[test]
    fn a_cie_colour_round_trips_and_points_the_right_way() {
        let red: ColorCie = "0.6400,0.3300,21.26".parse().unwrap();
        let rgb = red.to_linear_rgb();
        assert!(rgb[0] > rgb[1] && rgb[0] > rgb[2], "{rgb:?}");
        assert_eq!(red.to_string(), "0.64,0.33,21.26");
    }

    fn determinant(m: [[f32; 3]; 3]) -> f32 {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
}
