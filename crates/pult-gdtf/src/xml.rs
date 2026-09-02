//! Reading and writing `description.xml`.
//!
//! Two thin wrappers over quick-xml's serde support, in one place so the settings
//! that matter — the declaration, the indentation, the root element's name — are
//! decided once rather than at each call.

use serde::{de::DeserializeOwned, Serialize};

use crate::Error;

/// The declaration every GDTF and MVR file starts with.
pub const DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

/// Parse a document, declaration and all.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    // A UTF-8 BOM in front of the declaration is common enough in files written on
    // Windows, and quick-xml treats it as content.
    let text = text.trim_start_matches('\u{feff}');
    Ok(quick_xml::de::from_str(text)?)
}

/// Write a document, declaration and all, indented.
pub fn to_string<T: Serialize>(value: &T, root: &str) -> Result<String, Error> {
    let mut out = String::from(DECLARATION);
    out.push('\n');
    let mut serializer = quick_xml::se::Serializer::with_root(&mut out, Some(root))?;
    serializer.indent(' ', 4);
    value.serialize(serializer)?;
    out.push('\n');
    Ok(out)
}
