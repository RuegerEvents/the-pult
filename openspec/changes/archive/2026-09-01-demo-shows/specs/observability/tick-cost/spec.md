## Purpose

A station knows what its own tick costs and says so, in the row where it already
says how much CPU and memory it is using. What running a show costs becomes a
number anyone can read off a live console, rather than something re-derived by
hand with instrumentation that is removed again afterwards.

## ADDED Requirements

### Requirement: A station measures the whole tick, not only the playback part

A station SHALL measure the elapsed time of its entire tick — computing playback
effects *and* applying them — and SHALL separately measure the part spent
computing them. Both figures SHALL be published.

One figure is not enough. Applying a tick's effects costs one write, one
broadcast and one output push per fixture that moved, and that is the larger and
the more scale-sensitive half; a single number would attribute all of it to
playback and point the next optimisation at the wrong place.

#### Scenario: A rig where applying costs more than computing

- **WHEN** a station ticks a rig large enough that applying effects dominates
- **THEN** both the whole-tick figure and the playback figure are published
- **AND** the difference between them is readable without adding instrumentation

#### Scenario: A settled rig where one fixture moves

- **WHEN** a tick computes effects but almost nothing moves
- **THEN** the two figures are close together
- **AND** the reader can tell that from the numbers alone

### Requirement: A station reports the worst tick as well as the ordinary one

A station SHALL publish both a representative figure for the ticks in a reporting
interval and the worst single tick observed in that interval.

The tick has a 25 ms budget and the question that matters is whether it is being
missed. An average over a couple of seconds of ticks answers a different question
and hides an overrun that happens a few times a second.

#### Scenario: An occasional overrun

- **WHEN** most ticks in a reporting interval are inside the budget and a few are not
- **THEN** the worst figure is over budget and the ordinary figure is not
- **AND** an operator can tell "occasionally late" from "always late"

#### Scenario: A station that is comfortably inside budget

- **WHEN** every tick in an interval is well inside the budget
- **THEN** both figures are inside it

### Requirement: A station that is not ticking says so rather than reporting zero

A settled show stops ticking. A station that performed no ticks in a reporting
interval SHALL publish its tick cost as absent, distinguishably from a measured
figure of zero, and SHALL NOT carry a previously measured figure forward as
though it were current.

Zero would read as "instant", which is the opposite of the truth: nothing was
measured at all.

#### Scenario: A show with nothing running

- **WHEN** no sequence is running and no effect is up, so a station performs no ticks in a reporting interval
- **THEN** its published tick cost is absent, not zero

#### Scenario: A show that stops running

- **WHEN** a station has been ticking and every sequence is then taken off
- **THEN** the next report carries no tick cost
- **AND** it does not repeat the last figure measured while the show was running

### Requirement: Each station reports only its own tick cost

Tick cost SHALL be published on the reporting station's own `stations` row and on
no other. It SHALL NOT be aggregated across the session, averaged between
stations, or written by any station other than the one that measured it.

Playback runs on every station, so two stations running one show are doing the
same work on different hardware under different load. Their figures differing is
information about the session, not a disagreement to be resolved.

#### Scenario: Two stations of different speed

- **WHEN** two stations run the same show and one is slower than the other
- **THEN** each publishes its own figures and neither overwrites the other's
- **AND** a console reading the session can see both

#### Scenario: A station goes quiet

- **WHEN** a station stops publishing its row
- **THEN** its last tick figures go stale together with the rest of that row
- **AND** they are not treated as a current description of the session

### Requirement: Measuring the tick does not change what the tick costs

The per-tick cost of measurement SHALL be bounded and SHALL NOT grow with the
size of the rig. Measurement SHALL NOT produce a replicated write per tick;
figures are carried by the station report the station already publishes on its
own interval.

#### Scenario: A rig of thousands of fixtures

- **WHEN** a station runs a rig large enough that the tick is over budget
- **THEN** the work done to measure the tick is the same as on a rig of five fixtures
- **AND** no additional replicated write occurs per tick

### Requirement: A station that reports no tick cost is still a valid station

A station SHALL accept a peer's row that carries no tick figures, treating them
as not reported rather than as zero or as an error, and SHALL keep the rest of
that row.

A session can mix builds, and a station that has nothing to say about its tick is
the same case as one running a build that cannot say it.

#### Scenario: A mixed-build session

- **WHEN** a station running a build that does not report tick cost joins the session
- **THEN** its row is accepted and its other figures are shown
- **AND** its tick cost reads as absent rather than as zero
