## Purpose

What a plugin is allowed to remember between runs, which of the things it
remembers travel with the show and which stay on the machine, and what happens to
that data when the plugin is upgraded or removed.

## ADDED Requirements

### Requirement: A plugin declares the stores it uses

A plugin's manifest SHALL declare each store it uses, giving the store an
identifier unique within that plugin and a scope of either show or station. A
plugin SHALL be able to read and write only the stores it declared, and SHALL NOT
be able to address another plugin's stores.

A manifest declaring two stores with the same identifier, or a store with a scope
that is neither show nor station, SHALL be refused with the reason, as any other
invalid manifest is.

#### Scenario: Addressing an undeclared store

- **WHEN** a plugin reads or writes a store identifier its manifest does not declare
- **THEN** the call fails with an error naming the undeclared store
- **AND** no data is written

#### Scenario: Two plugins using the same store identifier

- **WHEN** two plugins each declare a store called `cache` and each write the key `grammar`
- **THEN** each reads back only its own value

### Requirement: A plugin can store, retrieve, list and delete values

A plugin SHALL be able to write a JSON value at a key in one of its stores, read
it back, delete it, and list the keys in a store filtered by prefix. A read of a
key that holds nothing SHALL be reported as absent rather than as an error.

Values written SHALL be readable unchanged by the same plugin after the station
restarts, subject to the scope's own guarantees below.

#### Scenario: A value survives a restart

- **WHEN** a plugin writes a value and the station is restarted
- **THEN** reading that key returns the value that was written

#### Scenario: A value survives a hot reload

- **WHEN** a plugin's files change and it is reloaded while the station is up
- **THEN** the new instance reads back what the previous one stored

#### Scenario: Reading a key that was never written

- **WHEN** a plugin reads a key it has not written
- **THEN** the result is absent, and is not an error

#### Scenario: Listing by prefix

- **WHEN** a plugin lists a store with a prefix
- **THEN** it receives exactly the keys in that store beginning with that prefix

### Requirement: Show-scoped data travels with the show

Data written to a show-scoped store SHALL be persisted in the showfile and SHALL
replicate to every peer station in the session. It SHALL be present after the
showfile is reopened and after it is copied to another machine.

#### Scenario: A macro written on one console is available on another

- **WHEN** a plugin on one station writes to a show-scoped store
- **THEN** the same plugin on every peer station reads the same value
- **AND** the value is still there after any station reopens the showfile

#### Scenario: A showfile is handed to someone else

- **WHEN** a showfile is copied to another machine and opened
- **THEN** show-scoped plugin data is present in it

### Requirement: Station-scoped data stays on the machine

Data written to a station-scoped store SHALL be persisted on the station that
wrote it, SHALL NOT be written into the showfile, and SHALL NOT replicate to any
peer. It SHALL be independent of which show is open, and SHALL be present after
that station restarts.

#### Scenario: A cache does not follow the show

- **WHEN** a plugin writes to a station-scoped store on one station
- **THEN** no peer station reads that value
- **AND** the value is absent from the showfile

#### Scenario: A cache outlives the show it was written under

- **WHEN** a station writes station-scoped data, then opens a different show
- **THEN** the same plugin on that station still reads the value

### Requirement: Plugin data does not appear as something a person did

Writes to a plugin store SHALL NOT be undoable and SHALL NOT appear in the
show's history of operator actions.

#### Scenario: Undo after a plugin writes

- **WHEN** a plugin writes to a show-scoped store and an operator then presses undo
- **THEN** the operator's own last change is undone, not the plugin's write

#### Scenario: The history stays a record of people

- **WHEN** a plugin writes to a show-scoped store
- **THEN** no entry for it appears in the history of what people changed

### Requirement: Stores are bounded

Each store SHALL have a maximum number of keys and a maximum total size. A
manifest MAY declare limits lower than the defaults and SHALL NOT be able to
raise them above. A write that would exceed either limit SHALL fail with an error
naming the limit, and SHALL leave the store as it was.

#### Scenario: Writing past the ceiling

- **WHEN** a plugin writes a value that would take a store over its size limit
- **THEN** the write fails naming the limit
- **AND** the store's existing contents are unchanged

#### Scenario: A plugin asking for more than the default

- **WHEN** a manifest declares a limit above the default
- **THEN** the manifest is refused with the reason

### Requirement: Data outlives the plugin that wrote it

Removing, disabling or upgrading a plugin SHALL NOT delete its stores. A plugin
that is installed again SHALL read back what it stored before.

An operator SHALL be able to see which stored data belongs to no installed
plugin, and SHALL be able to delete it deliberately.

#### Scenario: Removing a plugin by mistake

- **WHEN** a plugin is removed and then installed again
- **THEN** it reads back the data it stored before it was removed

#### Scenario: Upgrading a plugin to a new version

- **WHEN** a plugin's bundle is replaced by a different version with the same plugin id
- **THEN** the new version reads back the data the previous version stored

#### Scenario: Clearing out data nobody owns

- **WHEN** an operator inspects stored data belonging to no installed plugin
- **THEN** it is shown with the plugin id that wrote it and can be deleted

### Requirement: An existing plugin keeps working

A plugin built before this interface existed SHALL continue to load and run
unchanged, and SHALL NOT be required to declare any store.

#### Scenario: A plugin that stores nothing

- **WHEN** a plugin whose manifest declares no stores is loaded
- **THEN** it loads and runs exactly as it did before
