## MODIFIED Requirements

### Requirement: A station measures the whole tick, not only the playback part

A station SHALL measure what producing output costs it, as the elapsed time of one
whole output frame — gathering what a frame needs, evaluating it, and emitting it —
and SHALL separately measure the part spent evaluating. Both figures SHALL be
published.

The thing with a deadline is the output frame; there is no longer an engine tick
behind it, and nothing is applied to state on the way. One figure is still not
enough, and for the same reason it was not before: the two halves scale differently,
and a single number lets the larger one hide behind the name of the smaller. That is
not a hypothetical — a two-figure split showed that computing was 0.2% of a tick, and
finding what the other 99.8% actually was still needed a counter added by hand and
taken away again.

Where a station produces output through more than one connector, each SHALL be
measured on its own account, because their rates and their costs are their own.

#### Scenario: A rig where applying costs more than computing

- **WHEN** a station produces frames for a rig large enough that gathering and emitting them dominates the evaluating
- **THEN** both the whole-frame figure and the evaluating figure are published
- **AND** the difference between them is readable without adding instrumentation

#### Scenario: A settled rig where one fixture moves

- **WHEN** a frame evaluates a rig in which almost nothing is moving
- **THEN** the two figures are close together
- **AND** the reader can tell that from the numbers alone

#### Scenario: Two connectors at different rates

- **WHEN** a station runs two output connectors whose frame rates differ
- **THEN** each connector's cost is reported separately
- **AND** neither is presented as the station's single figure

### Requirement: A station that is not ticking says so rather than reporting zero

A station that produced no output frames in a reporting interval SHALL publish its
frame cost as absent, distinguishably from a measured figure of zero, and SHALL NOT
carry a previously measured figure forward as though it were current.

Zero would read as "instant", which is the opposite of the truth: nothing was
measured at all. A station with no outputs configured, and one whose protocols are
all idle and sending nothing, are both this case.

#### Scenario: A show with nothing running

- **WHEN** no sequence is running and no effect is up
- **THEN** a connector that still refreshes its protocol on an idle rig reports what those frames cost, however small
- **AND** a connector that emitted nothing at all in the interval reports absent, not zero

#### Scenario: A show that stops running

- **WHEN** a station has been emitting frames and everything is then taken off
- **THEN** any connector that stops emitting carries no figure on the next report
- **AND** it does not repeat the last figure measured while the show was running

#### Scenario: A station with nothing to send

- **WHEN** a station has no output configured at all
- **THEN** its published frame cost is absent, not zero

### Requirement: Measuring the tick does not change what the tick costs

The per-frame cost of measurement SHALL be bounded and SHALL NOT grow with the size
of the rig. Measurement SHALL NOT produce a replicated write per frame; figures are
carried by the station report the station already publishes on its own interval.

#### Scenario: A rig of thousands of fixtures

- **WHEN** a station produces frames for a rig large enough that a frame is over its budget
- **THEN** the work done to measure the frame is the same as on a rig of five fixtures
- **AND** no additional replicated write occurs per frame
