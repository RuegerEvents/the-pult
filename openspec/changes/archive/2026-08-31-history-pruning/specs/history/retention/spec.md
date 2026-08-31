## Purpose

How much of the operation log a station keeps, so a showfile stops growing for
as long as it is ever used, and what a peer asking for something already pruned
is given instead of a silently incomplete answer.

## ADDED Requirements

### Requirement: Operations a person performed are kept to the show's history depth

A station SHALL retain at least the most recent `history_depth` operations
attributed to a user, and SHALL be permitted to delete authored operations older
than that. `history_depth` is the show's, clamped where it is used, as it already
is for reads.

The operations reachable for undo and shown in the history of what people did
SHALL NOT be reduced below what `history_depth` promises by any pruning this
capability performs.

#### Scenario: Undo still reaches back as far as it promised

- **WHEN** more than `history_depth` authored operations have been written and pruning has run
- **THEN** the most recent `history_depth` authored operations are still present
- **AND** each of them is still undoable

#### Scenario: Older authored operations go

- **WHEN** many more than `history_depth` authored operations have been written and pruning has run
- **THEN** the log no longer holds the oldest of them

#### Scenario: A show that raises its history depth

- **WHEN** a show's `history_depth` is raised
- **THEN** operations already pruned do not come back
- **AND** operations written from then on are retained to the new depth

### Requirement: Unattributed operations are kept only as long as replication needs them

Operations carrying no user — the console's own writes, which no person can undo
and which never appear in the history of what people did — SHALL be retained on a
shorter basis than authored ones, bounded by what peer catch-up may still need
rather than by `history_depth`.

Pruning them SHALL NOT remove any authored operation, and SHALL NOT change what
the show's state is on any station.

#### Scenario: Telemetry does not accumulate without bound

- **WHEN** a station runs for long enough to write many unattributed operations and pruning has run
- **THEN** the log holds far fewer of them than were written
- **AND** every authored operation within `history_depth` is still present

#### Scenario: Pruning does not change the show

- **WHEN** pruning removes unattributed operations
- **THEN** the state each station holds for every path is unchanged

### Requirement: A peer that is behind the prune floor receives a snapshot

A station SHALL record how far it has pruned. When a peer asks for the operations
it has missed and any of them have been pruned, the station SHALL respond with a
full state snapshot rather than with the operations that happen to survive.

A peer SHALL NOT be given a partial catch-up that omits pruned operations and
reports success. A station rejoining a session after longer than the retention
SHALL converge on the same state as a station that has never seen the show.

#### Scenario: A peer away longer than the retention

- **WHEN** a peer that has been disconnected past the prune floor reconnects and asks for what it missed
- **THEN** it receives a full state snapshot
- **AND** after applying it, its state matches the station that served it

#### Scenario: A peer within the retention

- **WHEN** a peer reconnects having missed only operations that were not pruned
- **THEN** it receives those operations rather than a snapshot

#### Scenario: A write made while a peer was away is not lost

- **WHEN** a value is changed while a peer is disconnected, that change is pruned, and the peer reconnects
- **THEN** the peer holds the changed value

### Requirement: Pruning is local to a station

A station SHALL prune only its own copy of the log. Pruning SHALL NOT be
replicated to peers, and one station's pruning SHALL NOT delete operations from
another's log.

Two stations in one session MAY hold different amounts of history, and each SHALL
serve catch-up from what it holds, falling back to a snapshot per the requirement
above.

#### Scenario: One station prunes, the other does not

- **WHEN** one station of a two-station session prunes and the other does not
- **THEN** the other station's log is unchanged
- **AND** both stations still agree on the show's state

#### Scenario: Catch-up from either station

- **WHEN** a peer catches up from a station that has pruned past its clock, and then from one that has not
- **THEN** it reaches the same state either way

### Requirement: Pruning does not interrupt the show

Pruning SHALL run while a show is open, so that a station left running for weeks
is bounded and not only one that is restarted. It SHALL also run when a showfile
is opened.

Pruning SHALL NOT delay output: a station SHALL continue to emit at its normal
rate while pruning, and SHALL NOT drop or late-run a playback tick because a
deletion is in progress.

#### Scenario: A long-running show is bounded

- **WHEN** a station runs long enough to pass the retention without being restarted
- **THEN** the log is bounded without a restart

#### Scenario: Output is unaffected

- **WHEN** pruning runs while cues are playing back
- **THEN** output continues at its normal rate

#### Scenario: Opening an oversized showfile

- **WHEN** a showfile whose log is far past the retention is opened
- **THEN** the log is brought within the retention

### Requirement: The end of the history is shown as an end

Where the history of what people did reaches the oldest operation retained, the
console SHALL indicate that this is the limit of what is kept rather than
presenting it as though nothing further was ever done.

#### Scenario: Scrolling to the boundary

- **WHEN** an operator scrolls the history to its oldest retained entry on a show that has been pruned
- **THEN** the console shows that this is as far back as the show keeps
