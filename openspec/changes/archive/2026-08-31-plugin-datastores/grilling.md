# Grilling: plugin-datastores

Round 1, **answered**. The design in `design.md` was written *before*
`plugin-distribution` was built, and building that turned up three bugs of the
"a placeholder already claimed the digest" kind — states that looked settled and
were not. Two of the five questions below had the same smell, and both turned out
to rest on a premise that was false. The answers are folded into the proposal,
the design, the spec and the tasks; this file is the record of how they were
reached, so round 2 starts from facts rather than re-deriving them.

## What was checked, and what it says

- **Subscription is demand-driven, per collection.** `subscribeDeep` sends
  `Subscribe { <table>/** }` (`frontend/src/lib/ws/proxy.ts:140`) and
  `SubscriptionRegistry::broadcast_update` (`crates/pult-backend/src/api/ws/mod.rs:87`)
  sends only to sessions whose patterns match. `frontend/src/lib/stores/show.ts`
  reference-counts, so a collection nobody has on screen costs nothing, and
  nothing in the frontend subscribes more broadly than one collection.
  `frontend_paths()` is used in exactly one place — `engine/mod.rs:1628`,
  rebroadcasting after a peer snapshot — and that goes through the same filter.
  **So a browser never receives a plugin's store unless a panel asks for it.**
- `Operation::is_undoable` (`crates/pult-schema/src/events/operation.rs:195`) is
  `user_id.is_some() && previous.is_some() && !is_command_path(path)`, and the
  History panel reads `recent_by_people`, whose filter is
  `WHERE user_id IS NOT NULL` (`infra/showfile/oplog.rs:187`). **An unattributed
  write is non-undoable and invisible to history with nothing edited.**
- `Authorship` carries `user_id` and `gesture` independently, and
  `fold_into_the_gesture` (`oplog.rs:72`) keys on the gesture. Dropping the
  attribution therefore does not disturb coalescing.
- `fold_into_the_gesture` **refuses creates**, because every create in a
  collection shares the `<table>/__create` path and folding two would lose a row.
  So N writes to a *new* key in one call leave two rows, not one.
- `create_entity` takes the entity id from the value the caller supplies
  (`engine/mod.rs:1300`), so the host is free to derive it.
- **Nothing prunes the oplog.** There is no DELETE in `oplog.rs`; `history_depth`
  bounds what is read back, not what is kept. `Station` is SYNCED and
  `log_local_write` skips only LOCAL, so telemetry is already logging twice a
  second, forever.
- `crates/pult-backend/src/infra/plugins/instance.rs:195` sets one fresh gesture
  per inbound guest call.
- Still not verified: that a component built against `pult:plugin@0.1.0`
  instantiates against a host offering `0.2.0`'s extra import. Task 1.3 tests
  exactly this and is now marked do-first, because the whole versioning approach
  rests on it.

## The questions and their answers

### Q1 — Every browser gets every plugin's store

**The premise was false.** Subscriptions are demand-driven (see above), so the
3 MB-per-tablet figure describes something that cannot happen, and the choice
between accepting it, adding an opt-out, or cutting quotas as insurance was a
choice between three answers to a non-problem.

➡️ **Quotas stay at 1,000 keys / 1 MB.** The cost that is real is the showfile,
every backup of it, and the whole-state snapshot a joining station swallows, and
1 MB per store is defensible there. The design's "Consequence accepted" paragraph
and its second Risks bullet were rewritten to say what is true; the opt-out that
would have broken a thirty-four-task invariant was never needed.

### Q2 — Nothing bounds the write *rate*

➡️ **(a), and the argument is stronger than it was put.** Not merely that pruning
is the real answer — that the oplog is *already* unbounded and already absorbing
2/s of telemetry, which no plugin will beat. Task 3.7 asserts the coalescing
property with the real row count. A rate limit would invent a failure mode.

### Q3 — Is "a plugin's write is never undoable" actually right?

➡️ **(c), a per-store `undoable` flag defaulting to false** — which turned out to
cost almost nothing, because the mechanism is attribution rather than an
exclusion rule. The host writes with no user by default (non-undoable, absent
from history, no edit to `is_undoable`, the oplog SQL, or the frontend), and
attributes the write to the operator when the store asked for it. A write no
operator caused stays unattributed whatever the store declares, because
`ctx.userId` is absent — the rule falls out rather than being added.

The design's original plan to teach `Operation::is_undoable` about `plugin_data`
is dropped: it would have put plugin knowledge into the schema crate for a
property the schema crate already expresses.

One consequence to build rather than discover: `describeChange` names ids from
fixtures, cues and sequences, so an undoable store write would render
`plugin data → a1b2c3 → value`. Task 3.6 names it properly.

### Q4 — The WIT version bump, now that shows carry bundles

➡️ **(a) as designed**, with task 1.3 moved to the front. (c) stays out: a
bundle's `api` is displayed in the Plugins panel, so lying there is visible to
operators.

### Q5 — Is the grammar cache a good worked example?

➡️ **(b)**: `natural-language-control` remembering the operator's provider and
model. The example should show something that cannot be recomputed, or it argues
against its own feature.

## New, found while checking

### Q6 — Two stations writing the same key made two rows

Not asked in round 1, and a genuine hole in the design. `create_entity` takes the
id from the value supplied, so a fresh v4 per new key would mean station A and
station B each writing `macros/opening` create two `plugin_data` rows holding the
same key — not a conflict the vector clock resolves, but a duplicate it has no
reason to notice, and a plugin reading back two values for one key. The spec said
"last writer wins by vector clock", which only holds if both stations write the
same entity.

➡️ **A UUIDv5 over `(plugin_id, store, key)`.** Both stations then write one row,
the existing per-path conflict resolution is exactly right, and `set` needs no
search of the collection. The spec gained a requirement for it, since the
project's rules ask that requirements hold on a multi-station session.

## Round 2

Nothing outstanding that blocks implementation. The frontier now is the one
unverified claim — task 1.3 — and it is first in the list.
