//! The programmer entry id, derived the way the frontend derives it.
//!
//! Two implementations of one naming scheme (`frontend/src/lib/programmer.ts`
//! is the other), because a derived id is the whole mechanism that lets two
//! writers of one parameter converge on one row — a command line minting fresh
//! ids would leave a second row fighting the values panel over every fader.
//! The pinned values in the tests below appear verbatim in the frontend's
//! suite, so the two can only drift loudly.

const FNV_PRIME: u64 = 1099511628211;
const FNV_OFFSET: u64 = 14695981039346656037;

fn fnv1a(text: &str, seed: u64) -> u64 {
    let mut hash = seed;
    // charCodeAt on the JS side: UTF-16 code units. Everything hashed here is
    // ASCII (a uuid, a slash, a parameter key), where code units are bytes.
    for unit in text.encode_utf16() {
        hash ^= unit as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// The id of the programmer entry for one parameter of one fixture:
/// two FNV-1a passes over `"<fixture_id>/<key>"`, dressed as a version-8 UUID.
pub fn entry_id(fixture_id: &str, key: &str) -> String {
    let source = format!("{fixture_id}/{key}");
    let hi = fnv1a(&source, FNV_OFFSET);
    let lo = fnv1a(&source, FNV_OFFSET ^ u64::MAX);
    let mut hex: Vec<char> = format!("{hi:016x}{lo:016x}").chars().collect();
    // Version 8 (custom) and the RFC 4122 variant, same as the frontend.
    hex[12] = '8';
    let variant = hex[16].to_digit(16).unwrap_or(0) & 0x3 | 0x8;
    hex[16] = char::from_digit(variant, 16).unwrap_or('8');
    let s: String = hex.into_iter().collect();
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned against the TypeScript implementation: these exact pairs must
    /// produce these exact ids in `frontend/src/lib/programmer.test.ts` too.
    /// A change that moves either side breaks one suite or the other.
    #[test]
    fn agrees_with_the_frontend_by_pinned_example() {
        assert_eq!(
            entry_id("2f6b535b-9a71-4c39-9d95-6d6ab2f0f639", "Intensity"),
            "5f13b718-4585-810f-9f90-15d7509267f4"
        );
        assert_eq!(
            entry_id("00000000-0000-0000-0000-000000000000", "ColorRgb"),
            "3ad6b4b5-4891-8a54-ae06-93999b3641bd"
        );
    }

    #[test]
    fn the_result_is_a_wellformed_v8_uuid() {
        let id = entry_id("2f6b535b-9a71-4c39-9d95-6d6ab2f0f639", "Pan");
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "8", "version nibble");
        let variant = id.as_bytes()[19] as char;
        assert!(matches!(variant, '8' | '9' | 'a' | 'b'), "variant nibble was {variant}");
    }
}
