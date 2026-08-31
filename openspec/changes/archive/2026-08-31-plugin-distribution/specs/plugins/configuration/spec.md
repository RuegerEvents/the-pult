## Purpose

Where a plugin's settings live once the plugin itself is carried by the show
rather than sitting in a directory an operator can edit, and which layer wins
when more than one of them has an opinion.

## ADDED Requirements

### Requirement: A plugin's configuration is layered

A plugin's effective configuration SHALL be composed of, in increasing order of
precedence:

1. the defaults declared in the plugin's own manifest,
2. the show-level configuration held on the plugin's roster entry,
3. the station-level configuration held on the machine.

Layers SHALL be merged key by key rather than replaced wholesale, so a station
overriding one key keeps the show's values for the others. The composed
configuration SHALL be what the plugin receives when it starts.

#### Scenario: A station overrides one key

- **WHEN** the show sets a plugin's provider and model, and a station sets only the model
- **THEN** that plugin on that station runs with the show's provider and the station's model
- **AND** every other station runs with the show's provider and model

#### Scenario: A plugin with nothing configured anywhere

- **WHEN** neither the show nor the station configures a plugin
- **THEN** it starts with the defaults declared in its manifest

### Requirement: Show-level configuration replicates; station-level does not

Show-level plugin configuration SHALL be persisted in the showfile and SHALL
replicate to peer stations. Station-level plugin configuration SHALL be held on
the machine, SHALL NOT be written into the showfile, and SHALL NOT replicate.

Secrets SHALL have a home that does not travel with a showfile: credentials
SHALL be supplied either through the station-level configuration or through the
environment passthrough a manifest already declares.

#### Scenario: A showfile is copied to another console

- **WHEN** a showfile is copied to a machine with its own station-level plugin configuration
- **THEN** the show's plugin configuration travels with it
- **AND** the first machine's station-level values do not

#### Scenario: An API key stays on the machine it was entered on

- **WHEN** an operator supplies a credential as station-level configuration
- **THEN** it is not present in the showfile and does not reach any peer station

### Requirement: Changing configuration takes effect without a restart

Changing a plugin's show-level or station-level configuration SHALL restart that
plugin with the new composed configuration, on the stations affected by the
layer that changed, without restarting the station and without disturbing other
plugins.

#### Scenario: Editing a show-level setting during a show

- **WHEN** an operator changes a plugin's show-level configuration
- **THEN** that plugin restarts with the new value on every station
- **AND** other plugins keep running

#### Scenario: Editing a station-level setting

- **WHEN** an operator changes a plugin's station-level configuration on one console
- **THEN** that plugin restarts with the new value on that station alone
