//! The GDTF 1.2 object model (DIN SPEC 15800:2022).
//!
//! One module per section of the spec, serde-derived both ways so the same types
//! read a Share file and write one. Two rules hold throughout:
//!
//! - **Attributes are `@Name`.** quick-xml distinguishes an attribute from a child
//!   element by that prefix, and getting it wrong is silent: the field simply never
//!   appears.
//! - **Field order is element order.** quick-xml writes fields in declaration order
//!   and the XSD is sequence-ordered, so a struct's fields are declared in the order
//!   the spec lists them — attributes first, then children.
//!
//! Everything optional is `Option` or `#[serde(default)]`, because Share files vary:
//! a reader that insists on a field the spec calls optional fails whole
//! manufacturers over an attribute nobody writes.

pub mod attributes;
pub mod dmx_modes;
pub mod geometries;
pub mod models;
pub mod physical;
pub mod protocols;
pub mod wheels;

use serde::{Deserialize, Serialize};

use crate::values::de_number_opt;

pub use attributes::*;
pub use dmx_modes::*;
pub use geometries::*;
pub use models::*;
pub use physical::*;
pub use protocols::*;
pub use wheels::*;

/// The root of `description.xml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "GDTF")]
pub struct Gdtf {
    #[serde(rename = "@DataVersion")]
    pub data_version: String,
    #[serde(rename = "FixtureType")]
    pub fixture_type: FixtureType,
}

impl Gdtf {
    /// The version this crate writes. Reading is not restricted to it: a 1.0 file
    /// parses through the same model, since every later addition is optional.
    pub const DATA_VERSION: &'static str = "1.2";
}

/// Everything about one fixture type.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FixtureType {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(
        rename = "@ShortName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub short_name: String,
    #[serde(
        rename = "@LongName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub long_name: String,
    #[serde(
        rename = "@Manufacturer",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub manufacturer: String,
    #[serde(
        rename = "@Description",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub description: String,
    /// The identity of the type across revisions and across consoles. The console
    /// uses it as the `FixtureType` primary key for anything imported, which is what
    /// makes re-importing a newer revision an update rather than a duplicate.
    #[serde(
        rename = "@FixtureTypeID",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub fixture_type_id: String,
    #[serde(
        rename = "@Thumbnail",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub thumbnail: Option<String>,
    #[serde(
        rename = "@ThumbnailOffsetX",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub thumbnail_offset_x: Option<i32>,
    #[serde(
        rename = "@ThumbnailOffsetY",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub thumbnail_offset_y: Option<i32>,
    #[serde(rename = "@RefFT", default, skip_serializing_if = "Option::is_none")]
    pub ref_ft: Option<String>,
    #[serde(
        rename = "@CanHaveChildren",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub can_have_children: Option<YesNo>,

    #[serde(rename = "AttributeDefinitions", default)]
    pub attribute_definitions: AttributeDefinitions,
    #[serde(rename = "Wheels", default, skip_serializing_if = "Option::is_none")]
    pub wheels: Option<Wheels>,
    #[serde(
        rename = "PhysicalDescriptions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub physical_descriptions: Option<PhysicalDescriptions>,
    #[serde(rename = "Models", default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Models>,
    #[serde(rename = "Geometries", default)]
    pub geometries: Geometries,
    #[serde(rename = "DMXModes", default)]
    pub dmx_modes: DmxModes,
    #[serde(rename = "Revisions", default, skip_serializing_if = "Option::is_none")]
    pub revisions: Option<Revisions>,
    #[serde(rename = "FTPresets", default, skip_serializing_if = "Option::is_none")]
    pub ft_presets: Option<FtPresets>,
    #[serde(rename = "Protocols", default, skip_serializing_if = "Option::is_none")]
    pub protocols: Option<Protocols>,
}

/// The spec's boolean, which is spelled in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum YesNo {
    Yes,
    No,
}

impl From<bool> for YesNo {
    fn from(value: bool) -> Self {
        if value {
            YesNo::Yes
        } else {
            YesNo::No
        }
    }
}

impl From<YesNo> for bool {
    fn from(value: YesNo) -> Self {
        matches!(value, YesNo::Yes)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Revisions {
    #[serde(rename = "Revision", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Revision>,
}

/// One entry in the type's history. The console shows the latest one's text beside
/// an imported type, which is how an operator tells two files of the same fixture
/// apart when the name cannot.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Revision {
    #[serde(rename = "@Text", default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(rename = "@Date", default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(rename = "@UserID", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub user_id: Option<u32>,
    #[serde(
        rename = "@ModifiedBy",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub modified_by: String,
}

/// Presets a manufacturer shipped with the file. Carried across a round trip and
/// otherwise untouched — the console has no concept to map them onto.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FtPresets {
    #[serde(rename = "FTPreset", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<FtPreset>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FtPreset {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}
