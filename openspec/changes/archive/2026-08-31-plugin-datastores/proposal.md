# Plugin datastores

## Why

A plugin cannot remember anything. `lifecycle.init` runs, the plugin builds
whatever it needs, and a reload or a restart throws all of it away — hot reload
is documented as a fresh `init` precisely because there is nothing to preserve.
So a command-line plugin re-derives its grammar from introspection on every
start, and nothing can offer the features that are obviously wanted next: a saved
macro, a remembered provider, a per-show snippet library.

The two kinds of thing a plugin wants to remember are not the same kind of thing.
A macro an operator wrote belongs to the **show** and should be on every console
and in the backup. A cached grammar or a last-used tab belongs to the **machine**
and must not travel in a showfile. A single store would force every plugin author
to lie about one of them.

## What Changes

- **A new `store` interface in the WIT contract**: `get`, `set`, `delete` and
  `keys` (spelled that way because `list` is a WIT keyword), each naming which of
  the plugin's stores it addresses. Values are JSON text, like everything else
  that crosses the boundary.
- **A plugin declares its stores in its manifest**, each with a scope:
  - `scope = "show"` — persisted in the showfile and replicated to peers.
  - `scope = "station"` — persisted on this machine, never replicated, never
    written into a showfile.
  Declaring a store is what grants access to it; a plugin can address no store
  it did not declare, and no other plugin's.
- **Show-scoped data is an ordinary entity.** A PERSISTED `plugin_data`
  collection in `pult-schema`, so replication, persistence, catch-up and the
  snapshot round trip all work with no new sync code — the payoff of task 2's
  registry-driven dispatch.
- **Station-scoped data is a file beside the station's other machine-wide
  state**, next to `preferences.toml` and for the same reason.
- **Plugin writes are excluded from undo and from the History panel by default**,
  the way commands already are (task 31). A plugin's cache key appearing in the
  list of what people did would be noise, and Ctrl-Z restoring one means nothing
  to an operator. **A store may declare otherwise** — an operator who clicked
  *Save macro* and presses Ctrl-Z means the macro, and a console that took back
  their previous edit instead would be silently doing the wrong thing. Both
  behaviours come from whether the host attributes the write to the operator, so
  neither needs a mechanism of its own.
- **A key names the same datum on every station.** A `PluginDatum`'s id is
  derived from `(plugin_id, store, key)`, so two stations writing one key write
  one row and the show's existing conflict resolution applies. A fresh id per
  write would have left the store holding the same key twice.
- **Quotas per store**, enforced by the host at `set`: a key count and a total
  byte ceiling, with defaults a plugin may lower and not raise.
- **Data outlives its plugin.** Removing a plugin SHALL NOT delete its stores. A
  plugin removed by mistake and reinstalled finds its macros where it left them.

## Capabilities

### New Capabilities

- `plugins/datastores`: what a plugin may store, where each kind goes, who can
  read it, what happens when it is gone, and what the limits are.

### Modified Capabilities

None. The plugin runtime's existing behaviour is unchanged; this only adds an
interface a plugin may choose to import.

## Non-goals

- **No cross-plugin store access.** A plugin reads another plugin's data by
  asking it over `call-plugin`, which is already permission-gated and already
  refuses cycles. Two doors into one plugin's state would need a second
  permission model to guard the other one.
- **No queries.** Key-value with a prefix listing. A plugin that wants an index
  builds one out of keys.
- **No secrets in stores.** Credentials keep the two homes they have: the
  environment passthrough a manifest declares by name, and station-level
  configuration. Show-scoped data replicates and lands in backups, so a store is
  the wrong place for a key and the documentation must say so.
- **No migration tooling for a plugin that changes its own data shape.** That is
  the plugin author's problem, as it is for any application.
- **No pruning of orphaned data** beyond an operator being able to see and delete
  it.

## Impact

- `wit/pult-plugin.wit`: a new `store` interface imported by the `plugin` world,
  and the package moved to `1.0.0`. It cannot stay at `0.x`: a component's
  imports carry the package version, and under semver a `0.x` minor bump is
  breaking, so *any* addition would strand every plugin already built. The
  manifest's `api` becomes a floor the station must meet rather than a string to
  match — see `design.md`, which records what was verified rather than assumed.
- `crates/pult-schema`: a `PluginDatum` entity, and nothing else. `is_undoable`
  is left alone — its existing "somebody has to have asked for it" clause is what
  decides a store write, so the schema crate never learns what a plugin is.
- `crates/pult-backend/src/infra/plugins/`: manifest `[[stores]]` parsing and
  validation, the host implementation of the interface with permission and quota
  enforcement, and the station-scoped backing file.
- `frontend`: the History panel names a `plugin_data` row by its plugin, store
  and key, so an undoable store write reads as something rather than as a uuid.
- `plugins/sdk`: a typed wrapper so a plugin author writes `store::get` rather
  than assembling JSON.
- `plugins/natural-language-control`: remembers the operator's chosen provider
  and model in a station-scoped store — the worked example, and the thing that
  proves the interface is usable. Deliberately not a cache of derived data: an
  example that could be recomputed argues against its own feature.
- `docs/PLUGINS.md`: the chapter on what a plugin may remember.
