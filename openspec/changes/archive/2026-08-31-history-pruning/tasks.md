# The log has an end — tasks

Each group is meant to end in its own commit with `cargo test`, `npm run check`
and a zero-warning build passing.

**Do group 2 before group 3.** The floor is what makes deleting safe, and a
station that prunes before it can tell a peer it pruned is a station that loses
other people's writes quietly. Building the guard first means the dangerous state
never exists in the tree.

## 1. Knowing how far back the log goes

- [x] 1.1 Add a migration for the prune floor: `(node_id, pruned_through_seq)`,
      one row per node whose operations have been pruned, in the showfile beside
      the oplog. Verify the migration applies to a fresh showfile and to one
      written before this change.
- [x] 1.2 Add an index supporting the retention queries — `recent_by_people`
      already wants `(user_id, timestamp)`. Verify with `EXPLAIN QUERY PLAN` in a
      test, or by asserting the query does not scan, that both the history read
      and the authored-retention delete use it.
- [x] 1.3 Add `floor(pool)` and `raise_floor(pool, node, seq)` to `oplog.rs`.
      Verify with tests that an unpruned showfile reports an empty floor, that
      raising it is idempotent, and that a floor never goes down — a lower value
      arriving is ignored rather than written.

## 2. Nobody is told they are caught up when they are not

- [x] 2.1 Teach `operations_since` (`engine/mod.rs`) its third reason to answer
      `None`: any `(node, floor_seq)` where the asking clock's entry for that
      node — 0 when absent — is below `floor_seq`. Verify with unit tests over a
      constructed floor and clock, including the absent-node case.
- [x] 2.2 Verify with a two-station test that a peer behind the floor receives a
      snapshot rather than the surviving rows, and that after applying it its
      state matches the station that served it.
- [x] 2.3 Verify with a two-station test that a peer *within* the retention still
      receives operations rather than a snapshot — the guard must not collapse
      into "always snapshot", which would pass 2.2 and defeat catch-up entirely.
- [x] 2.4 Verify with a test that a value changed while a peer was disconnected,
      whose operation is then pruned, is still what the peer holds after
      reconnecting. This is the property the whole change risks and the one worth
      failing loudly.

## 3. Cutting

- [x] 3.1 Add the authored retention to `oplog.rs`: delete rows with
      `user_id IS NOT NULL` beyond the newest `history_depth`, clamped where it is
      used as it already is. Verify with tests that exactly the newest
      `history_depth` survive and that each is still undoable.
- [x] 3.2 Add the unattributed retention: delete rows with `user_id IS NULL`
      older than the station's configured window. Verify with tests that authored
      rows are untouched however old they are, and that the show's state on every
      path is unchanged by the delete.
- [x] 3.3 Write the floor before deleting, in one transaction where the pool
      allows it. Verify with a test that a prune interrupted between the two
      leaves a floor at least as high as what was deleted — over-reporting is
      safe, under-reporting is the failure mode.
- [x] 3.4 Add `oplog_retention` (a duration) to `preferences.toml` beside
      `history_depth`, defaulting to an hour, clamped like the others. Verify
      with tests in `infra/preferences/tests.rs` that a missing, malformed and
      out-of-range value each land on something sensible, as the existing
      preference tests do.
- [x] 3.5 Verify with a test that a plugin's store write is retained by the same
      two rules — attributed by `history_depth`, unattributed by the window —
      with nothing in the retention code knowing what a plugin is.

## 4. When it runs

- [x] 4.1 Prune once when the showfile is opened. Verify with a test that a
      showfile whose log is far past both retentions is brought within them by
      opening it.
- [x] 4.2 Trigger a prune every N appends, spawned against the pool rather than
      awaited in the engine's loop, with a flag preventing a second starting
      while one runs. Verify with a test that a long run of appends prunes
      without the engine's loop blocking, and that concurrent triggers produce
      one prune.
- [x] 4.3 Verify that output is unaffected: with cues playing back, force a prune
      over a large log and assert the tick keeps its rate. Task 29's measurements
      are what to compare against.
- [x] 4.4 Measure the first-open prune of a large existing showfile — the
      migration case, and the slowest this will ever be. Record the number in the
      roadmap entry rather than asserting a threshold in a test.

## 5. Saying where the end is

- [x] 5.1 Show the end of the retained history as a boundary in
      `HistoryPanel.svelte`, derived from whether the oldest returned row is at
      the retention rather than from a new API. Verify by pruning a demo show and
      scrolling to the bottom of the panel.
- [x] 5.2 Rewrite `HISTORY_DEPTH_MAX`'s comment in
      `crates/pult-schema/src/types/show.rs` — "Nothing prunes the log yet" stops
      being true with this change and is the sort of comment that outlives its
      subject. Verify by reading it.

## 6. Writing it down

- [x] 6.1 Add the roadmap entry: the two retentions and why they are counted
      differently, the floor as a per-node seq rather than a timestamp, why
      `since` was left alone, and the first-open measurement from 4.4.
- [x] 6.2 Record in the roadmap that a station without this change can
      short-change a peer from a pruned showfile, so a session should not mix
      builds across it.
- [x] 6.3 Mark `history-pruning` done in `openspec/BACKLOG.md`, and update the
      `plugin-datastores` entry, which names "nothing prunes the oplog" as one of
      the two things it left open.
