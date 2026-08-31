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

**Consequence accepted:** `ShowState::frontend_paths()` is derived from the
`EntityMeta` registry, so show-scoped plugin data is visible to frontends like
every other collection. That is mostly a feature — a plugin's web panel can
subscribe to its own store instead of round-tripping through `rpc.handle` — and
partly a cost, noted under Risks.

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

### The API version check becomes a supported set

Adding an interface to the `plugin` world is backward-compatible in the direction
that matters: a component may import **less** than the host offers, so a plugin
built against `0.1` instantiates against a `0.2` host untouched. But
`PluginManifest::validate` compares `api` against a single `API_VERSION` string
and would refuse every existing plugin the moment the contract moves.

So `API_VERSION` becomes `SUPPORTED_API_VERSIONS: &[&str] = &["0.1", "0.2"]`,
the WIT package goes to `0.2.0`, and a manifest naming any supported version
loads. The error message keeps naming what the station speaks — now a list.

**This is a rule with an expiry date**, and the design says so rather than
letting a future author discover it: a version stays in the list only while the
host can still satisfy a guest built against it. The day an interface is
*removed* or a signature changes, that version leaves the list and the plugins
naming it are refused with a clear reason. Additive changes extend the list;
breaking ones truncate it.

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

### Plugin writes are not undoable

`Operation::is_undoable` already excludes commands — the mechanism and the
argument both exist (task 31: a new command is non-undoable by default, which is
the safe direction). Writes whose path names `plugin_data` join them, and the
History panel filter follows.

**One property to preserve rather than assume**: task 32's within-gesture
coalescing replaces an earlier write to the same path inside one gesture, and
`PluginCtx` gives each inbound call one gesture. So a plugin writing the same key
repeatedly while handling one call should collapse to one oplog row. That is the
main thing standing between this feature and a plugin filling the oplog, so the
task list verifies it explicitly instead of trusting that undo-exclusion and
coalescing are independent.

## Risks / Trade-offs

- **A plugin writing show-scoped data in a tight loop floods the oplog**, which
  nothing prunes. → Quotas bound the *size*, not the rate. Within-gesture
  coalescing bounds the common case. Beyond that: the documentation says a store
  is for what an operator would miss, and a station-scoped store is the right
  home for anything written often. A rate limit is a follow-up if a real plugin
  provokes one.
- **All show-scoped plugin data reaches every frontend**, because
  `frontend_paths()` is derived and has no opt-out. → Accepted; adding an opt-out
  would be the first exception to a rule that has held since task 2, and the
  quotas cap what any one plugin can put on the wire. Worth revisiting if a
  plugin's store turns out to be large and of no interest to any panel.
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
  through the frontend proxy, or only through `rpc.handle`. The data is on the
  wire either way; this is a documentation and SDK question, not a schema one.
- **Whether `openspec`-style store versioning** (a plugin declaring a schema
  version for its own data) is worth offering, or is properly the author's
  problem. Currently the author's problem, per the proposal's non-goals.
