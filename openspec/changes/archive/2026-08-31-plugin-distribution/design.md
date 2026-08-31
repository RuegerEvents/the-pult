# Plugin distribution — design

## Context

See `proposal.md` — Why. What matters for the approach is what already exists.

- `PluginManager` (`infra/plugins/mod.rs`) is an actor with per-plugin instance
  actors under it, discovering plugins by walking the directories in
  `Config::plugin_dirs`. It publishes a LOCAL `plugins` key (`PluginsState`) and
  reloads a plugin when a watcher reports its directory changed. Two rules from
  task 8 constrain anything added here: **the manager never awaits guest code**,
  and load sequencing is nothing more than the order instance mailboxes are
  created in.
- `PluginManifest::parse` validates before any WASM is compiled, and already
  rejects a path that leaves the plugin directory and a manifest built against a
  different `API_VERSION`.
- `infra/assets.rs` stores blobs by sha256 in the showfile, serves them at
  `GET /assets/{sha}`, and fetches a missing one from the `stations` rows with
  `x-pult-asset-relay` stopping a forwarding ring. It verifies the digest of what
  comes back before storing it.
- `OutputManager` (task 9) is the worked example of reconciling a manager against
  a PERSISTED collection while the show is up, including the lesson that it must
  not rebuild a live thing when only its label changed.
- `infra/preferences.rs` is the machine-wide settings file, deliberately
  stateless in the router.

## Goals / Non-Goals

**Goals**

- Reuse the asset store rather than build a second content-addressed store.
- Reconcile without restarting the station, and without the manager ever
  awaiting a guest.
- Keep the dev loop — `--plugins`, hot reload — exactly as it is.
- Make the trust boundary legible even though it is not enforced.

**Non-Goals (design-level, beyond the proposal's)**

- No change to `wit/pult-plugin.wit`. A carried plugin and a directory plugin are
  the same thing to the guest; only where the bytes came from differs.
- No new transport. Bundles move over the HTTP path assets already move over.
- No cache eviction policy beyond "unpack on demand, keyed by digest".

## Decisions

### A bundle is a zip, addressed by the digest of the zip

**Why:** the unit that must be replicated is a manifest plus a component plus a
directory of web assets. Hashing the component alone would leave the manifest and
the panels unversioned, and a plugin whose panel script changed would be
indistinguishable from one whose did not.

Zip over tar: the frontend has to produce one at some point (a future "export
this plugin" is a browser action), and zip is the format a browser can build and
an operator can inspect without a shell.

**Alternative considered:** store the manifest as JSON on the roster row and only
the component as an asset. Rejected — it splits one artifact across two stores
and makes "the plugin at digest X" mean nothing on its own.

### The asset store gains a mime, not a sibling

`ACCEPTED` becomes a per-mime table with a ceiling for each, so images keep the
32 MB limit and `application/vnd.pult.plugin+zip` gets its own. `put` and
`fetch_from_peers` are unchanged in shape.

**The one thing to be careful about:** `assets.rs` refuses SVG because serving
one from the console's own origin would let an uploaded file run as the console.
A zip does not execute in a browser, so serving it is inert — but the response
SHALL carry `Content-Disposition: attachment` so a bundle can never be navigated
to as a document.

**Alternative considered:** a separate `plugin_bundles` blob table. Rejected —
`fetch_from_peers`, digest verification, and the dedup that makes re-installing
the same bundle free would all have to be written a second time.

### `plugin_packages` is a PERSISTED entity in pult-schema

One `#[derive(PultSchema)]` type with `#[pult(table = "plugin_packages")]`.
Task 2's registry-driven dispatch means no edit anywhere in `engine/mod.rs`, and
`pult-codegen` produces the TypeScript, the proxy and the migration.

Fields: `id`, `plugin_id`, `name`, `version`, `api`, `sha256`, `enabled`,
`stage`, `config` (JSON), plus the authorship the oplog already stamps.

**`plugin_id` is not the primary key.** The entity id is a uuid like every other
entity, and uniqueness on `plugin_id` is enforced at the install path rather than
by the schema, because nothing in the entity machinery expresses a unique
secondary key and inventing one for this would be the first exception to a rule
that has held for thirty-three tasks.

**Why PERSISTED and not SYNCED:** a roster that vanished on reload would mean a
show that runs different plugins the second time it is opened. See the proposal.

### The LOCAL `plugins` key stays what it is

`PluginsState` describes **what this station is running** — including a directory
plugin the show knows nothing about, and including a failure that is this
station's alone. The roster describes **what the show asks for**. Two different
questions, so two keys, and the panel reads both: the roster gives the rows, the
LOCAL state gives each row's state on this station.

This is the same shape as task 10's `stations` (SYNCED, each station writes its
own row) beside LOCAL per-link latency: a fact about the session and a fact about
this machine are not one field.

### Reconcile is a subscription, and it is a diff by digest

`PluginManager` subscribes to `plugin_packages/**` and, on any change, diffs the
roster against what it runs, keyed by `(plugin_id, sha256)`:

- a digest it is not running and has bytes for → unpack and start,
- a digest it does not have → mark `Fetching`, fetch on a **separate task**, and
  send itself a message when the bytes land,
- a plugin id no longer in the roster, or disabled → stop,
- a row that changed in `name` or `stage` only → publish, start nothing.

That last case is task 9's lesson repeated: rebuilding a live thing because its
label changed is a visible fault, and here it would be a plugin restarting during
a show because somebody fixed a typo.

The fetch must not run inside the event loop. Task 8's deadlock — the manager
awaiting something that can call back into the manager — is the same shape, and a
ten-second HTTP timeout inside the loop would stall every plugin call in the
station.

### The unpack cache is station-local and keyed by digest

`<config-dir>/the-pult/plugin-cache/<sha256>/`. Immutable by construction, so a
cache hit needs no validation and two shows carrying the same plugin share one
directory. The existing `PluginManifest` machinery then works unchanged, because
after unpacking a carried plugin *is* a directory with a `pult-plugin.toml` in it.

**Zip extraction is the one genuinely dangerous step here** and gets treated as
such: entry paths are rejected if absolute or containing `..` (the rule
`contained_relative_path` already states for manifest paths), the uncompressed
total and the entry count are capped, and symlink entries are refused outright.

The watcher must not watch the cache. A directory it unpacked would report a
change and reload the plugin it just started.

### Directory beats roster, per station

The dev loop is the reason the plugin runtime is usable, and a station joined to
a session that silently ran the show's copy of a plugin the developer is editing
would be the worst kind of confusing. `discover()` already returns directory
plugins; those ids are simply removed from the reconcile set, and the LOCAL state
carries an `overridden_by_disk` flag so the panel can say so.

### Configuration: manifest, then show, then station

Deep-merged in that order, reusing the `deep_merge` already in `manifest.rs`. The
manifest's `[config]` and a `config.toml` beside it keep working for directory
plugins; a carried plugin's cache directory has no `config.toml` and must not
grow one, since it is shared by digest across shows.

**Station-last, not show-last.** The most specific layer wins, and the things a
station legitimately overrides — a credential, a local model URL, a machine with
a different GPU — are exactly the things that must not be in a showfile. A
show-last order would make a station physically unable to correct a value the
show got wrong for it.

Station-level config lives in `preferences.toml` under a per-plugin table.
`Preferences` gains a `plugins: BTreeMap<String, toml::Table>`, which keeps the
file's "read it, use it, never fail" contract.

### Install is `POST /api/plugins`, multipart

It stores the bundle as an asset, opens it far enough to validate the manifest,
and writes the roster row — refusing before the write if the manifest is invalid,
the API version is wrong, or the id is malformed. Validating before storing means
a rejected upload leaves nothing behind.

Removal is an ordinary entity delete through the existing path, so it is
undoable, attributed and replicated like any other write.

## Risks / Trade-offs

- **Opening a showfile runs its plugins.** → Not mitigated by design; it is the
  chosen model (see proposal — Trust assumption). What bounds it is the sandbox,
  the epoch deadline, and the manifest permission gates, all unchanged. What this
  design adds is legibility: permissions are shown as text beside each package,
  and the digest is stable and displayable, so "is this the bundle I shipped?" is
  answerable. An approval gate is an open question below, and the schema is laid
  out so adding one is a field and a status, not a redesign.
- **A bundle carrying a plugin built for a newer API** → refused with both
  versions named; the show still opens and the rest of the roster still runs.
  Already how `PluginManifest::parse` behaves; the new part is that it must not
  be fatal to the *show*.
- **A large bundle blocks a station from being useful** → fetch is on its own
  task with a timeout, `Fetching` is a reported state, and every other plugin
  loads meanwhile.
- **Zip bombs and path traversal** → caps on entry count and uncompressed size,
  path containment, symlinks refused. Tested with hostile archives, not only
  well-formed ones.
- **The cache grows without bound**, one directory per digest ever carried. →
  Accepted for now, consistent with assets never being pruned (task 13). It is
  station-local, so it can be deleted by hand with no loss.
- **Two operators installing different versions of one plugin at once** →
  last writer wins by the existing vector-clock rules, and the loser's bundle
  stays in the asset store unreferenced. Acceptable: the roster converges, which
  is the property that matters.
- **A `stage` hint that gates nothing** → risks reading as unimplemented. It is
  recorded as advisory in the spec so the next person knows it was a decision.

## Migration Plan

Additive throughout. The new column set arrives as a fresh table, so an existing
showfile opens with an empty roster and behaves exactly as it does today. A
station with no `--plugins` and a show with no roster runs no plugins, which is
also today's behaviour.

Rollback is opening the showfile on a build without this change: the
`plugin_packages` table is ignored, `--plugins` still works, and nothing else in
the show refers to the roster. Saved layouts naming a carried plugin's panels
degrade to a missing panel, which is what they already do for any unknown panel.

## Open Questions

- **Should the `stage` hint eventually gate loading** — skipping setup-only
  plugins once a show is running? Deferrable: the field is recorded either way,
  and gating is a later behavioural change with its own spec delta.
- **Where an approval gate would sit** if one is wanted later. The likely shape
  is a station-level list of trusted digests in `preferences.toml` and a
  `PluginStatus::AwaitingApproval`, which is additive to everything here. Not
  built, deliberately.
- **Whether a bundle should be exportable from the console** ("give me the
  plugin this show is running as a file"). Cheap once bundles are assets, and it
  changes no requirement here.
