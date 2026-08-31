# Plugin distribution — tasks

Each group is meant to end in its own commit with `cargo test`,
`npm --prefix frontend run check` and a zero-warning build passing.

## 1. The roster in the schema

- [x] 1.1 Add a `PluginPackage` type in `crates/pult-schema/src/types/plugin.rs`
      with `#[derive(PultSchema)]` and `#[pult(table = "plugin_packages")]`,
      fields `plugin_id`, `name`, `version`, `api`, `sha256`, `enabled`,
      `stage`, `config`, all PERSISTED. Verify with a lifecycle test asserting
      the row survives a save/load round trip, alongside the existing ones.
- [x] 1.2 Run `cargo run -p pult-codegen -- generate` and verify the generated
      migration gains a `plugin_packages` table with one column per PERSISTED
      field, that `frontend/src/lib/generated/` picks up the type and the proxy
      path, and that no file outside `pult-schema` and the generated output
      needed a hand edit — the point task 2 established.
- [x] 1.3 Add a `stage` enum (`Setup` / `Runtime` / `Both`, default `Both`) and
      verify a roster row deserialises with the field absent, so a row written
      by an older build still reads.

## 2. Bundles in the asset store

- [x] 2.1 Turn `ACCEPTED` in `infra/assets.rs` into a per-mime table of
      `(mime, max_bytes)` and add `application/vnd.pult.plugin+zip`. Verify the
      existing image tests still pass unchanged and a new test rejects a bundle
      over its own ceiling.
- [x] 2.2 Serve bundle-mime assets with `Content-Disposition: attachment` and
      verify by asserting the header on a bundle response and its absence on an
      image response.
- [x] 2.3 Write `bundle.rs`: read a zip from bytes, validate and extract to a
      target directory. Verify with hostile archives as well as good ones — an
      absolute entry path, a `..` traversal, a symlink entry, an entry count
      over the cap, and an uncompressed total over the cap must each be refused
      by name, and a well-formed bundle must yield a directory
      `PluginManifest::parse` accepts.

## 3. Reconciling what runs against what the show says

- [x] 3.1 Give `PluginManager` a subscription to `plugin_packages/**` and a
      `RosterChanged` message, and verify with a test that a write to the
      collection reaches the manager without it awaiting guest code.
- [x] 3.2 Implement the diff keyed by `(plugin_id, sha256)` — start, stop,
      replace on digest change, and **publish only** when just `name` or
      `stage` changed. Verify with a test that renaming a package does not
      restart the running plugin (task 9's lesson, asserted rather than hoped).
- [x] 3.3 Unpack into `<config-dir>/the-pult/plugin-cache/<sha256>/` on demand,
      reusing a directory that already exists. Verify a second show carrying the
      same digest unpacks nothing and starts from the cache.
- [x] 3.4 Exclude the cache directory from the watcher and verify a started
      carried plugin does not immediately reload itself.
- [x] 3.5 Add `PluginStatus::Fetching` and fetch missing bytes from peers on a
      separate task via `assets::fetch_from_peers`, messaging the manager when
      they land. Verify the event loop stays responsive during a fetch by
      answering a call to another plugin while one is in flight.
- [x] 3.6 Report a bundle no peer has, an unreadable archive, an invalid
      manifest and an API-version mismatch as that plugin's failure with the
      reason, and verify the show still opens and other roster entries still run.

## 4. Directory plugins keep winning

- [ ] 4.1 Remove ids found by `discover()` from the reconcile set and add an
      `overridden_by_disk` flag to `PluginInfo`. Verify with a test where the
      roster and a plugin directory carry the same id: the directory copy runs,
      the flag is set, and editing the file on disk still reloads it.

## 5. Layered configuration

- [ ] 5.1 Add `plugins: BTreeMap<String, toml::Table>` to `Preferences` and
      verify a preferences file without the key still loads (the "never fails"
      contract) and that a round trip preserves it.
- [ ] 5.2 Compose manifest → show → station with the existing `deep_merge` and
      verify with a test that a station overriding one key keeps the show's
      value for its siblings.
- [ ] 5.3 Restart a plugin when its composed configuration changes, on the
      stations the changed layer affects only. Verify a show-level edit restarts
      it everywhere, a station-level edit restarts it on that station alone, and
      neither disturbs another plugin.

## 6. Installing and removing

- [ ] 6.1 Add `POST /api/plugins` taking a multipart bundle: validate the
      manifest before storing anything, store the asset, then write or replace
      the roster row for that `plugin_id`. Verify a bad bundle is rejected with
      its reason and leaves neither an asset nor a row, and that installing an
      id already present replaces rather than duplicates.
- [ ] 6.2 Verify removal through the ordinary entity delete path is undoable,
      attributed and replicated, by asserting a removed package comes back on
      Ctrl-Z.

## 7. The Plugins panel

- [ ] 7.1 Read the roster and the LOCAL `plugins` state together: a row per
      package with its state on this station, its version, its digest, and the
      disk-override note. Verify in the running app with `scripts/demo.sh`.
- [ ] 7.2 Show each package's declared permissions — data access, commands,
      HTTP hosts, env names — as plain text on the row.
- [ ] 7.3 Put install, remove, enable/disable and show-level configuration
      behind the panel's Edit lock, per task 23's rule; station-level
      configuration sits beside them and is marked as this machine's.
- [ ] 7.4 Group rows by `stage` and verify a setup-only plugin is grouped but
      still runs.

## 8. End to end, and the documentation

- [ ] 8.1 Extend `crates/pult-backend/tests/plugins.rs` with a two-station test:
      install a bundle on one station, assert the other fetches it by digest and
      runs it, then remove it and assert both stop.
- [ ] 8.2 Add a test that a peer answering with the wrong bytes is refused and
      the plugin is not run.
- [ ] 8.3 Teach `scripts/build-plugins.sh` to emit bundles beside the
      components, and verify the reference plugins install from the bundles it
      produces.
- [ ] 8.4 Update `docs/PLUGINS.md`: packaging, installing, where configuration
      lives and which layer wins, and the trust assumption stated plainly.
- [ ] 8.5 Update `CLAUDE.md`'s WASM plugins section and add a roadmap entry
      recording what was decided and what it cost.
