# The log has an end

## Why

Nothing deletes an operation. `crates/pult-backend/src/infra/showfile/oplog.rs`
has `append`, `since`, `len` and `recent_by_people`, and there is no `DELETE` in
the backend at all. A showfile therefore grows for as long as it is ever used.

The number that should bound it already exists and bounds the wrong thing.
`history_depth` is show data, clamped, settable over REST, defaulting from the
station's `preferences.toml` — and it limits *reads*: the `History` command
clamps its limit to it, and `recent_by_people` passes it as a SQL `LIMIT`.
Everything past it stays on disk, invisible and unreachable, paid for and never
used. `HISTORY_DEPTH_MAX`'s own comment says as much: "Nothing prunes the log
yet, so this is the only thing keeping a long show's history from being read in
full."

Two facts make this a real cost rather than a tidiness complaint:

- **The bulk of the table is not what anybody did.** A station replaces its own
  row every two seconds (`REPORT_INTERVAL`, `infra/stations.rs:24`), as a SYNCED
  whole-row write, so it is logged. That is around 43,000 rows per station per
  day, each carrying a whole `Station` struct as `value_json`. A two-station
  fortnight is on the order of a million rows of telemetry nobody will ever read.
- **Catch-up reads the whole table into memory.** `oplog::since` issues
  `SELECT ... FROM oplog ORDER BY timestamp, seq` with no `WHERE` and filters in
  Rust (`oplog.rs:105`). Every reconnecting peer deserializes the entire log to
  find the handful of operations it missed. The log's size is not a disk problem
  that shows up eventually; it is a cost paid on every reconnection, growing all
  season.

## What Changes

- **Two retentions, because the table serves two masters.**
  - **Authored rows** — `user_id IS NOT NULL` — are kept to the show's
    `history_depth`. That is what the number already promises an operator about
    how far Ctrl-Z goes, and pruning to it makes the promise true in both
    directions.
  - **Unattributed rows** — the engine's own writes, which no one can undo and
    which never appear in the history panel — are kept only as long as sync could
    still want them, a much shorter window. They are the bulk of the table and the
    first thing to drop.

  One rule over both would break what `recent_by_people`'s filter exists for: at
  a row every two seconds, a 500-row window is a few minutes of edits rather than
  five hundred of them.

- **A prune floor, so a peer is never quietly short-changed.** The station
  records how far it has cut. `operations_since`
  (`crates/pult-backend/src/engine/mod.rs:1486`) already answers `None` to mean
  "send a snapshot instead", and already does so when replaying most of the log
  would cost more than the show. It gains one more reason: a requesting clock
  that is behind the floor gets a snapshot. Without this, a peer behind the cut
  receives the surviving rows, sees no error, and believes it is caught up —
  losing exactly the writes that were pruned.

- **Pruning is local and never replicated.** Each station cuts its own copy. A
  station that pruned cannot serve those rows and serves a snapshot instead,
  which is a path that already exists and is already exercised by any node
  joining with an empty clock. Making deletion a replicated operation would
  invent a new kind of write for the sync layer to carry, and would let one
  station's disk pressure delete everyone's history.

- **Pruning runs on open, and then every N appends while the show is up.** A
  fortnight of tech without a restart is the case that motivates this, so
  bounding only at open would miss it. The delete runs off the tick path: a slow
  `DELETE` must cost a frame of nothing rather than a frame of output.

- **The end of the log reads as an end.** `history_depth` is a promise about
  where Ctrl-Z stops, and once the rows past it are actually gone the History
  panel should show a boundary rather than an empty scroll that looks like a bug.

## Capabilities

### New Capabilities

- `history/retention`: how much of the operation log a station keeps, what the
  two retentions are and why they differ, and what a peer asking for something
  already pruned receives instead.

### Modified Capabilities

None. No existing spec describes the oplog's retention, because there was none.

## Non-goals

- **No archive.** Pruned rows are gone, not moved to a second table or a
  sidecar file. A history that is kept but unreachable is the state this change
  exists to end, and reintroducing it under another name would end nothing.
- **No rewriting `history_depth`'s meaning.** It stays a count of what people
  did, show data, clamped where it is used. This change makes it bound the log as
  well as the read, and changes nothing about who sets it or how.
- **No compaction or squashing.** Folding several old operations into one
  synthetic write would produce rows no station ever made, with clocks that mean
  nothing. Gesture folding (`fold_into_the_gesture`) already does the one form of
  this that is safe, at write time, inside a boundary somebody drew.
- **No VACUUM policy.** Reclaiming pages a delete freed is a showfile-management
  question — see `showfile-management` in the backlog, which owns save, backup and
  what a version is. This change stops the table growing; it does not promise the
  file on disk shrinks.
- **No undo across a prune.** Rows past the boundary are not undoable, which is
  what `history_depth` already promised and what the panel will now show.

## Impact

- **Lifecycle**: none of this is new state in the schema. The prune floor is a
  fact about *this station's own copy* of the log — it is not show data and must
  not replicate, since two stations legitimately hold different amounts of
  history. It belongs beside the log itself in the showfile's own tables, read
  and written by the station that owns the file, in the way `identity` and the
  oplog already are. Nothing gains a LOCAL/SYNCED/PERSISTED field.
- **Native, not a plugin.** Retention is the storage layer's own business, runs
  with no show open in the ordinary sense, and is on the path of every peer
  reconnection. A plugin cannot be in that path and should not be.
- `crates/pult-backend/src/infra/showfile/oplog.rs`: the first `DELETE` in the
  backend, the two retention queries, and the floor's storage and accessors.
  `since` keeps its shape — see `design.md` for why its missing `WHERE` is not
  the free win it looks like, and why bounding the table is the fix for it.
- `crates/pult-backend/src/engine/mod.rs`: `operations_since` consults the floor;
  the prune is triggered from the append path by count, off the tick.
- `crates/pult-backend/src/infra/showfile/migrations/`: a table or row for the
  floor, and an index supporting the retention queries — `recent_by_people`
  already wants one on `(user_id, timestamp)`.
- `crates/pult-schema/src/types/show.rs`: `HISTORY_DEPTH_MAX`'s comment stops
  being true and is rewritten.
- `frontend/src/lib/components/HistoryPanel.svelte`: the end of the log shown as
  a boundary.
- `crates/pult-backend/src/infra/stations.rs`: nothing changes, but its
  `prune_stale` is the prior art — the leader already prunes stale station rows,
  and this is the same idea applied to the log.
