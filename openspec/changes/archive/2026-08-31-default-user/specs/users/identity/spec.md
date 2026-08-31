## Purpose

That a show always has somebody to attribute a change to, so undo works before
anybody has been asked to configure anything: who creates that user, how two
stations opening one show agree on it, and what identity a client is working as
before a person has chosen one.

## ADDED Requirements

### Requirement: A show always has a user

Every show SHALL contain at least one user from the moment a station has loaded
it. A station SHALL create a default user when it loads a show whose `users`
collection does not already contain one, and SHALL do so without any client being
connected — a station running headless has plugins and station RPCs writing to it
and must be able to attribute those writes.

The default user SHALL be created for an existing showfile on open, not only for
a newly created show.

#### Scenario: A newly created show

- **WHEN** a station creates a show and loads it
- **THEN** the show's `users` collection contains the default user
- **AND** it is persisted to the showfile

#### Scenario: A showfile written before this change

- **WHEN** a station opens a showfile whose `users` collection is empty
- **THEN** the default user is created and persisted
- **AND** the operations already in that showfile's oplog are unchanged

#### Scenario: No client is attached

- **WHEN** a station loads a show with no browser connected to it
- **THEN** the default user exists and a write made by a plugin or a station RPC can be attributed to a user

### Requirement: One default user per show, not per station

The default user SHALL be identified by a value derived so that every station
computes the same one for the same show. Two stations opening the same show SHALL
converge on a single default user rather than one each.

The default user's identity SHALL NOT be derived from the station, the machine,
or the operating system account, so that one person working at a console and at a
tablet is not two users.

#### Scenario: Two stations open the same show

- **WHEN** two stations load the same show and each seed a default user
- **THEN** the show contains exactly one default user
- **AND** both stations agree on its id

#### Scenario: A station joins a session

- **WHEN** a station with its own loaded show joins a session led by another station and receives its state
- **THEN** the show contains exactly one default user

### Requirement: Seeding never overwrites an edited default

A station SHALL create the default user only when the show does not already
contain it. A station that finds it present SHALL write nothing.

Where a station's seed and another station's edit to the default user race, the
show SHALL converge on the edit: a seed SHALL NOT restore the default name or
colour over a value somebody chose.

#### Scenario: The default user has been renamed

- **WHEN** the default user has been renamed on one station and a second station loads the same show
- **THEN** the second station does not write the default name
- **AND** both stations show the chosen name

#### Scenario: A restart after a rename

- **WHEN** the default user has been renamed and the station is restarted
- **THEN** the chosen name is what comes back

### Requirement: A client works as the default until a person chooses otherwise

A client that has no remembered identity SHALL work as the show's default user
rather than as nobody. A client SHALL NOT be able to reach a state in which it is
identified as nobody: every write a client makes SHALL carry a user.

A client working as the default user because nothing was chosen SHALL make that
visible, along with the means to say who the operator actually is — two people at
two clients sharing one undo history is a consequence of the default and SHALL
NOT be silent.

#### Scenario: A browser that has never been used for this show

- **WHEN** a client with no remembered identity connects and the show's users have arrived
- **THEN** it is working as the default user
- **AND** it shows that it is, and offers a way to identify the operator

#### Scenario: The first change is undoable

- **WHEN** a client with no remembered identity makes a change and asks to take it back
- **THEN** the change is taken back

#### Scenario: A chosen identity is kept

- **WHEN** an operator identifies as a user other than the default and reconnects later on the same client
- **THEN** the client is still working as that user, not the default

#### Scenario: Leaving a chosen identity

- **WHEN** an operator on a client working as a chosen user asks to stop working as them
- **THEN** the client works as the default user
- **AND** the client is not left identified as nobody

### Requirement: The default user is an ordinary user

The default user SHALL be an ordinary row in the `users` collection: renameable,
recolourable, and deletable by the same means as a user somebody created. Its
name and colour SHALL be show data, so two stations working one show call it the
same thing.

A rename SHALL replicate to peers and survive a restart. Deleting it SHALL be
permitted; the next load of that show recreates it, since the show would
otherwise have no user.

#### Scenario: Renaming replicates

- **WHEN** the default user is renamed on one station of a two-station session
- **THEN** the other station shows the new name

#### Scenario: Deleting the default user

- **WHEN** the default user is deleted and the show is loaded again
- **THEN** a default user exists again

### Requirement: The engine's own writes stay unattributed

A write the console makes for itself — playback advancing, a station publishing
its own telemetry — SHALL NOT be attributed to the default user, and SHALL remain
outside undo and outside the history of what people did.

#### Scenario: Telemetry is not undoable

- **WHEN** a station publishes its own station row and an operator asks to take back their last change
- **THEN** the station row is not what is taken back

#### Scenario: A fade is not somebody's change

- **WHEN** a cue is fading and the engine writes the values it computes
- **THEN** those writes do not appear in the history of what people did
