//! The seven OpenHaunt module types, and the fixture each one becomes.
//!
//! This is the only place in the system that knows what a module id means. Adoption
//! asks for a fixture type by module id and gets one; nothing else has an opinion.
//!
//! The ids are derived rather than random, so adopting the same kind of module on
//! two consoles — or on the same console after the showfile was rebuilt — lands on
//! one fixture type instead of a pile of identical ones.

use uuid::Uuid;

use crate::types::fixture::{
    FixtureType, ParameterBinding, ParameterDefinition, ParameterDirection, ParameterKind,
    ParameterValue,
};

pub const MODULE_TYPE_DMX_OUT: u16 = 0x0001;
pub const MODULE_TYPE_DIGITAL_IN: u16 = 0x0002;
pub const MODULE_TYPE_WS2812: u16 = 0x0003;
pub const MODULE_TYPE_MAINS_RELAY: u16 = 0x0004;
pub const MODULE_TYPE_OLED: u16 = 0x0005;
pub const MODULE_TYPE_DRY_CONTACT: u16 = 0x0006;
pub const MODULE_TYPE_ENVIRONMENT: u16 = 0x0007;

pub const MODULE_TYPES: &[u16] = &[
    MODULE_TYPE_DMX_OUT,
    MODULE_TYPE_DIGITAL_IN,
    MODULE_TYPE_WS2812,
    MODULE_TYPE_MAINS_RELAY,
    MODULE_TYPE_OLED,
    MODULE_TYPE_DRY_CONTACT,
    MODULE_TYPE_ENVIRONMENT,
];

/// Descriptor flag bit 6 means the module switches mains voltage.
pub const MODULE_FLAG_MAINS: u32 = 1 << 6;

/// A module type known to switch mains, before any HTTP call has confirmed it.
///
/// The panel warns from the mDNS record alone, because the warning is worth showing
/// a moment early rather than a round trip late.
pub fn is_mains_module(module_type: u16) -> bool {
    module_type == MODULE_TYPE_MAINS_RELAY
}

/// Namespace for OpenHaunt builtin fixture type ids. Arbitrary, and fixed forever:
/// changing it orphans every fixture patched against the old ids.
const NAMESPACE: Uuid = Uuid::from_u128(0x0e6f_9f31_5f24_4f3a_9f1d_6f6f7e2c8a01);

/// The stable id of the builtin fixture type for a module.
pub fn builtin_fixture_type_id(module_type: u16) -> Uuid {
    Uuid::new_v5(&NAMESPACE, &module_type.to_be_bytes())
}

pub fn module_name(module_type: u16) -> Option<&'static str> {
    Some(match module_type {
        MODULE_TYPE_DMX_OUT => "DMX Gateway",
        MODULE_TYPE_DIGITAL_IN => "Digital Inputs",
        MODULE_TYPE_WS2812 => "LED Strip",
        MODULE_TYPE_MAINS_RELAY => "Mains Relay",
        MODULE_TYPE_OLED => "Display",
        MODULE_TYPE_DRY_CONTACT => "Dry Contacts",
        MODULE_TYPE_ENVIRONMENT => "Environment Sensor",
        _ => return None,
    })
}

/// The fixture type an adopted module becomes, or None for a module id we have
/// never heard of — a node newer than this console, which is not an error.
pub fn builtin_fixture_type(module_type: u16) -> Option<FixtureType> {
    let name = module_name(module_type)?;
    let parameters = match module_type {
        // A gateway forwards a whole universe. It has no parameters of its own:
        // the lights behind it are patched as their own DMX fixtures.
        MODULE_TYPE_DMX_OUT => Vec::new(),
        MODULE_TYPE_DIGITAL_IN => (0..8).map(contact).collect(),
        MODULE_TYPE_WS2812 => vec![
            driven(ParameterKind::ColorRgb, 0, ParameterValue::Color { r: 0.0, g: 0.0, b: 0.0 }),
            driven(ParameterKind::Intensity, 1, ParameterValue::Float(0.0)),
        ],
        MODULE_TYPE_MAINS_RELAY => vec![switch(0)],
        MODULE_TYPE_OLED => vec![driven(ParameterKind::Text, 0, ParameterValue::Text(String::new()))],
        MODULE_TYPE_DRY_CONTACT => (0..4).map(switch).collect(),
        MODULE_TYPE_ENVIRONMENT => vec![
            read(ParameterKind::Temperature, 0),
            read(ParameterKind::Humidity, 1),
            read(ParameterKind::AirQuality, 2),
        ],
        _ => return None,
    };

    Some(FixtureType {
        id: builtin_fixture_type_id(module_type),
        name: name.to_string(),
        manufacturer: "OpenHaunt".to_string(),
        // Only the gateway occupies DMX channels, and it occupies all of them.
        channel_count: if module_type == MODULE_TYPE_DMX_OUT { 512 } else { 0 },
        parameters,
    })
}

fn driven(kind: ParameterKind, port: u8, default_value: ParameterValue) -> ParameterDefinition {
    ParameterDefinition {
        kind,
        direction: ParameterDirection::Output,
        binding: ParameterBinding::Port { index: port },
        default_value,
    }
}

fn read(kind: ParameterKind, port: u8) -> ParameterDefinition {
    ParameterDefinition {
        kind,
        direction: ParameterDirection::Input,
        binding: ParameterBinding::Port { index: port },
        default_value: ParameterValue::Float(0.0),
    }
}

fn switch(port: u8) -> ParameterDefinition {
    driven(ParameterKind::Switch(port), port, ParameterValue::Bool(false))
}

fn contact(port: u8) -> ParameterDefinition {
    ParameterDefinition {
        kind: ParameterKind::Contact(port),
        direction: ParameterDirection::Input,
        binding: ParameterBinding::Port { index: port },
        default_value: ParameterValue::Bool(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_type_resolves_to_a_fixture_type() {
        for module_type in MODULE_TYPES {
            assert!(
                builtin_fixture_type(*module_type).is_some(),
                "module {module_type:#06x} has no fixture type",
            );
        }
    }

    #[test]
    fn a_module_this_console_has_never_heard_of_is_not_an_error() {
        assert!(builtin_fixture_type(0x00ff).is_none());
        assert!(module_name(0x00ff).is_none());
    }

    #[test]
    fn the_ids_are_stable_and_distinct() {
        // Adopting the same module on two consoles has to land on one fixture type,
        // or the same rig ends up with a different id on every node.
        assert_eq!(
            builtin_fixture_type(MODULE_TYPE_MAINS_RELAY).unwrap().id,
            builtin_fixture_type_id(MODULE_TYPE_MAINS_RELAY),
        );
        assert_eq!(
            builtin_fixture_type_id(MODULE_TYPE_MAINS_RELAY).to_string(),
            builtin_fixture_type_id(MODULE_TYPE_MAINS_RELAY).to_string(),
        );

        let ids: std::collections::BTreeSet<Uuid> =
            MODULE_TYPES.iter().map(|t| builtin_fixture_type_id(*t)).collect();
        assert_eq!(ids.len(), MODULE_TYPES.len(), "two modules must not share an id");
    }

    #[test]
    fn what_a_node_reports_is_read_and_what_it_drives_is_written() {
        let inputs = builtin_fixture_type(MODULE_TYPE_DIGITAL_IN).unwrap();
        assert_eq!(inputs.parameters.len(), 8);
        assert!(inputs.parameters.iter().all(|p| p.direction == ParameterDirection::Input));

        let sensor = builtin_fixture_type(MODULE_TYPE_ENVIRONMENT).unwrap();
        assert!(sensor.parameters.iter().all(|p| p.direction == ParameterDirection::Input));

        let relay = builtin_fixture_type(MODULE_TYPE_MAINS_RELAY).unwrap();
        assert!(relay.parameters.iter().all(|p| p.direction == ParameterDirection::Output));
    }

    #[test]
    fn every_parameter_on_a_module_sits_on_a_port_numbered_after_itself() {
        for module_type in MODULE_TYPES {
            let ft = builtin_fixture_type(*module_type).unwrap();
            for (index, parameter) in ft.parameters.iter().enumerate() {
                assert_eq!(
                    parameter.binding,
                    ParameterBinding::Port { index: index as u8 },
                    "{} parameter {index} is not on its own port",
                    ft.name,
                );
            }
        }
    }

    #[test]
    fn only_the_relay_carries_a_mains_warning() {
        assert!(is_mains_module(MODULE_TYPE_MAINS_RELAY));
        assert!(!is_mains_module(MODULE_TYPE_DRY_CONTACT));
        assert!(!is_mains_module(MODULE_TYPE_DMX_OUT));
    }
}
