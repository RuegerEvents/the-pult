## Purpose

What a fixture's parameter rests at when nothing is driving it, and the acts that
put it back there — taking a sequence off, sending a selection home, and letting go
of a key nothing was underneath — so that a console can stop asserting a look
instead of only ever asserting a different one.

## ADDED Requirements

### Requirement: Every parameter has a home value

Every parameter of every patched fixture SHALL have a home value: what that
parameter rests at when nothing is driving it.

The home value SHALL be the fixture's own override for that parameter where one is
set, and what the fixture's type says the parameter rests at otherwise. A fixture
with no override SHALL have the same home value as every other fixture of its
type.

Every station in a session SHALL resolve the same home value for the same
parameter of the same fixture, and SHALL do so without asking another station.

#### Scenario: A fixture with nothing to say

- **WHEN** a fixture has no override for a parameter
- **THEN** its home value for that parameter is the one its type declares

#### Scenario: A house light that defaults to on

- **WHEN** a fixture overrides a parameter's home value
- **THEN** that fixture goes to the override wherever it goes home
- **AND** every other fixture of the same type is unaffected

#### Scenario: Two stations agree

- **WHEN** two stations in one session resolve the home value of one parameter
- **THEN** both resolve the same value

### Requirement: A fixture's home override is show data

An override of a parameter's home value SHALL be a property of the patched
fixture, SHALL be persisted in the showfile, and SHALL be replicated to every
station in the session.

It SHALL survive the fixture's type being described again by the device it came
from, and SHALL NOT be written to the fixture type.

An override SHALL be settable and clearable by an operator, and clearing it SHALL
return that parameter to what the type declares.

#### Scenario: An override travels with the show

- **WHEN** a show with an overridden home value is opened on another station
- **THEN** that station resolves the override, not the type's value

#### Scenario: The device describes itself again

- **WHEN** a device re-describes its ports and the console rebuilds the fixture type
- **THEN** the overrides set on fixtures of that type are unchanged

#### Scenario: Clearing an override

- **WHEN** an operator clears a fixture's override for a parameter
- **THEN** that parameter's home value is what the fixture's type declares

### Requirement: A sequence can be taken off

A sequence SHALL have an act that takes it off, leaving no cue of it active.

Taking a sequence off SHALL return to their home values every parameter that
sequence could drive — the parameters captured by any of its cues — except a
parameter that another sequence which is on could drive, and except a parameter
the programmer is holding.

Which parameters a sequence could drive SHALL be read from the show rather than
from what this station has watched happen, so that a station that joined the
session part way through takes the same parameters home as one that has been up
all evening.

Every station SHALL take the same parameters home, from the one act, without any
of them sending live values to the others.

#### Scenario: A sequence is taken off

- **WHEN** a sequence driving a fixture is taken off
- **THEN** that fixture's parameters go to their home values
- **AND** no cue of that sequence is active

#### Scenario: Another sequence still has it

- **WHEN** two sequences that are both on capture one fixture's intensity, and one of them is taken off
- **THEN** that intensity is left alone
- **AND** the parameters only the sequence taken off could drive go home

#### Scenario: The programmer has it

- **WHEN** a sequence is taken off while the programmer holds one of the parameters it could drive
- **THEN** the programmer goes on showing that parameter
- **AND** clearing the programmer afterwards leaves it at its home value

#### Scenario: A station that joined late

- **WHEN** a station joins a session in which a sequence has already run several cues, and that sequence is then taken off
- **THEN** that station takes the same parameters home as the stations that ran the cues

### Requirement: Running out of cues is not taking a sequence off

Going to the next cue when the last cue of a sequence is active SHALL leave that
cue active. It SHALL NOT leave the sequence with no active cue, and SHALL NOT
return anything to a home value.

A sequence SHALL have no active cue only after it has been taken off.

#### Scenario: Go at the last cue

- **WHEN** the last cue of a sequence is active and the operator goes to the next cue
- **THEN** the last cue is still active
- **AND** what the sequence is showing does not change

#### Scenario: A follow at the last cue

- **WHEN** the last cue of a sequence has a follow and it comes due
- **THEN** the sequence stays on that cue

### Requirement: An operator can send a selection home

An operator SHALL be able to send the selected fixtures home, putting every output
parameter of the selection at its home value.

Sending home SHALL be a programmer act: the values SHALL be held by the programmer,
SHALL override playback while they are held, SHALL be attributed and undoable like
any other programmer write, and SHALL be given back by clearing the programmer.

It SHALL be available on the command line as well as in the interface, and SHALL
require no access to the show beyond what setting a level requires — so that a
client that can ask for a level can ask for home without being able to read the
rig.

A parked value SHALL survive being sent home in the same way it survives being
cleared.

#### Scenario: Sending a selection home

- **WHEN** an operator sends a selection home
- **THEN** the programmer holds every output parameter of those fixtures at its home value
- **AND** what the rig is showing is those values

#### Scenario: Taking it back

- **WHEN** an operator sends a selection home and then clears the programmer
- **THEN** playback has those parameters back

#### Scenario: Undoing a home

- **WHEN** an operator sends a selection home and then undoes
- **THEN** the programmer holds what it held before

#### Scenario: From the command line

- **WHEN** an operator selects fixtures and asks for home on the command line
- **THEN** the same values are held as by the interface

#### Scenario: A parked value is not sent home

- **WHEN** a parked programmer value is one of the parameters a home would cover
- **THEN** it keeps the value it was parked at

### Requirement: A release with nothing underneath lands on home

Where the programmer lets go of a parameter that nothing was driving when it took
it, that parameter SHALL be left at its home value.

Where a fade begins on a parameter that has never been driven, it SHALL begin from
that parameter's home value.

Neither SHALL use a fixed zero in place of the home value.

#### Scenario: Clearing the programmer on an untouched fixture

- **WHEN** an operator takes a parameter of a fixture nothing has driven, moves it, and clears the programmer
- **THEN** that parameter is at its home value

#### Scenario: A cue fading a parameter for the first time

- **WHEN** a cue fades a parameter that has never been driven
- **THEN** the fade begins at that parameter's home value and arrives at the cue's

### Requirement: Going home can take time

A show SHALL carry how long a move to a home value takes, and every station in the
session SHALL use the show's value, so that stations driving one rig fade it home
together.

The default SHALL be to arrive immediately.

A station SHALL carry its own preference for what a newly created show starts that
value at. That preference SHALL NOT apply to a show that already carries one.

#### Scenario: Going home immediately

- **WHEN** a show carries no time for going home and a sequence is taken off
- **THEN** the parameters are at their home values on the next output frame

#### Scenario: Fading home

- **WHEN** a show carries a time for going home and a sequence is taken off
- **THEN** the parameters fade to their home values over that time

#### Scenario: Two stations fade together

- **WHEN** two stations with different preferences are in one session and a sequence is taken off
- **THEN** both fade home over the show's time

#### Scenario: A new show

- **WHEN** a station creates a show
- **THEN** that show starts with the time this station prefers
