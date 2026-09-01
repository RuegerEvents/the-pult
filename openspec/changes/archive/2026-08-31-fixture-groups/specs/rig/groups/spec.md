## Purpose

Saved fixture groups: a name attached to a question about the rig, kept in the
show so that every station, every operator and every plugin can say "the
downstage movers" and get the same answer — including after somebody patches a
new one.

## ADDED Requirements

### Requirement: A group stores a query, not a list of fixtures

A group SHALL store the selection query that picks its fixtures, and SHALL NOT
store the fixture ids that query currently resolves to. Resolving a group SHALL
read the rig as it is at that moment.

A group SHALL have a name, chosen by whoever saved it.

#### Scenario: A fixture patched after the group was saved is in it

- **WHEN** a group is saved from a query that matches every fixture of a type
- **AND** a new fixture of that type is patched afterwards
- **THEN** resolving the group includes the new fixture

#### Scenario: A fixture deleted after the group was saved leaves it

- **WHEN** a fixture that a group resolved to is deleted
- **THEN** resolving the group no longer includes it
- **AND** the group is still a group, with the same query and name

#### Scenario: A group that currently matches nothing is kept

- **WHEN** every fixture a group's query matches has been deleted
- **THEN** the group still exists, and resolving it answers with no fixtures
- **AND** it is not removed by the console on its own account

### Requirement: Groups are show data and reach every station

Groups SHALL be PERSISTED: written to the showfile and replicated to every peer
station in the session. A group saved on one station SHALL become available on
every other station in the session without any further action, and SHALL still
be there when the show is next opened.

#### Scenario: A group saved on one station appears on another

- **WHEN** an operator on one station saves a group
- **THEN** the group is present on every other station in the session
- **AND** resolving it on any of them names the same fixtures, in the same order

#### Scenario: A group survives the showfile being reopened

- **WHEN** a show containing groups is closed and reopened
- **THEN** every group is present with its name and its query

#### Scenario: A station that joins late receives the groups

- **WHEN** a station joins a session in which groups already exist
- **THEN** it holds those groups once it has caught up

### Requirement: A query has one meaning everywhere

A selection query SHALL resolve to the same ordered list of fixture ids
regardless of who resolves it — a frontend, a station, or a plugin — given the
same rig.

Where a query is ordered by a property a fixture does not have, resolution SHALL
still be total and deterministic: every matched fixture appears exactly once, and
two resolutions of the same query against the same rig SHALL produce the same
order.

#### Scenario: Frontend and station agree

- **WHEN** the same query is resolved by a frontend and by a station against the same rig
- **THEN** both answer with the same fixture ids in the same order

#### Scenario: Two stations agree

- **WHEN** a group is resolved on two stations that have both caught up
- **THEN** both answer with the same fixture ids in the same order

#### Scenario: Fixtures with no position in a geometric order

- **WHEN** a query ordered by position is resolved against a rig containing fixtures that have never been placed
- **THEN** every matched fixture appears exactly once
- **AND** the order is the same on every resolver

### Requirement: A group can be resolved by anything that can reach a station

A station SHALL offer a way to resolve a group to its ordered fixture ids that is
available over the WebSocket API and to plugins, and that is discoverable through
introspection rather than by being written down in a client.

Asking to resolve a group that does not exist SHALL answer with an error naming
the problem, not with an empty result.

#### Scenario: A plugin resolves a group

- **WHEN** a plugin asks a station to resolve a group by id
- **THEN** it receives the group's fixture ids, in the group's order

#### Scenario: A plugin discovers that resolution exists

- **WHEN** a plugin lists what a station can do through introspection
- **THEN** group resolution is among the calls listed, with its arguments

#### Scenario: Resolving a group that is not there

- **WHEN** a caller asks to resolve a group id that no group has
- **THEN** the call fails with a message saying so
- **AND** the caller can tell that apart from a group that matched no fixtures

### Requirement: Recalling a group leaves a live selection

Recalling a group into the selection SHALL make the selection that group's query,
so that the selection continues to answer the question as the rig changes. It
SHALL NOT freeze the group's current fixtures into a list.

Refining a recalled selection SHALL NOT alter the group. A group changes only
when somebody saves over it.

#### Scenario: The recalled selection follows the rig

- **WHEN** an operator recalls a group and a new matching fixture is then patched
- **THEN** the new fixture is in the operator's selection

#### Scenario: Refining does not write back

- **WHEN** an operator recalls a group and then removes a fixture from the selection
- **THEN** the group is unchanged
- **AND** another station's view of the group is unchanged

#### Scenario: Two operators recall the same group

- **WHEN** two operators on two stations recall the same group
- **AND** one of them then changes their selection
- **THEN** the other's selection is unaffected

### Requirement: Editing a group is an ordinary show edit

Creating, renaming, re-saving and deleting a group SHALL be attributed to the
user who did it, SHALL appear in the history of what people did, and SHALL be
undoable on the same terms as any other change to the show.

#### Scenario: Undoing a group deletion

- **WHEN** a user deletes a group and then undoes
- **THEN** the group is back, with its name and its query
- **AND** it is back on every station in the session

#### Scenario: A group edit is attributed

- **WHEN** a user renames a group
- **THEN** the history of what people did shows that user's rename

### Requirement: Groups can be addressed by name from the command line

The command line SHALL be able to select a group's fixtures, and SHALL accept a
group by its position in the collection and by its name.

Selecting a group from the command line SHALL leave the same live selection that
recalling it in the frontend does, so the two paths do not disagree about what is
selected.

#### Scenario: Selecting a group

- **WHEN** an operator types a command selecting a group
- **THEN** the fixtures the group resolves to are selected, in the group's order

#### Scenario: Selecting a group and setting a level in one line

- **WHEN** an operator selects a group and gives an intensity in the same line
- **THEN** the group's fixtures are selected and held at that level in the programmer

#### Scenario: Naming a group that does not exist

- **WHEN** an operator names a group that is not there
- **THEN** the command line says so and changes neither the selection nor the programmer
