## Context

See `proposal.md` — Why. What the approach has to fit:

- `oplog::since(pool, known)` (`oplog.rs:105`) selects **every** row and filters
  in Rust: an operation is missing when `known.0.get(&op.node_id) < op.seq`. The
  comparison is per originating node, against a vector clock.
- `operations_since` (`engine/mod.rs:1486`) already has two reasons to answer
  `None`, meaning "send a whole snapshot instead": an empty clock, and
  `missing * 2 > total`. The snapshot path is therefore well-worn, not a new
  branch to be trusted for the first time.
- `recent_by_people` (`oplog.rs:183`) filters `WHERE user_id IS NOT NULL`, and
  its doc comment explains why that is load-bearing: telemetry at a row every two
  seconds would otherwise make a 500-row window about a quarter of an hour.
- `append` folds a write into its gesture's existing row where it can
  (`fold_into_the_gesture`), moving the row's `seq` forward. So `seq` order and
  insertion order are not the same thing, and a row's age is its `timestamp`.
- The engine is a single actor. Anything awaited inside its loop is time the tick
  is not running.
- `infra/stations.rs:169` has `prune_stale`, where the leader deletes stale
  station rows. Deleting things on a schedule is not a new idea here; deleting
  from the oplog is.

## Goals / Non-Goals

**Goals:**

- A showfile that stops growing, whether the station is restarted nightly or left
  up for a fortnight.
- No peer ever told it is caught up when it is not.
- No new replicated state, no new sync message, no change to what an operation is.

**Non-Goals:**

- Anything the proposal's Non-goals section rules out.
- Making `since` fast. Bounding the table is what makes it fast enough; see the
  decision below on why its `WHERE` is not the easy win it appears to be.
- Deciding when a showfile is compacted on disk. That is `showfile-management`.

## Decisions

### The floor is a seq per originating node, not one number

Catch-up compares per node, so the floor has to as well: a row is
`(node_id, seq)`, and "everything up to here is gone" is only meaningful about
one node's sequence. The floor is a small table of `(node_id, pruned_through_seq)`
in the showfile, one row per node whose operations have ever been pruned.

`operations_since` gains its third reason to answer `None`: for any
`(node, floor_seq)` in that table, if `known.0.get(node).unwrap_or(0) < floor_seq`,
the asking peer is behind the cut and gets a snapshot. A node absent from the
peer's clock reads as 0 and so is behind any floor above zero, which is correct —
it has seen nothing from that node.

*Alternative rejected:* one global "pruned before this timestamp" marker. Cheaper
to store and wrong to compare against, since a peer's position is a vector clock
and not a time. It would either be conservative to the point of always sending
snapshots, or compare two things that are not the same kind of thing.

### The floor is written before the rows are deleted

In one transaction if the same transaction can carry both; and where they must be
separate, the floor goes first. The two failure directions are not symmetric. A
floor recorded for rows that were not deleted costs unnecessary snapshots, which
is a performance loss on a path that is already correct. Rows deleted without a
floor recorded is exactly the silent partial catch-up this design exists to
prevent.

### Two retentions, in two homes

- **Authored rows** are bounded by the show's `history_depth`, because that is
  already the show's promise about how far Ctrl-Z goes and two consoles must
  agree about it. The prune keeps the newest `history_depth` rows with
  `user_id IS NOT NULL` and deletes the rest.
- **Unattributed rows** are bounded by a *station preference*, a duration, in
  `preferences.toml` beside `history_depth`'s own default. Per-station rather
  than per-show because the decision it encodes is "how long an absence should
  this machine be able to answer without sending a snapshot", which is about this
  machine's disk and this rig's network, not about the show. Pruning is already
  local and unreplicated, so a preference is the consistent home.

An hour is the proposed default: long enough that a peer dropping off a switch
and coming back does not trigger a snapshot, short enough that an hour of two
stations' telemetry is a few thousand rows rather than a season's worth.

The two retentions are applied as two deletes, and the authored one is expressed
by count while the unattributed one is expressed by age. That asymmetry is not an
inconsistency: `history_depth` counts changes because an operator counts changes,
and sync retention is a duration because an absence is a duration.

### `since` keeps its shape, and the retention is what fixes it

Reading the whole table to filter in Rust looks like an obvious `WHERE` waiting
to be written, and it is not, because the predicate is a vector clock: the query
would be `(node_id = ?1 AND seq > ?2) OR (node_id = ?3 AND seq > ?4) OR ...`
built per request from the asking peer's clock, plus every row from a node the
clock has never heard of. A query whose shape depends on the number of peers, to
replace a filter that is only expensive because the table is unbounded, is
solving the symptom of the thing being fixed in the same change.

Once the table is bounded by the retention above, the full read is bounded too.
If it is still hot afterwards, it can be measured and dealt with then, with the
numbers in hand — task 29's precedent for doing it in that order.

### Pruning runs off the engine's actor loop

The trigger is a counter on the append path: every N appends, and once when the
showfile is opened. But the work is `tokio::spawn`ed against the pool rather than
awaited in the loop, because the engine is one actor and a `DELETE` over a
million rows inside it is a stalled tick.

A flag guards against a second prune starting while one is running — at a
threshold of a thousand appends and a prune that takes seconds, an overlap is
reachable, and two concurrent deletes racing on the floor is the one way to get
the ordering above wrong.

The counter is in memory, so a station restarted often prunes at open and rarely
otherwise, which is the correct amount for a station that is restarted often.

*Alternative rejected:* a timer. It wakes to do nothing on an idle show, and the
thing that should drive the work is how much has been written, which is what the
counter measures directly.

### The panel shows a boundary, not an absence

The History panel currently ends where the query's `LIMIT` ends, which is
indistinguishable from the show having no more history. Once rows are genuinely
gone, that ambiguity becomes a lie in one of the two directions. The panel gets a
terminal row saying this is as far back as the show keeps — read from whether the
oldest returned row is at the retention rather than from a new API, so nothing
new has to be plumbed to say it.

## Risks / Trade-offs

- **The first `DELETE` in the backend, on the table sync reads from.** → The floor
  and its ordering are the mitigation, and the fallback it triggers is the
  snapshot path that already runs for every node joining with an empty clock. The
  spec's requirement that a write made while a peer was away survives its own
  pruning is the test that this holds.

- **A snapshot is much more expensive than a catch-up, and pruning causes more of
  them.** → Bounded by the unattributed retention, which is the knob for exactly
  this: a station whose peers drop out for longer than an hour raises it. The
  alternative — pruning only to what every peer has acknowledged — lets one
  station that went home for the weekend pin the log, which is the unbounded
  growth being fixed.

- **An hour of unattributed retention is a guess.** → It is a station preference
  precisely because it is a guess, and a wrong one costs snapshots rather than
  correctness. Worth revisiting with a real rig's reconnection behaviour.

- **Deleting does not shrink the file.** SQLite reuses freed pages; the file stays
  its high-water mark until vacuumed. → Named as a non-goal and pointed at
  `showfile-management`. The costs this change is actually paying down — the full
  read on every catch-up, the snapshot a joining station swallows, the size of a
  backup's meaningful content — are all about rows rather than bytes on disk.

- **A prune racing an append.** A row written between the floor being recorded and
  the delete running is newer than the floor and outside the retention window, so
  it is not deleted; a row deleted while a peer is mid-catch-up was already
  outside the retention when `since` read it. → Both are covered by the floor
  being conservative rather than exact. The guard flag stops two prunes doing this
  to each other.

## Impact on pult-schema, the WIT contract and the sync protocol

- **pult-schema**: no new types and no new fields. `HISTORY_DEPTH_MAX`'s comment
  ("Nothing prunes the log yet, so this is the only thing keeping a long show's
  history from being read in full") stops being true and is rewritten. That is
  the whole of the schema change.
- **WIT contract**: none. A plugin's writes are ordinary operations and are
  retained by the same two rules — an attributed store write by `history_depth`,
  an unattributed one by the sync window. Nothing about retention is visible
  across the boundary.
- **Sync protocol**: no new message and no changed message. The only change is
  one more condition under which an existing decision — catch-up or snapshot —
  comes out as snapshot. That is deliberately the smallest possible surface: the
  protocol already had to handle a peer being sent a snapshot instead of what it
  asked for, and this makes that case more common rather than new.

## Migration Plan

An existing showfile is pruned on its first open by a station with this change,
which for a long-running show is the largest delete it will ever do — the tasks
call for that case to be measured rather than assumed, since it happens at
startup where a slow query is most visible.

No floor exists in such a file before the first prune, which reads as "nothing
pruned" and is correct. A station running an older build against a pruned
showfile serves catch-up without consulting a floor it does not know about, and
can therefore short-change a peer — so this is a change to have on every station
in a session, and the roadmap entry should say so.
