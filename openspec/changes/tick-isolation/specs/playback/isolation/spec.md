## Purpose

The show keeps running whatever else the station is asked to do. A plugin in a
write loop, a browser pulling the whole rig, a showfile being saved and a peer
catching up are all ordinary, and none of them is allowed to cost the operator a
frame. This says what may delay playback, what happens to work that cannot keep
up, and what the station reports about it.

## ADDED Requirements

### Requirement: Playback does not wait for anything that is not playback

A station's playback tick SHALL NOT be delayed by plugin activity, by client
requests, by peer synchronisation, or by disk writes. Playback SHALL run
independently of the task that serves those, so that work queued by any of them
cannot occupy the moment a tick is due.

The tick has a 25 ms deadline and everything else on this list does not. Sharing
one queue between work with a deadline and work without one means the deadline is
missed by whatever arrived first.

#### Scenario: A plugin writing continuously

- **WHEN** a plugin writes to the show as fast as the host will accept for a sustained period
- **THEN** playback continues to tick within its budget
- **AND** the plugin's writes are still applied

#### Scenario: A client reading the whole show

- **WHEN** a client requests every collection of a rig of thousands of fixtures, repeatedly
- **THEN** playback continues to tick within its budget
- **AND** each request is answered

#### Scenario: The showfile being written

- **WHEN** the station persists a large change, or the oplog is appended to and pruned
- **THEN** playback continues to tick within its budget

#### Scenario: All of them together

- **WHEN** a plugin writes continuously, a client reads the whole show repeatedly, and the showfile is being written, all at once
- **THEN** playback still ticks within its budget for the duration
- **AND** no tick is skipped as a way of meeting it

### Requirement: Reading the show does not cost more as the rig grows

The cost of a tick reading the show state it needs SHALL NOT grow in proportion to
the size of the rig on every tick. Show data SHALL be readable by playback without
being converted from its stored representation each time it is read.

A tick that re-derives the whole show to look at it turns every fixture into a
per-tick cost, which is what made a 2000-fixture rig miss its deadline by an order
of magnitude while the computation it was doing took a fortieth of a millisecond.

#### Scenario: A rig four times the size

- **WHEN** a station runs a rig four times larger than another, with the same number of live sequences
- **THEN** the tick's cost does not rise in proportion to the fixture count merely to read the show

#### Scenario: A show nobody is editing

- **WHEN** a show is running and no operator, plugin or peer has written to it for several seconds
- **THEN** the tick performs no repeated conversion of unchanged show data

### Requirement: A late tick loses smoothness, never correctness

Where a station cannot keep to its tick budget, playback SHALL still place every
value at the position the wall clock gives it, rather than accumulating from the
previous tick. Two stations under different load running the same show SHALL agree
on what the rig is doing.

This is the property that already makes stations agree at all; it is stated here
because it is what makes "the tick may occasionally be late" an acceptable failure
rather than a divergence.

#### Scenario: One station heavily loaded and one idle

- **WHEN** two stations run the same show and one is loaded enough to miss ticks
- **THEN** both put the rig in the same state for a given moment
- **AND** a cue reaches the end of its fade at the same time on both

#### Scenario: A tick that is skipped entirely

- **WHEN** a station is late enough that a tick's moment passes before it runs
- **THEN** the next tick lands on the value belonging to the time it runs at, not the one it missed

### Requirement: Output continues while the engine is busy

Values already computed SHALL continue to leave the station while the engine is
occupied. Where the output side cannot keep up with playback, the update SHALL be
dropped rather than made to wait, because the next tick carries the same state.

#### Scenario: An output that has fallen behind

- **WHEN** an output connector is slower than the tick producing frames for it
- **THEN** playback is not slowed to match it
- **AND** the output sends the most recent state rather than a backlog of stale ones

### Requirement: No kind of work can starve another

Requests reaching the engine SHALL be admitted per source — plugins, clients,
peers and the station's own housekeeping — such that no single source occupies the
engine to the exclusion of the others. A source that exceeds its share SHALL be
slowed rather than dropped, and SHALL NOT fail with an error that suggests the
station is broken.

#### Scenario: A plugin outpacing a person

- **WHEN** a plugin issues writes far faster than a person can
- **AND** an operator writes at the same time from a browser
- **THEN** the operator's write is applied without waiting for the plugin's backlog to clear

#### Scenario: A source exceeding its share

- **WHEN** one source sends more than the engine will admit at once
- **THEN** it is made to wait rather than refused
- **AND** it receives no error, because being asked to slow down is not a failure

### Requirement: A station reports where its tick's time went

A station SHALL report what its tick spent reading show state, what it spent
computing playback, and what it spent applying the result, as three figures rather
than two.

Two figures were enough to show that computing was not the cost. They were not
enough to say what was, and the answer — reading, at 93% — was found only by adding
a counter by hand and removing it again, which is the thing the published figures
exist to make unnecessary.

#### Scenario: A large rig under load

- **WHEN** a station runs a rig large enough for the difference to matter
- **THEN** the reading, computing and applying figures are published separately
- **AND** they account for the whole tick between them

#### Scenario: A station that did not tick

- **WHEN** a station performed no ticks in a reporting window
- **THEN** none of the three figures is reported, rather than three zeroes

### Requirement: The show's state is reachable by every existing path

Changing how show state is held SHALL NOT change what can be read or written
through a path, what a client or plugin observes, what is persisted, or what
reaches a peer. A collection added to the schema SHALL remain readable, writable,
persisted, synced and visible to clients with no hand-written edit outside the
schema crate.

#### Scenario: A newly added collection

- **WHEN** a new entity type with a table is added to the schema and code generation is run
- **THEN** it is readable, writable, persisted, synced and visible to clients
- **AND** no file outside the schema crate was edited by hand to achieve it

#### Scenario: An existing showfile

- **WHEN** a showfile written before this change is opened
- **THEN** it loads with the same contents it had
- **AND** what a peer or client sees is unchanged

#### Scenario: A path verb

- **WHEN** a write arrives carrying `__create`, `__delete`, `__by`, `__home` or `__set_home`
- **THEN** it behaves exactly as it did before
