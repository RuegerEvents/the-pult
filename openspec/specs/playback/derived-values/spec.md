# playback/derived-values Specification

## Purpose
What a parameter is doing right now is a question, not a stored fact. The console
keeps what is *driving* a parameter — the fades and effects anchored in time, the
programmer over them, the home value under them — and everything that needs a number
evaluates one, at the moment and the rate it actually needs. This says who evaluates,
from what, against which clock, and what stays state because it is sensed rather than
driven.

## Requirements

### Requirement: A driven value is evaluated, never stored

The console SHALL NOT keep a materialised copy of what a driven output parameter is
doing. What drives a parameter — the fades and effects running on it, the programmer
holding it, the home value beneath it — SHALL be the state, and any consumer needing
a value SHALL derive it from that state and a moment in time.

Storing the answer is what made a tick cost thirty-five milliseconds to compute
values worth a fifteenth of one. It also made the console assert something it cannot
know: that the value it stored is the value that is out there now, rather than the
value that was out there when it last got round to looking.

#### Scenario: A fade in progress

- **WHEN** a cue is fading and a consumer asks what a parameter is at a given moment
- **THEN** it is given the value that moment implies
- **AND** no stored sample of that parameter was consulted or written

#### Scenario: Nothing driving a parameter

- **WHEN** nothing is fading, running an effect, or holding a parameter in the programmer
- **THEN** evaluating it gives its home value
- **AND** the absence of anything driving it is not stored as a value either

#### Scenario: The same moment twice

- **WHEN** two consumers evaluate the same parameter for the same instant
- **THEN** they get the same value, whichever station or runtime they are in

### Requirement: One evaluator, whatever runtime asks

The maths that turns what is driving a parameter into a value SHALL exist as a single
implementation, used by the station, by its output connectors, by plugins, and by the
browser. A second implementation of it in another language SHALL NOT be introduced.

Easings, curves, step lists, spread, phase, direction, width, rates against speed
masters, priority between programmer and playback, home fallback, and split in and out
fades are a large surface. Two implementations of it would drift, and the visible form
of that drift is a screen disagreeing with the lamps — the one thing an operator
cannot be asked to work around.

#### Scenario: The browser and the station agree

- **WHEN** the browser and the station evaluate the same parameter for the same instant
- **THEN** they produce the same value

#### Scenario: A new curve or easing

- **WHEN** a curve, easing or effect shape is added
- **THEN** every consumer gains it without a second implementation being written
- **AND** no consumer needs updating for it separately

### Requirement: Everyone evaluating is talking about the same clock

A consumer evaluating a value SHALL do so against the console's clock rather than its
own. A browser SHALL determine its offset from the station it is connected to, keep
that offset current, and apply it when evaluating.

What is driving a parameter is anchored in console time. A consumer evaluating against
an unadjusted local clock runs every fade early or late by however wrong its clock is,
and does so silently — the values are all individually plausible.

#### Scenario: A browser whose clock is wrong

- **WHEN** a browser's own clock differs from the station's by a noticeable margin
- **THEN** the values it evaluates still match the station's for the same moment

#### Scenario: An offset that has not been established yet

- **WHEN** a browser has connected but has not yet determined its offset
- **THEN** it does not present values it cannot place in time
- **AND** it says so rather than showing a plausible wrong number

#### Scenario: A clock that steps

- **WHEN** a browser's clock is adjusted while it is connected
- **THEN** its offset is re-established
- **AND** what it shows converges without needing a reload

### Requirement: Each consumer evaluates at the rate it needs

A consumer SHALL choose its own evaluation rate. The console SHALL NOT impose a single
rate on every consumer, and SHALL NOT evaluate on behalf of a consumer that has not
asked.

A protocol sending whole frames, a protocol sending an object once, and a screen
drawing at its own refresh have three different needs, and the lowest common multiple
of them was the tick.

#### Scenario: A screen showing part of a large rig

- **WHEN** a browser is showing some fixtures of a much larger rig
- **THEN** it evaluates the ones it is showing
- **AND** the cost of what it is not showing is not paid

#### Scenario: A protocol that sends whole frames

- **WHEN** an output protocol needs a value for every patched fixture on every frame
- **THEN** it evaluates every patched fixture at its own frame rate
- **AND** it does so whether or not anything is on screen anywhere

#### Scenario: A protocol that can be told once

- **WHEN** an output protocol can carry what is driving a parameter rather than its values
- **THEN** it sends that once when it changes
- **AND** it does not send a stream of samples

#### Scenario: A settled show

- **WHEN** nothing is fading or running and nobody is looking
- **THEN** no repeated evaluation happens on the station's account

### Requirement: A sensed value is state and stays state

A value the console reads *from* a device — a contact, a temperature, a humidity, any
input a device reports — SHALL remain stored state. Only values the console drives are
derived.

An input is not a function of time and cannot be evaluated from anything the console
holds. The line is what keeps the model honest: the console computes what it is
saying, and remembers what it was told.

#### Scenario: A device reporting a reading

- **WHEN** a device reports an input value
- **THEN** it is stored, replicated and readable as before

#### Scenario: A flow watching an input

- **WHEN** a flow watches a value the console reads from a device
- **THEN** it behaves as it did before this change

### Requirement: Something that must notice a change still can

Where the console itself must react to a driven value changing — a flow watching a
parameter, or anything else needing an edge rather than a reading — it SHALL sample
that value, and SHALL sample only what something is actually watching.

Edge detection cannot be done from a function without evaluating it. Making that
sampling proportional to what is watched, rather than to the rig, is what stops it
becoming the tick again under another name.

#### Scenario: A flow watching one parameter of a large rig

- **WHEN** a flow watches a single parameter and the rig has thousands of fixtures
- **THEN** the sampling done on its behalf is proportional to what is watched, not to the rig

#### Scenario: Nothing watching anything

- **WHEN** no flow watches any driven parameter
- **THEN** no sampling happens on their behalf

### Requirement: Asking what a value is now remains possible everywhere it was

Every operation that needed to know what a parameter was doing SHALL still be able to
ask, and SHALL get the value for the moment it asks about. This includes storing what
is showing into a cue, taking a fixture's current output as its home value, a plugin
reading a value, and any panel displaying one.

#### Scenario: Storing what is showing

- **WHEN** an operator stores the current look into a cue mid-fade
- **THEN** the values stored are those of the moment the store happened

#### Scenario: Taking the current output as home

- **WHEN** an operator sets a fixture's home value from where it is now
- **THEN** the value taken is the one being output at that moment

#### Scenario: A plugin asking

- **WHEN** a plugin asks what a parameter is doing
- **THEN** it receives a value, without needing to know that nothing stored it
