# Plugin datastores — tasks

Each group is meant to end in its own commit with `cargo test`, the plugins
workspace's own tests, and a zero-warning build passing.

## 1. The contract

- [ ] 1.1 Add a `store` interface to `wit/pult-plugin.wit` — `get`, `set`,
      `delete`, `list`, each taking the store id — and import it in the `plugin`
      world. Bump the package to `0.2.0`. Verify the host bindings generate and
      the workspace still builds.
- [ ] 1.2 Replace `API_VERSION` with `SUPPORTED_API_VERSIONS = ["0.1", "0.2"]`
      in `manifest.rs`, keeping the error naming what the station speaks. Verify
      with tests that a `0.1` manifest and a `0.2` manifest both pass validation
      and a `9.9` one is refused with both versions in the message.
- [ ] 1.3 Verify a component built against `0.1` still instantiates against the
      `0.2` host, by loading an unmodified reference plugin in
      `crates/pult-backend/tests/plugins.rs`.

## 2. Declaring stores

- [ ] 2.1 Parse `[[stores]]` in the manifest — `id`, `scope`, optional
      `max_keys` and `max_bytes`. Verify with tests that a duplicate store id, an
      unknown scope, and a limit above the default are each refused by name.
- [ ] 2.2 Carry the declared stores into `PluginCtx` at instance start, so the
      host resolves a store from the manifest rather than from anything the guest
      passes. Verify a call naming an undeclared store fails before any read or
      write.

## 3. Show-scoped storage

- [ ] 3.1 Add a `PluginDatum` entity to `pult-schema` with
      `#[pult(table = "plugin_data")]` and PERSISTED `plugin_id`, `store`, `key`,
      `value`. Verify the generated migration gains the table and that
      `engine/mod.rs` needed no edit.
- [ ] 3.2 Run `cargo run -p pult-codegen -- generate` and verify the TypeScript,
      the proxy and the migration are the only changed generated files.
- [ ] 3.3 Implement the show-scoped half of the host interface through the
      engine's ordinary write path, so the write is attributed to the caller the
      way `data.set` already is. Verify a plugin's write replicates to a peer
      station in the integration tests.
- [ ] 3.4 Exclude `plugin_data` paths from `Operation::is_undoable` and from the
      History panel's filter. Verify with a test that an operator's undo after a
      plugin write takes back the operator's own last change.
- [ ] 3.5 Verify within-gesture coalescing still applies to plugin writes: a
      plugin writing one key ten times while handling a single call leaves one
      oplog row, not ten. This is the property that keeps a plugin from filling
      the log, so it is asserted rather than assumed.

## 4. Station-scoped storage

- [ ] 4.1 Add `infra/plugins/station_store.rs`: a SQLite file at
      `<config-dir>/the-pult/plugin-data.db`, overridable by `PULT_PLUGIN_DATA`,
      keyed by `(plugin_id, store, key)`. Verify a station that cannot open the
      file logs and carries on with the stores reading empty, following
      `preferences.rs`'s contract.
- [ ] 4.2 Implement the station-scoped half of the host interface against it.
      Verify with a two-station test that a station-scoped write reaches no peer
      and appears nowhere in the showfile.
- [ ] 4.3 Verify station-scoped data is independent of the open show, by writing
      under one show, opening another, and reading it back.

## 5. Limits and lifetime

- [ ] 5.1 Enforce the key-count and byte ceilings in `set`, before the write.
      Verify a write past either limit fails naming the limit and leaves the
      store byte-identical.
- [ ] 5.2 Verify data outlives its plugin: remove a plugin, reinstall it, and
      assert it reads back what it stored; then replace its bundle with a
      different version under the same plugin id and assert the same.
- [ ] 5.3 Surface data belonging to no installed plugin in the Plugins panel,
      grouped by plugin id, with a delete behind the Edit lock. Verify in the
      running app with `scripts/demo.sh`.

## 6. Making it usable

- [ ] 6.1 Add a typed `store` wrapper to `plugins/sdk` so an author writes
      `store::get::<T>("cache", "grammar")` rather than assembling JSON. Verify
      with the SDK's own tests.
- [ ] 6.2 Cache the derived grammar in `plugins/command-line` in a
      station-scoped store, and verify the plugin still behaves identically when
      the cache is cold, warm, and stale after a schema change — the last is the
      one that matters, since a stale grammar would be worse than none.
- [ ] 6.3 Update `docs/PLUGINS.md` with the stores chapter: the two scopes, what
      each is for, the limits, that data outlives the plugin, and — where stores
      are introduced, not in a footnote — that a credential belongs in the
      environment passthrough or station configuration and never in a store.
- [ ] 6.4 Update `CLAUDE.md`'s WASM plugins section and add a roadmap entry
      recording the `scope` versus `lifecycle` decision and why the word changed.
