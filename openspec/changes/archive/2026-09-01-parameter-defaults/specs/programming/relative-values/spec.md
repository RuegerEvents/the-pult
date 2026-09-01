## MODIFIED Requirements

### Requirement: A relative write is resolved against what is showing

A relative write SHALL be turned into an absolute value by the station receiving
it, using the value currently in effect for that path.

For a parameter of a fixture, "currently in effect" SHALL mean what the priority
stack is showing: the programmer's value where the programmer holds that
parameter, the value playback is producing where it does not, and the parameter's
home value where nothing has ever driven it. For any other field, it SHALL mean
the field's current value.

#### Scenario: Nudging a parameter the programmer already holds

- **WHEN** the programmer holds a parameter at 0.5 and a relative write of +0.1 arrives
- **THEN** the programmer holds it at 0.6

#### Scenario: Nudging a parameter the programmer does not hold

- **WHEN** playback is showing a parameter at 0.4, the programmer holds nothing for it, and a relative write of +0.1 arrives
- **THEN** the programmer takes the parameter and holds it at 0.5
- **AND** it is held, so it overrides playback from then on

#### Scenario: Nudging a parameter nothing has ever driven

- **WHEN** a parameter has never been driven, its home value is 0.4, and a relative write of +0.1 arrives
- **THEN** the programmer takes the parameter and holds it at 0.5

#### Scenario: Nudging an ordinary field

- **WHEN** a cue's fade time is 3 and a relative write of 1.5 arrives for it
- **THEN** the cue's fade time is 4.5

#### Scenario: Two nudges do not overwrite each other

- **WHEN** two relative writes of +0.1 arrive for the same parameter from two operators
- **THEN** the parameter ends 0.2 higher than it began
- **AND** neither nudge is lost
