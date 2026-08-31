# plugins/datastores Specification

## Purpose
What a plugin is allowed to remember between runs, which of the things it
remembers travel with the show and which stay on the machine, and what happens to
that data when the plugin is upgraded or removed.

## Requirements

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
it back, delete it, and ask for the keys in a store filtered by prefix. A read of a
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

### Requirement: A plugin's writes are the plugin's, unless the store says otherwise

Writes to a plugin store SHALL NOT be undoable and SHALL NOT appear in the show's
history of operator actions.

A store MAY declare that its writes are the operator's rather than the plugin's.
Writes to such a store SHALL be undoable and SHALL appear in the history,
attributed to the operator whose action caused them, and SHALL be named there by
the plugin, the store and the key rather than by an identifier. This is what a
plugin uses when the operator asked it to save something — a macro, a snippet —
and would expect to take it back.

A write no operator caused SHALL NOT be undoable and SHALL NOT appear in the
history, whatever the store declares. A plugin writing on a timer, while
starting up, or while handling a call no person made has no operator to attribute
the write to, and guessing at one would put a stranger's action into somebody's
undo history.

#### Scenario: Undo after a plugin writes

- **WHEN** a plugin writes to a show-scoped store and an operator then presses undo
- **THEN** the operator's own last change is undone, not the plugin's write

#### Scenario: The history stays a record of people

- **WHEN** a plugin writes to a show-scoped store
- **THEN** no entry for it appears in the history of what people changed

#### Scenario: Taking back what the operator asked the plugin to save

- **WHEN** an operator asks a plugin to save something into a store it declared as the operator's, and then presses undo
- **THEN** what the plugin saved is taken back
- **AND** the history names it by the plugin, the store and the key

#### Scenario: A plugin writing with nobody behind it

- **WHEN** a plugin writes to a store it declared as the operator's, while handling something no person asked for
- **THEN** the write is not undoable and does not appear in the history

### Requirement: One key is one datum, on every station

A key in a show-scoped store SHALL name the same datum on every station in the
session, so that two stations writing the same key of the same store of the same
plugin are writing to one place. Concurrent writes to one key SHALL resolve to a
single value by the same rule the rest of the show uses, and SHALL NOT leave the
store holding that key twice.

#### Scenario: Two stations write the same key

- **WHEN** two stations in a session each write the key `opening` in the same store of the same plugin
- **THEN** the plugin on every station reads back one value for that key, not two entries

#### Scenario: A station that was away catches up

- **WHEN** a station rejoins after a key was written on both sides of the split
- **THEN** the key holds one value once the session has converged

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

### Requirement: A plugin need not use a store, and one built against an earlier contract keeps working

A plugin SHALL NOT be required to declare any store, and one that declares none
SHALL load and run exactly as it did before.

A plugin built against an earlier *minor* version of the plugin contract SHALL
continue to load and run unchanged, so that adding an interface does not strand
the plugins already carried in showfiles. A station SHALL run a plugin whose
declared contract version has the same major and a minor no greater than its
own, and SHALL refuse any other with a reason saying which thing to change — the
plugin rebuilt, or the console updated.

A plugin built against a *different major* — including every version from before
the contract settled at 1.0 — SHALL be refused by name rather than loaded
wrongly. This is a one-time cost, taken because a contract that cannot grow
additively would have to strand its plugins on every future addition instead.

#### Scenario: A plugin that stores nothing

- **WHEN** a plugin whose manifest declares no stores is loaded
- **THEN** it loads and runs exactly as it did before

#### Scenario: A plugin built before the station's contract grew

- **WHEN** a plugin built against an earlier minor of the contract is loaded on a station whose contract has since gained an interface
- **THEN** it loads and runs unchanged, importing only what it was built against

#### Scenario: A plugin built against a contract the station does not have

- **WHEN** a plugin declares a later minor than the station speaks
- **THEN** it is refused with a reason naming the console as the thing to update

#### Scenario: A plugin from before the contract settled

- **WHEN** a plugin declares a different major version of the contract
- **THEN** it is refused with a reason naming the plugin as the thing to rebuild
