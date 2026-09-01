## Purpose

Writes that say how much to change something rather than what to change it to —
"ten percent brighter" instead of "at 62%" — resolved by the station that holds
the authoritative value, so that two operators nudging one parameter both get
their nudge.

## ADDED Requirements

### Requirement: A relative write is resolved against what is showing

A relative write SHALL be turned into an absolute value by the station receiving
it, using the value currently in effect for that path.

For a parameter of a fixture, "currently in effect" SHALL mean what the priority
stack is showing: the programmer's value where the programmer holds that
parameter, and the value playback is producing where it does not. For any other
field, it SHALL mean the field's current value.

#### Scenario: Nudging a parameter the programmer already holds

- **WHEN** the programmer holds a parameter at 0.5 and a relative write of +0.1 arrives
- **THEN** the programmer holds it at 0.6

#### Scenario: Nudging a parameter the programmer does not hold

- **WHEN** playback is showing a parameter at 0.4, the programmer holds nothing for it, and a relative write of +0.1 arrives
- **THEN** the programmer takes the parameter and holds it at 0.5
- **AND** it is held, so it overrides playback from then on

#### Scenario: Nudging an ordinary field

- **WHEN** a cue's fade time is 3 and a relative write of 1.5 arrives for it
- **THEN** the cue's fade time is 4.5

#### Scenario: Two nudges do not overwrite each other

- **WHEN** two relative writes of +0.1 arrive for the same parameter from two operators
- **THEN** the parameter ends 0.2 higher than it began
- **AND** neither nudge is lost

### Requirement: Nothing downstream of resolution sees a relative write

A relative write SHALL be resolved before it is recorded, broadcast, persisted or
replicated. The operation log, the showfile, connected frontends and peer
stations SHALL only ever see the resolved absolute value.

A peer station SHALL NOT resolve a relative write for itself.

#### Scenario: A peer receives an absolute value

- **WHEN** an operator on one station makes a relative write
- **THEN** every peer station receives the resolved absolute value
- **AND** every station holds the same value afterwards, whatever each was showing before

#### Scenario: The history shows what it became

- **WHEN** a user makes a relative write
- **THEN** the history of what people did records it as a change to the value it became

#### Scenario: Undo restores what was there

- **WHEN** a user makes a relative write and then undoes
- **THEN** the value is what it was before the write
- **AND** undoing again does not apply the delta a second time

### Requirement: A relative write only applies to values arithmetic means something for

A relative write SHALL be accepted for numeric and colour values, applying the
delta to each colour channel. It SHALL be refused, with a message naming the
reason, for values where addition has no meaning.

A relative write SHALL be refused where the programmer holds the parameter as a
running shape rather than as a value.

The result SHALL be held within the range the parameter accepts, so that a nudge
past the end of a range comes to rest at the end of it rather than beyond.

#### Scenario: A relative write on a switch

- **WHEN** a relative write arrives for a parameter whose value is on or off
- **THEN** it fails with a message saying so
- **AND** the value is unchanged

#### Scenario: A relative write on a held shape

- **WHEN** the programmer holds a parameter as a running effect and a relative write arrives for it
- **THEN** it fails with a message saying so
- **AND** the effect goes on running unchanged

#### Scenario: Nudging past the top

- **WHEN** a parameter is at its maximum and a relative write would take it past
- **THEN** it comes to rest at the maximum

#### Scenario: Nudging a colour

- **WHEN** a relative write arrives for a colour
- **THEN** each channel moves by the delta, each held within its range

### Requirement: A relative write is an ordinary attributed change

A relative write SHALL be attributed to the user who made it and SHALL be
undoable on the same terms as the equivalent absolute write. It SHALL be
available to a plugin and over the WebSocket API by the same means as an absolute
write, without a separate call.

#### Scenario: A plugin makes a relative write

- **WHEN** a plugin writes relatively to a path it is permitted to write
- **THEN** the value changes by the delta
- **AND** the plugin needed no permission it did not already have for an absolute write

#### Scenario: A relative write to a path that is not there

- **WHEN** a relative write names a field that does not exist
- **THEN** it fails with a message naming the path
- **AND** nothing is written

### Requirement: The command line can say how much to change by

The command line SHALL accept a level given as a change rather than as a
destination, on the current selection and on a selection made in the same line.

A signed level SHALL be distinguishable from an absolute one, so that `at 10` and
`at +10` do not mean the same thing.

#### Scenario: Nudging the selection

- **WHEN** an operator selects fixtures showing 50% and asks for a level of +10
- **THEN** those fixtures are held at 60%

#### Scenario: Selecting and nudging in one line

- **WHEN** an operator selects fixtures and gives a signed level in the same line
- **THEN** the fixtures just selected are the ones nudged

#### Scenario: An absolute level still means a destination

- **WHEN** an operator asks for a level of 10
- **THEN** the fixtures are held at 10%, not 10% higher than they were
