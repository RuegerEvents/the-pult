# Plugin datastores — tasks

Each group is meant to end in its own commit with `cargo test`, the plugins
workspace's own tests, and a zero-warning build passing.

## 1. The contract

**1.3 was done first, and it failed.** The premise — that a component built
against `0.1` still instantiates against a `0.2` host — is false. A component's
imports are stamped with the package version, so a `0.1` guest asks for
`pult:plugin/data@0.1.0` while a `0.2` host offers `@0.2.0` and nothing
resolves; wasmtime's semver-compatible linking cannot bridge it because under
semver a `0.x` minor bump *is* breaking. At `0.x` the contract could never grow
without stranding every plugin already in a showfile. **So the package moved to
`1.0.0` and the manifest's `api` became a floor rather than a match.** A `1.0`
component running on a `1.1` host is verified, not assumed. The tasks below say
what was built.

- [x] 1.1 Add a `store` interface to `wit/pult-plugin.wit` — `get`, `set`,
      `delete` and `keys` (`list` is a WIT keyword), each taking the store id —
      and import it in the `plugin` world. Take the package to `1.0.0`. Verify
      the host bindings generate and the workspace still builds.
- [x] 1.2 Make `API_VERSION` an `ApiVersion { major, minor }` and the manifest's
      `api` a floor: same major, station's minor at least the plugin's. Verify
      with tests that a station runs what was built against it and against any
      earlier minor, refuses a later minor and either direction of a major, and
      that the two failures say which thing to change — a plugin to rebuild, or
      a console to update.
- [x] 1.3 Verify a component built against an *earlier minor* still instantiates
      against this host. Needs a component built against a version this tree no
      longer has, so it is an `#[ignore]`d test in
      `crates/pult-backend/tests/plugins.rs` driven by
      `scripts/check-api-compat.sh`, which builds one, bumps the contract, and
      runs the station against it. Nothing is checked in: the fixture is a build
      output, which is the point.

## 2. Declaring stores

- [x] 2.1 Parse `[[stores]]` in the manifest — `id`, `scope`, optional
      `max_keys`, `max_bytes` and `undoable` (default false). Verify with tests
      that a duplicate store id, an unknown scope, and a limit above the default
      are each refused by name, and that `undoable` on a station-scoped store is
      refused too: station data never reaches the oplog, so there is nothing
      there to take back and a manifest saying otherwise is wrong rather than
      ignored.
- [x] 2.2 Carry the declared stores into `PluginCtx` at instance start, so the
      host resolves a store from the manifest rather than from anything the guest
      passes. Verify a call naming an undeclared store fails before any read or
      write.

## 3. Show-scoped storage

- [x] 3.1 Add a `PluginDatum` entity to `pult-schema` with
      `#[pult(table = "plugin_data")]` and PERSISTED `plugin_id`, `store`, `key`,
      `value`. Verify the generated migration gains the table and that
      `engine/mod.rs` needed no edit.
- [x] 3.2 Derive the entity id as a UUIDv5 over `(plugin_id, store, key)` in the
      host, so a key names one row on every station. Verify with a two-station
      test that both writing the same key of the same store converge on one row
      holding one value — with a random id this is two rows and a plugin reading
      two values for one key, which is the failure this exists to prevent.
- [x] 3.3 Run `cargo run -p pult-codegen -- generate` and verify the TypeScript,
      the proxy and the migration are the only changed generated files.
- [x] 3.4 Implement the show-scoped half of the host interface through the
      engine's ordinary write path, attributing the write to the operator only
      when the store declared `undoable` and `PluginCtx` has a user — otherwise
      writing with no user and the gesture kept. Verify a plugin's write
      replicates to a peer station in the integration tests.
- [x] 3.5 Verify that no edit to `Operation::is_undoable` or to
      `recent_by_people` was needed: a write to an ordinary store is unattributed
      and so is neither undoable nor in the history, and an operator's undo after
      one takes back the operator's own last change. Then verify the other
      direction — a write to a store declaring `undoable`, made while handling an
      operator's call, is undoable and appears in the history; and the same store
      written from a timer or from `lifecycle.init` is neither.
- [x] 3.6 Name a `plugin_data` row in the History panel by its plugin, store and
      key. `describeChange` names ids from the fixtures, cues and sequences it
      holds, so without this an undoable store write reads
      `plugin data → a1b2c3 → value`. Verify with the frontend's own tests.
- [x] 3.7 Verify within-gesture coalescing applies to plugin writes, and assert
      the real number rather than the plausible one: a plugin writing one *new*
      key ten times while handling a single call leaves **two** oplog rows — the
      create, which `fold_into_the_gesture` refuses to fold because every create
      shares the `<table>/__create` path, and one folded value write. Writing an
      *existing* key ten times leaves one. This is the property that keeps a
      plugin from filling the log, so it is asserted rather than assumed.

## 4. Station-scoped storage

- [x] 4.1 Add `infra/plugins/station_store.rs`: a SQLite file at
      `<config-dir>/the-pult/plugin-data.db`, overridable by `PULT_PLUGIN_DATA`,
      keyed by `(plugin_id, store, key)`. Verify a station that cannot open the
      file logs and carries on with the stores reading empty, following
      `preferences.rs`'s contract.
- [x] 4.2 Implement the station-scoped half of the host interface against it.
      Verify with a two-station test that a station-scoped write reaches no peer
      and appears nowhere in the showfile.
- [x] 4.3 Verify station-scoped data is independent of the open show, by writing
      under one show, opening another, and reading it back.

## 5. Limits and lifetime

- [x] 5.1 Enforce the key-count and byte ceilings in `set`, before the write.
      Verify a write past either limit fails naming the limit and leaves the
      store byte-identical.
- [x] 5.2 Verify data outlives its plugin: remove a plugin, reinstall it, and
      assert it reads back what it stored; then run a different version under the
      same plugin id and assert the same, since a row is keyed by plugin id,
      store and key with no version in it.
- [x] 5.3 Surface data belonging to no installed plugin in the Plugins panel,
      grouped by plugin id, with a delete behind the Edit lock. Verify in the
      running app with `scripts/demo.sh`.

## 6. Making it usable

- [x] 6.1 Add a typed `store` wrapper to `plugins/sdk` so an author writes
      `store::get::<T>("prefs", "provider")` rather than assembling JSON. Verify
      with the SDK's own tests.
- [x] 6.2 Remember the operator's chosen provider and model in
      `plugins/natural-language-control`, in a station-scoped store, and verify
      the plugin reads them back after a restart and falls back to its manifest
      config when the store is empty. Deliberately not the grammar cache
      `command-line` could have kept: derived data is cheap to rebuild and a
      stale copy is worse than none, so an example built on it would argue
      against its own feature.
- [x] 6.3 Update `docs/PLUGINS.md` with the stores chapter: the two scopes, what
      each is for, the limits, that data outlives the plugin, when a store should
      declare its writes undoable and why the default is not, and — where stores
      are introduced, not in a footnote — that a credential belongs in the
      environment passthrough or station configuration and never in a store.
- [x] 6.4 Update `CLAUDE.md`'s WASM plugins section and add a roadmap entry
      recording three decisions: `scope` versus `lifecycle` and why the word
      changed, attribution as the switch that makes undo a per-store choice with
      nothing in `pult-schema` learning what a plugin is, and the derived entity
      id that makes a key mean one row across stations.
