# Plugin datastores — design

## Context

See `proposal.md` — Why. What shapes the approach:

- `host_impls.rs` is where every guest import is implemented and where manifest
  permissions are enforced — "not in the guest, which is untrusted, and not in
  the manager, which never sees individual calls". A store interface belongs
  there and nowhere else.
- `PluginCtx` already carries `user` and `gesture`, set by the instance actor
  around each inbound call, so writes a plugin makes are attributed and one
  inbound call is one gesture.
- The engine's write path (`data.set` in the host) resolves lifecycle from the
  path and needs no per-type code (task 2).
- `preferences.rs` and `identity.rs` are the two existing examples of state that
  is persistent but deliberately not part of the show.

## Goals / Non-Goals

**Goals**

- Two honest homes for two genuinely different kinds of data.
- No new replication, persistence or catch-up code for the show-scoped half.
- An existing plugin keeps loading with no edit.

**Non-Goals (design-level)**

- No transactions across keys, no compare-and-swap. A single `set` is atomic;
  anything larger is the plugin's problem.
- No change to how permissions are declared for anything other than stores.

## Decisions

### The word is `scope`, not `lifecycle`

The manifest says `scope = "show"` or `scope = "station"`, not
`lifecycle = "persisted" | "local"`, and the difference matters rather than
being cosmetic.

**In this codebase LOCAL means "not persisted".** `Lifecycle::Local` is state a
station holds and shares with its own frontends and which does not survive a
reload — `live_effects`, `PluginsState`, the peer latency table. A plugin store
that vanished when the station restarted would be useless for the thing a
station-scoped store is *for*, which is remembering across restarts. Calling it
LOCAL would have made the documentation lie about the one property anybody cares
about.

What a station-scoped store actually is, is **persistent and not replicated** —
a fourth combination the lifecycle enum has no name for but which the console
already has two instances of, in `identity` and `preferences`. Naming the axis
`scope` says the true thing (whose data is this?) and leaves `Lifecycle` alone.

### Show-scoped data is an ordinary entity

A `PluginDatum` in `pult-schema` with `#[pult(table = "plugin_data")]`, fields
`plugin_id`, `store`, `key`, `value` (JSON), all PERSISTED.

Everything then arrives for free and correct: SQLite persistence, replication to
peers, catch-up from the oplog, the snapshot round trip, vector-clock conflict
resolution, and `showfile::upgrades` having nothing to do because the table is
new. This is exactly the promise task 2 made — a new collection needs no edit
outside `pult-schema` — and taking it is far better than writing a second
persistence path that would have to be kept in step with the first.

**Alternative considered:** a bespoke `plugin_data` SQL table written directly by
the host, outside the entity machinery. Rejected: it would need its own
replication, and a plugin's macros not reaching the second console would be a
bug reported as "the plugin is broken".

**Consequence, and it is smaller than it looks:** show-scoped plugin data is
*reachable* from a frontend like every other collection, but reachable is not
sent. Subscription is demand-driven and per collection — `subscribeDeep` asks for
`plugin_data/**` and `broadcast_update` sends only to sessions whose patterns
match — and `stores/show.ts` reference-counts, so a collection nobody has on
screen costs nothing. `frontend_paths()` is used in one place, rebroadcasting
after a peer snapshot, and that goes through the same filter. No browser receives
a plugin's store unless something asks for it.

So this is a feature with no matching cost on the wire: a plugin's web panel can
subscribe to its own store instead of round-tripping through `rpc.handle`, and a
tablet with no plugin panel open never sees a byte of it. The real cost of
choosing an entity is elsewhere — the showfile, every backup of it, and the
whole-state snapshot a joining station has to swallow — and that is what the
quotas are sized against.

### A key's identity is its row's identity

The entity id of a `PluginDatum` is a UUIDv5 over `(plugin_id, store, key)`, not
a fresh v4.

This is what makes the store correct on more than one station rather than merely
persistent. `create_entity` takes the id from the value the caller supplies, so a
random id would mean two stations each writing `macros/opening` create two rows
holding the same key — not a conflict the vector clock resolves, but a duplicate
it has no reason to notice, and a plugin reading back two values for one key. A
derived id makes both stations write the *same* entity, at which point the
existing per-path conflict resolution is exactly right and no new merge rule is
needed.

It also makes `set` cheaper: the host knows the path without searching the
collection for a matching key.

### Station-scoped data is one SQLite file beside the preferences

`<config-dir>/the-pult/plugin-data.db`, a single table keyed by
`(plugin_id, store, key)`. SQLite rather than a directory of TOML files because
two consoles on one machine can have it open, quotas want a `SUM(length(value))`
rather than a directory walk, and sqlx is already a dependency.

It follows `preferences.rs`'s contract: a station that cannot open it logs and
carries on with the stores reporting empty, because a plugin's cache is never a
reason to keep an operator from their show.

`PULT_PLUGIN_DATA` names the file outright, the way `PULT_PREFERENCES` does, so
tests get their own and two stations on one machine can be separated.

### The contract moves to 1.0, and `api` becomes a floor

The original plan — take the package to `0.2.0` and turn `API_VERSION` into a
supported-versions list — **does not work**, and the reason is worth writing
down because it is invisible until you try it.

A component may indeed import *less* than the host offers: a guest that never
calls `store` does not import it, and that half of the premise held. But a
component's imports are stamped with the **package version** they were built
against. A `0.1` guest asks for `pult:plugin/data@0.1.0`; a `0.2` host offers
`pult:plugin/data@0.2.0`; nothing resolves, and the plugin fails to instantiate
with `a matching implementation was not found in the linker`. Wasmtime does
resolve component imports semver-compatibly — but under semver a `0.x` minor
bump *is* a breaking change, so `0.1` and `0.2` are unrelated. **At `0.x` the
contract can never grow additively, however the check is spelled.** A
supported-versions list would have passed validation and then failed to link,
which is worse than refusing honestly.

So the package is **`1.0.0`**, where a minor bump is additive and wasmtime's
matching applies, and the manifest's `api` is a **floor**: the station's major
must equal the plugin's and its minor must be at least the plugin's. That is
exactly the rule wasmtime enforces underneath, asked early enough to answer an
operator rather than a linker — and a pre-flight check that disagreed with the
linker would be worse than none.

Verified rather than argued: a component built against `1.0.0` runs unchanged on
a `1.1.0` host, extra import and all. `scripts/check-api-compat.sh` reproduces
the check — it builds a component, bumps the contract, and runs a station
against it — because a wasmtime upgrade could quietly change the answer and
nothing else in the tree would notice.

**The cost, paid once:** every bundle built against `0.1` must be rebuilt. It is
refused by name, as a major mismatch, with a message saying to rebuild it. Taken
now, while the ecosystem is two reference plugins and any bundles in showfiles
are freshly built; the alternative is a contract that can never grow.

### Declaring the store is the permission

No new key under `[permissions]`. A store is the plugin's own namespace — it
cannot address another plugin's data, and the host derives the physical location
from `(plugin_id, store)` rather than from anything the guest passes. What an
operator needs to know is "does this plugin keep data, and does that data go into
my showfile", and the `[[stores]]` block answers exactly that where the rest of
the permissions are read.

The host still enforces: a `store` argument naming an undeclared store is refused
before anything is read or written, in `host_impls.rs`, from the manifest the
instance was started with.

### Quotas are enforced on write, in the host

Defaults of 1,000 keys and 1 MB per store, which a manifest may lower and not
raise. Checked in `set`, before the write, so a refused write leaves the store
exactly as it was.

A ceiling that a plugin could raise would not be a ceiling. The reason to let it
*lower* one is that a plugin author who knows their cache should never exceed
sixteen keys is saying something useful to the operator reading the manifest.

### Attribution is the switch, so nothing has to learn about plugins

A plugin's write is non-undoable and absent from the History panel by writing it
with **no user**. `Operation::is_undoable` already requires `user_id.is_some()`,
and the History panel reads `recent_by_people`, whose filter is
`WHERE user_id IS NOT NULL`. An unattributed write therefore satisfies both
conditions with no edit to `pult-schema`, no edit to the oplog's SQL, and no edit
to the frontend — where the design previously proposed teaching `is_undoable`
about `plugin_data`, which would have put plugin knowledge into the schema crate
for a property the schema crate already expresses.

`Authorship` carries `user_id` and `gesture` independently, so the write keeps
its gesture. That matters: `fold_into_the_gesture` keys on the gesture, not on
the user, so coalescing is unaffected by dropping the attribution.

**And because attribution is a switch, a store may ask for it.** A store
declaring `undoable = true` gets its writes attributed to the operator in
`PluginCtx`, and is then undoable and visible in the history by the same two
rules, still with nothing edited. Default false, which is the safe direction and
the one task 31 already chose for commands. The argument for offering it at all:
an operator who clicks *Save macro* and presses Ctrl-Z means the macro, and a
console that instead took back their previous edit would be silently doing the
wrong thing. The argument for the default: a plugin caching a derived value while
handling an operator's command would otherwise put an invisible entry at the top
of that operator's undo stack.

`ctx.userId` is absent for a write nobody asked for — a timer, `lifecycle.init`,
a `call-plugin` chain with no person at its head — so those stay unattributed
whatever the store declares. The rule falls out of the existing mechanism rather
than being a second one.

**One consequence to build rather than discover**: an undoable store write shows
in the History panel, and `describeChange` names ids from the fixtures, cues and
sequences it holds. A `plugin_data` row would render as
`plugin data → a1b2c3 → value`. It has to be named by its plugin, store and key,
or the feature produces an entry nobody can read.

**One property to preserve rather than assume**: task 32's within-gesture
coalescing replaces an earlier write to the same path inside one gesture, and
`PluginCtx` gives each inbound call one gesture. So a plugin writing the same key
repeatedly while handling one call collapses in the log. But **not to one row**:
the first write to a key that does not exist yet is a create, and
`fold_into_the_gesture` refuses creates on purpose, because every create in a
collection shares the `<table>/__create` path and folding them would lose a row.
Ten writes to one new key inside one call therefore leave two rows — the create,
and one folded value write — and ten writes to an existing key leave one. That is
the property that keeps a plugin from filling the log, so the task list asserts
the actual number rather than a plausible one.

## Risks / Trade-offs

- **A plugin writing show-scoped data in a tight loop appends to the oplog**,
  which nothing prunes — and *nothing* is meant literally: `history_depth` bounds
  what is read back, not what is kept, and there is no DELETE anywhere in
  `oplog.rs`. → Not this change's problem to solve, and the scale says why: the
  `stations` table is SYNCED, so telemetry is already logging twice a second
  forever, which no plugin is going to beat. Quotas bound the size, coalescing
  bounds the common case, and the documentation says anything written often
  belongs in a station-scoped store, which never reaches the log at all. A rate
  limit here would invent a failure mode to prevent a problem that pruning is the
  real answer to.
- **Show-scoped plugin data lands in the showfile and in every backup**, and in
  the whole-state snapshot a joining station is handed. → This, not frontend
  bandwidth, is what the quotas are for. An earlier draft of this design worried
  that every browser would receive every plugin's store; that was wrong —
  subscriptions are demand-driven and per collection, so a browser gets a store
  only if a panel asks for it. The station-side cost is real and bounded at
  1 MB per store.
- **Nothing stops an author putting a credential in a show-scoped store**, from
  where it replicates and lands in every backup. → Documented, not enforced; a
  host cannot tell a token from a string. `docs/PLUGINS.md` states the two
  correct homes at the point where stores are introduced.
- **Orphaned data accumulates** for plugins that were removed. → Deliberate, so a
  mistaken removal is recoverable. An operator can see it grouped by plugin id
  and delete it, which is more than assets get today.
- **Two stations writing the same show-scoped key** resolve by the existing
  vector clock, last writer wins. → Correct for a cache, wrong for a counter. A
  plugin needing agreement between stations should ask the leader, as device
  driving already does.

## Migration Plan

Additive. A new table appears in the generated migration; an existing showfile
opens with it empty. Every existing plugin loads unchanged because it declares no
stores and imports no store interface, which is asserted by a test rather than
assumed.

Rollback is a build without the change: the `plugin_data` table is ignored, the
station-scoped file is left on disk untouched, and a plugin built against `0.2`
is refused by name with the version mismatch — which is the intended behaviour of
that check, not a failure.

## Open Questions

- **Whether a rate limit on show-scoped writes is needed**, and if so whether it
  belongs here or in the general oplog-pruning work the roadmap has deferred
  since task 6. Deferrable: it changes no requirement in the spec.
- **Whether a store should be readable by a plugin's web panel directly**
  through the frontend proxy, or only through `rpc.handle`. Now that
  subscription is known to be demand-driven, the panel subscribing to its own
  store is the cheap path and `rpc.handle` the expensive one, so this is a
  documentation and SDK question with an obvious answer waiting for a plugin to
  confirm it.
- **Whether `openspec`-style store versioning** (a plugin declaring a schema
  version for its own data) is worth offering, or is properly the author's
  problem. Currently the author's problem, per the proposal's non-goals.
- **Whether an undoable store wants a change notification.** Undo writes the
  previous value back into `plugin_data`; a plugin holding the value in memory
  will not learn about it until it next reads. Same property peer replication
  already has, so no new mechanism is proposed — but a macro plugin is the first
  thing likely to notice.
