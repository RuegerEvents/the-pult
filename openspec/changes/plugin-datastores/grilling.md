# Grilling: plugin-datastores

Round 1, asked and **not yet answered**. The design in `design.md` was written
*before* `plugin-distribution` was built, and building that turned up three bugs
of the "a placeholder already claimed the digest" kind — states that looked
settled and were not. Q1 and Q3 below have the same smell.

Answer these, then recompute the frontier for round 2. Fold the answers in with
`/opsx:update plugin-datastores` before implementing anything.

## Facts already established (do not re-derive)

- `ShowState::frontend_paths()` (`crates/pult-backend/src/engine/mod.rs:89`) is
  derived from every entity with a table, and `engine/mod.rs:1628` broadcasts
  each wholesale. A `plugin_data` entity therefore reaches **every browser**, and
  no opt-out mechanism exists.
- `Operation::is_undoable` (`crates/pult-schema/src/events/operation.rs:195`) is
  `user_id.is_some() && previous.is_some() && !is_command_path(path)`. Plugin
  writes *do* carry `user_id` (from `ctx.userId`), so they are undoable by
  default — excluding them is an edit to that function.
- `crates/pult-backend/src/infra/plugins/instance.rs:195` sets one fresh gesture
  per inbound guest call, so task 32's within-gesture coalescing should apply to
  repeated writes. Nothing asserts this yet.
- The oplog already absorbs machine-generated churn: station telemetry writes
  twice a second. There is precedent for non-user writes in the log.
- Not yet verified: that a component built against `pult:plugin@0.1.0`
  instantiates against a host offering `0.2.0`'s extra import. Task 1.3 tests
  exactly this; do it early, because the whole versioning approach rests on it.

## The questions

### Q1 — Every browser gets every plugin's store

Show-scoped data as a `PluginDatum` entity buys replication, persistence and
catch-up for free, but `frontend_paths()` is derived and has no opt-out, so a
tablet receives every plugin's entire store on connect and every change after.
At the design's quotas (1000 keys × 1 MB per store) three plugins could put 3 MB
on the wire per browser.

- (a) accept it, and call panel-subscribes-to-own-store a feature
- (b) add an opt-out to the derived rule — the first exception since task 2
- (c) cut quotas hard (say 64 KB) so it cannot matter

➡️ **(a) with (c) as insurance**: default 256 KB / 500 keys. An opt-out breaks an
invariant that has held for thirty-four tasks to solve a problem no real plugin
has. Quotas can be raised later; a broken invariant cannot be un-broken.

### Q2 — Nothing bounds the write *rate*

Quotas bound size. A plugin writing one key in a loop appends to an oplog nothing
prunes.

- (a) document it and rely on gesture coalescing
- (b) rate-limit show-scoped writes in the host, refusing beyond N/sec
- (c) defer to the oplog-pruning work deferred since task 6

➡️ **(a) plus a hard assertion**: test that ten writes to one key inside one call
leave one oplog row, and document that anything written often belongs in a
station-scoped store. A rate limit invents a failure mode to prevent a problem
pruning is the real answer to.

### Q3 — Is "a plugin's write is never undoable" actually right?

The design excludes `plugin_data` from undo the way commands are. But consider a
macro plugin: the operator clicks *Save macro*, the plugin writes show-scoped
data, the operator regrets it, presses Ctrl-Z — and takes back their *previous*
edit instead, silently doing the wrong thing.

- (a) exclude all plugin writes (design as written)
- (b) include them, so a store write undoes like any edit
- (c) let a plugin declare per store whether its data is operator-visible, and
  undo only those

➡️ **(a) for now, documented**: a plugin wanting an undoable action should write
to the show's own entities through `data.set`, which is already undoable and
attributed. A store is the plugin's private memory. Held loosely — this is the
answer I am least sure of.

### Q4 — The WIT version bump, now that shows carry bundles

Adding `store` moves the package to 0.2.0 and turns `API_VERSION` into a
supported *list*, on the argument that a guest may import less than the host
offers. Distribution raises the stakes: a showfile now carries bundles between
machines, so a 0.1 bundle will routinely open on a 0.2 station and vice versa.

- (a) supported-versions list as designed
- (b) treat `api` as a *minimum* the station must meet
- (c) do not bump — add the interface under 0.1

➡️ **(a), verifying the superset claim first** (see Facts). (c) is out: a
bundle's `api` is now displayed in the Plugins panel, so lying there is visible
to operators.

### Q5 — Is the grammar cache a good worked example?

Task 6.2 has `command-line` cache its introspection-derived grammar in a
station-scoped store. But that grammar is *derived data* — cheap to rebuild, and
a stale one is worse than none, since the schema can change under it.

- (a) keep it, keying the cache on a schema fingerprint
- (b) use `natural-language-control` remembering the operator's last
  provider/model — genuinely state, not derivable
- (c) both

➡️ **(b)**: the example should show something that cannot be recomputed, or it
argues against its own feature.
