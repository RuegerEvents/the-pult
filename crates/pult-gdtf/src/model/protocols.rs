//! `<Protocols>`: how something other than plain DMX addresses this fixture.
//!
//! Nothing downstream reads these yet. They are modelled so that a round trip does
//! not silently drop a manufacturer's RDM personality table, which is the sort of
//! loss that only shows up on somebody else's console.

use serde::{Deserialize, Serialize};

use crate::values::{Node, de_from_str_opt, de_number_opt, ser_display_opt};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Protocols {
    #[serde(rename = "RDM", default, skip_serializing_if = "Option::is_none")]
    pub rdm: Option<Rdm>,
    #[serde(rename = "Art-Net", default, skip_serializing_if = "Option::is_none")]
    pub art_net: Option<ProtocolMaps>,
    #[serde(rename = "sACN", default, skip_serializing_if = "Option::is_none")]
    pub sacn: Option<ProtocolMaps>,
    #[serde(
        rename = "PosiStageNet",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub posi_stage_net: Option<Empty>,
    #[serde(
        rename = "OpenSoundControl",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub open_sound_control: Option<Empty>,
    #[serde(rename = "CITP", default, skip_serializing_if = "Option::is_none")]
    pub citp: Option<Empty>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Empty {}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProtocolMaps {
    #[serde(rename = "Maps", default, skip_serializing_if = "Option::is_none")]
    pub maps: Option<Maps>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Maps {
    #[serde(rename = "Map", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Map>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Map {
    #[serde(rename = "@Key", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub key: Option<u32>,
    #[serde(rename = "@Value", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub value: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Rdm {
    #[serde(
        rename = "@ManufacturerID",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub manufacturer_id: String,
    #[serde(
        rename = "@DeviceModelID",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub device_model_id: String,
    #[serde(
        rename = "SoftwareVersionID",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub software_versions: Vec<SoftwareVersionId>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SoftwareVersionId {
    #[serde(rename = "@Value", default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(
        rename = "DMXPersonality",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub personalities: Vec<DmxPersonality>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DmxPersonality {
    #[serde(rename = "@Value", default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(rename = "@DMXMode", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub dmx_mode: Option<Node>,
}
