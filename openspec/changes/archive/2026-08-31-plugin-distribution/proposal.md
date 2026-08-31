# Plugin distribution

## Why

A plugin is currently a directory on one station's disk, named by `--plugins` at
startup. That is right for developing one and wrong for everything else: a show
authored with the command-line plugin's panels in its layout degrades on every
other console in the session, installing a plugin on a ten-station rig is ten
manual copies, and there is no way to get a plugin onto a console that is not a
dev checkout. `PluginsState` reports what a station is running; nothing anywhere
records what a show *needs*.

The asset store already solved the hard half of this for stage plans in task 13:
content-addressed blobs that a station fetches from a peer on demand and verifies
by hash before storing. A plugin bundle is the same shape of problem, so the
mechanism is reused rather than reinvented.

## What Changes

- **A show carries its plugins.** A new PERSISTED `plugin_packages` collection
  records the roster: plugin id, name, version, API version, the sha256 of its
  bundle, whether it is enabled, and its show-level config. PERSISTED because
  every console working one show must run the same plugins — a station-local
  roster would not be a preference, it would be a disagreement about what the
  show *is*, the same argument `Show.history_depth` settled in task 33.
- **The bundle is an asset.** A plugin ships as a zip of `pult-plugin.toml`, its
  component, and `assets/`, stored in the existing content-addressed asset store
  and fetched from peers by the existing `fetch_from_peers` path. The store gains
  a bundle mime and its own size ceiling; nothing else about it changes.
- **Stations reconcile against the collection.** `PluginManager` grows the
  reconcile loop `OutputManager` already has (task 9): the roster changes, the
  manager unpacks what it lacks into a station-local cache keyed by sha and
  starts it, and stops what has been removed. **Auto-run: a station runs what the
  show carries, with no per-station approval step.**
- **`PluginStatus` gains `Fetching`.** A station that has the roster row but not
  yet the bytes is in a real state and says so, rather than looking failed.
- **Installing is an upload.** `POST /api/plugins` takes a bundle, stores it, and
  writes the roster row; the Plugins panel gets an install control and a remove
  control behind the Edit lock. Because the roster replicates, installing on one
  console installs on all of them.
- **`--plugins` stays, and wins locally.** A directory-loaded plugin still hot
  reloads and is still absent from the show. Where a dev directory and the show
  both carry the same plugin id, the directory wins on that station and the panel
  says so — otherwise editing a plugin on a station joined to a session would
  silently run the show's copy instead.
- **Plugin config becomes layered**: the manifest's `[config]` table, then the
  roster row's show config, then a per-plugin table in `preferences.toml`. Most
  specific wins, so a station can always override — which is where an API key or
  a local model URL belongs, and neither of those may travel in a showfile.
- **A `stage` hint** (`setup` / `runtime` / `both`, default `both`) declared in
  the manifest, recorded and used to group the panel. Advisory only in this
  change: nothing gates loading on it.

## Capabilities

### New Capabilities

- `plugins/distribution`: what a show records about the plugins it needs, how
  bundles reach a station that lacks them, how a station reconciles what it runs
  against the show's roster, and how that interacts with dev directories.
- `plugins/configuration`: where a plugin's settings live and which layer wins.

### Modified Capabilities

None — the plugin runtime has no spec under `openspec/specs/` yet, and this
change does not alter how a loaded plugin behaves once it is running.

## Trust assumption

**Opening a showfile runs the plugins it carries.** That is the deliberate choice
here, and it means a showfile is trusted input in the way a binary someone hands
you is trusted input. What still bounds it: the wasmtime sandbox, the epoch
deadline that traps a guest running five seconds, and every manifest permission
gate — data access, the outbound HTTP host allowlist, and env passthrough by
name. What it does not bound: a carried plugin with `data = "read-write"` and an
HTTP allowlist can move show data off the network, and `env` passthrough can
reach a key such as `PULT_LLM_API_KEY` on whichever station opens the show.

The Plugins panel therefore shows each package's permissions as plain text
beside it, so what a show is asking for is readable without opening the bundle.
An approval gate is recorded as an open question in `design.md`, not built here.

## Non-goals

- No approval, signing, or provenance checking of bundles.
- No install from a URL or a registry; the install path is a file upload.
- No change to the WIT contract, the permission model, or the sandbox.
- No asset pruning. A replaced bundle's bytes stay in the showfile, exactly as a
  replaced stage plan's do (task 13 left this open and it stays open).
- No gating of loading on the `stage` hint.
- Plugin persistence is out of scope — that is the sibling `plugin-datastores`
  change.

## Impact

- `crates/pult-schema`: a new `PluginPackage` entity with `#[pult(table = ...)]`.
  Registry-driven dispatch means no edit in `engine/mod.rs`; codegen regenerates
  the TypeScript, the proxy, and the migration.
- `crates/pult-backend/src/infra/assets.rs`: a bundle mime and its size ceiling.
- `crates/pult-backend/src/infra/plugins/`: a reconcile loop, a bundle unpacker
  and a station-local cache, plus the `Fetching` status.
- `crates/pult-backend/src/api/`: `POST /api/plugins`.
- `crates/pult-backend/src/infra/preferences.rs`: per-plugin station overrides.
- `frontend/`: the Plugins panel gains install, remove, permissions and status.
- `docs/PLUGINS.md`: how to package and ship a plugin, and where config goes.
