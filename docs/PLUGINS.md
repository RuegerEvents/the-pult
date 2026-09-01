# Writing a plugin

A plugin is a WebAssembly component the station loads from a directory: a
`pult-plugin.toml` describing it, a `.wasm` built against the SDK, and —
if it ships its own panel UI — some JavaScript under `assets/`. The two
plugins in `plugins/` are the reference: `command-line` shows almost every
capability, `natural-language-control` shows depending on another plugin and
talking to the network.

The contract is `wit/pult-plugin.wit`. Everything a plugin can see or do is
in that file; this page is the tour.

## The shortest possible plugin

```
my-plugin/
  pult-plugin.toml
  Cargo.toml
  src/lib.rs
```

`pult-plugin.toml`:

```toml
[plugin]
id = "my-plugin"           # lowercase letters, digits, hyphens
name = "My Plugin"
version = "0.1.0"
api = "1.0"                # the WIT version you built against
wasm = "my_plugin.wasm"    # beside this file
```

`Cargo.toml` (in the `plugins/` workspace, add it to `members`; elsewhere,
use a path dependency on `plugins/sdk`):

```toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
pult-plugin-sdk = { path = "../sdk" }
serde_json = "1"
```

`src/lib.rs`:

```rust
use pult_plugin_sdk::{self as sdk, PultPlugin};
use serde_json::Value;

struct MyPlugin;

impl PultPlugin for MyPlugin {
    fn init(_config: Value) -> Result<Self, String> {
        sdk::log_info!("hello from a plugin");
        Ok(MyPlugin)
    }

    fn handle(&mut self, method: &str, _args: Value, _ctx: Value) -> Result<Value, String> {
        Err(format!("no method called {method:?}"))
    }
}

sdk::plugin_main!(MyPlugin);
```

Build and run:

```
scripts/build-plugins.sh                 # or: cargo build --target wasm32-wasip2
cargo run -p pult-backend -- --plugins path/to/my-plugin
```

`--plugins` takes one plugin's directory or a directory of them, and can be
repeated. The demo loads the reference plugins with `scripts/demo.sh
--plugins`.

## The development loop

The station watches every loaded plugin's directory. Rebuild the `.wasm`,
change the manifest, or edit `config.toml`, and the plugin is stopped and
started fresh — same as the node simulator applying a config. Nothing
survives a reload; `init` runs again. `scripts/build-plugins.sh --watch`
rebuilds on save, which together with the station's watcher is hot reload
end to end.

What the runtime is doing is visible in two places: the station log
(`[plugin:<id>] …`) and the LOCAL `plugins` state every frontend gets — a
failed plugin shows up in its panels with the reason.

A guest call that runs longer than five seconds is trapped and the plugin
marked failed; the next call to it (after a cooldown) starts it fresh.

## What the host offers

All bindings are on `pult_plugin_sdk`; JSON crosses as `serde_json::Value`.

| | |
|---|---|
| `host::get(path)` | read a path, `Null` where nothing is |
| `host::set(path, value)` | write a path (`__create` / `__delete` for collections) |
| `host::call(method, args)` | entity commands (`"sequences.goNext"`) and station RPCs (`"session.join"`) |
| `host::subscribe(pattern)` | slash-joined pattern; updates arrive at `on_update` |
| `host::call_plugin(id, method, args)` | another plugin's `handle` — see dependencies |
| `host::entities()` / `commands()` / `rpcs()` | the schema, at runtime |
| `store::get` / `set` / `delete` / `keys` / `subscribe` | what your plugin remembers — see below |
| `sdk::log_info!` (`_warn`, `_error`, `_debug`) | the station log, prefixed with your id |

Paths are spelled the way the WebSocket spells them: `["sequences",
"<uuid>", "name"]`, indices as strings, `__create`/`__delete` sentinels.
Lifecycle is derived from the path by the host, same as for every other
writer.

**Say how far, not where, when that is what you mean.** A third sentinel,
`__by`, writes a *change* instead of a destination: `["cues", id,
"fade_in_ms", "__by"]` with `1500` is a second and a half more than
whatever that is now. The station resolves it against what it holds at the
moment it applies the write — so two plugins nudging one value both get
their nudge, where two that each read a value and computed a destination
would leave only one of them heard.

The programmer has its own form, because the ordinary case is that nobody
is holding the key yet:

```rust
host::set(&["programmer_values", "__by"], &json!({
    "fixtureId": id, "parameterKind": "Intensity", "by": 0.1,
}))?;
```

That takes the key if the programmer does not have it, starting from what
playback is showing. Numeric fields and parameter values only; a switch or
a line of text refuses by name, as does a parameter running a shape.

Resolution happens before anything is recorded, so the history, the
showfile and every peer see the number. Nothing downstream of your call
knows a relative write happened.

**Never enumerate the schema in a plugin.** `host::entities()` and
`host::commands()` serve the live registries — entity tables, field
lifecycles, command argument schemas, doc strings. A plugin that drives
itself from introspection (as `command-line` does for its entire grammar)
stays correct when the data model grows.

### Writes are attributed

When a call arrives with an operator's context, everything the plugin writes
during that call is attributed to that operator and gathered into one
gesture — so a command that fans out over a selection is one Ctrl-Z, exactly
like a drag.

## What a plugin remembers

`lifecycle::init` runs on every start and every hot reload, and nothing else
survives one. A store is where anything that should outlive that goes.

There are two kinds, and the difference is not a detail:

```toml
# Belongs to the show: in the showfile, on every console in the session,
# in the backup.
[[stores]]
id = "macros"
scope = "show"

# Belongs to this machine: never replicated, never written into a showfile.
[[stores]]
id = "prefs"
scope = "station"
```

```rust
sdk::store::set("prefs", "provider", &"ollama")?;
let provider: Option<String> = sdk::store::get("prefs", "provider")?;
let saved: Vec<String> = sdk::store::keys("macros", "opening/")?;
sdk::store::delete("macros", "opening/old")?;
```

**Declaring the store is the permission.** There is no key under
`[permissions]`: you can address no store you did not declare and no other
plugin's, because the host works out where the data lives from your plugin id
and the store's, never from anything you pass. What an operator wants to know
is whether a plugin keeps data and whether that data goes into their showfile,
and the `[[stores]]` block answers exactly that where the rest of the
permissions are read.

**Never put a credential in a store.** A show-scoped store replicates to every
station and lands in every backup. Keys belong in the environment passthrough
your manifest declares by name, or in the station's `preferences.toml` — the
two homes that do not travel with the show. The host cannot tell a token from
a string, so this one is on you.

**Choose the scope by asking who the data is true of.** A macro an operator
wrote is true of the show and should be on the second console. A cached
grammar, a last-used tab, the model that happens to be installed here is true
of this machine — put it in a showfile and it replicates to a console it is
wrong for. `natural-language-control` is the worked example: it remembers
which model this console talks to, station-scoped, and deliberately not a
cache of anything derived (derived data is cheap to rebuild and a stale copy
is worse than none).

### Stores are bounded

1,000 keys and 1 MB each. A manifest may ask for less and not for more:

```toml
[[stores]]
id = "prefs"
scope = "station"
max_keys = 8
```

A write past either limit fails naming the limit and leaves the store exactly
as it was. Lowering a ceiling is worth doing — it tells the operator reading
your manifest that your cache is small on purpose.

### Your data outlives you

Removing a plugin does **not** delete its stores, so one removed by mistake
and put back finds its work where it left it. Data belonging to no installed
plugin shows up in the Plugins panel under *Left behind*, grouped by the
plugin id that wrote it, where an operator can clear it out deliberately.

### Undo, and when a write is the operator's

By default a store write is *yours*, not the operator's: it is not undoable
and does not appear in the History panel. That is what you want for
bookkeeping — an operator who presses Ctrl-Z after running your command means
their own last edit, not your cache.

When the operator *asked* you to save something — a macro, a snippet — say so:

```toml
[[stores]]
id = "macros"
scope = "show"
undoable = true
```

Then the write is attributed to whoever asked, undoes like any other edit, and
appears in the history named by your plugin, the store and the key. A write
with no operator behind it — from a timer, or from `init` — is never undoable
whatever the store says, because there is nobody to attribute it to.

Station-scoped stores cannot be `undoable`: that data never reaches the show's
history, so a manifest saying otherwise is refused rather than ignored.

### If you cache it, watch it

A show-scoped store is show data, and show data moves without you. An operator
presses Ctrl-Z on an `undoable` store and the key you wrote is gone; the same
plugin on the console next to yours writes that key and your copy is stale.
Neither reaches a value you are holding in a struct field, and if you never
read the key again you never find out.

```rust
fn init(_config: Value) -> Result<Self, String> {
    Ok(Self { token: sdk::store::subscribe("macros"), macros: HashMap::new() })
}

fn on_update(&mut self, token: u64, path: &[String], value: Value) {
    if token == self.token {
        // path is [store, key]; a null value means the key was forgotten.
        let key = path[1].clone();
        if value.is_null() { self.macros.remove(&key); } else { self.macros.insert(key, value); }
    }
}
```

Your own writes come back too — the subscription is on the show, not on your
calls. Ignoring your own echo is easy (you know what you just wrote); hearing
an undo any other way is not possible at all, which is why it works this way
round. Tokens share one space with `host::subscribe`, so compare the token
rather than guessing from the path.

Subscribing to a station-scoped store returns `0` and never fires: it is this
machine's file and you are the only writer, so there is nothing to be told.

## Permissions

Everything is off until the manifest asks:

```toml
[permissions]
data = "read-write"     # "none" (default) | "read" | "read-write"
commands = true         # host::call
http = ["localhost"]    # outbound hosts; empty means no network
env = ["MY_API_KEY"]    # environment variables passed through, by name
```

The host enforces these, not the guest. Outbound HTTP (over `wasi:http` —
the `waki` crate is the easy client; see `natural-language-control/src/http.rs`)
is checked per request against the `http` list: an entry allows its host on
any port, `"host:port"` allows exactly that.

## Dependencies between plugins

```toml
[dependencies]
plugins = ["command-line"]
```

Declared dependencies load first and are the only plugins `call_plugin` may
name. The current call's context travels along automatically, and call
cycles are refused by the host. A dependency can be reloaded under you;
nothing breaks, because calls go by id through the runtime, never through a
held reference.

## Configuration

Three layers, merged key by key, each beating the one before it:

1. the manifest's `[config]` table — your defaults, and a `config.toml`
   beside it while you are developing (that one is the operator's, and it is
   never put into a bundle),
2. the **show's** configuration for your plugin, on its roster row,
3. **this station's** configuration, under `[plugins.<your-id>]` in the
   console's `preferences.toml`.

The composed result reaches `init` as JSON, and changing any layer restarts
the plugin — you are handed your configuration once and never again.

Station last is deliberate. What a station legitimately overrides is what is
true of that machine and no other, and a show cannot know which console has
the local model on it or which one holds a key.

**A credential does not go in a store or in show-level configuration.** Both
travel with the showfile, into every backup and onto every peer. The two
right homes are the station's own `preferences.toml` and the environment
passthrough your manifest declares by name.

## UI: two ways to have a panel

**A surface** is a panel the console draws for you — no JavaScript in your
plugin at all:

```toml
[[surfaces]]
id = "console"
kind = "console"      # prompt + scrollback + completions + help
title = "My Console"
```

`kind = "console"` is a command line; `kind = "bar"` is a one-line input
with a transcript. Both drive your plugin over three methods arriving at
`handle`, with JSON shapes typed in `pult_plugin_sdk::surface`:

- `surface.exec` — `{ "line": "…" }` → `ExecResponse`: output lines, an
  optional error with a byte span into the line (the surface draws the
  caret), and optional effects. `{"selection": {"fixtureIds": […]}}`
  changes the caller's selection to those fixtures;
  `{"selection": {"query": …}}` changes it to a `SelectionQuery`, which
  keeps following the rig — that is what selecting a saved group hands
  back, and why it stays live rather than freezing into a list.
- `surface.complete` — `{ "line", "cursor" }` → items plus `replaceFrom`.
  An item with empty `text` is a hint, shown but never inserted.
- `surface.help` — `{ "topic"? }` → `{ "text" }`.

`ctx` on these calls is the browser's context: `{ "selection": [fixture
uuids], "userId": … }`. Selection lives in the browser, not the show — two
operators hold different fixtures — so it comes to you per call and goes
back as an effect.

**A web-component panel** is JavaScript your plugin ships:

```toml
[[panels]]
id = "monitor"
title = "My Monitor"
element = "my-element"     # the custom element the script defines
script = "panel.js"        # served from assets/ by the station
fills = true               # the panel does its own scrolling
```

The console loads `/api/plugins/<id>/assets/panel.js` as a module and mounts
the element with a `pult` property: `call(method, args)` to your plugin,
`get(path)` and `subscribe(pattern, cb)` to the show.
`plugins/command-line/assets/panel.js` is the worked example.

Either way the panel appears in every browser's `+` menu as soon as the
plugin loads, and layouts saved with it open fine on consoles without it.

## Shipping it: bundles

A plugin directory is how you develop. A **bundle** is how a plugin reaches
anybody else: one zip holding your `pult-plugin.toml` at its root, your
component, and any `assets/` a panel loads.

```
scripts/build-plugins.sh --bundle     # → plugins/dist/<id>.pult-plugin.zip
```

Install it from the console's **Plugins** panel, or with a request:

```
curl -X POST http://<station>:7700/api/plugins \
     -H 'content-type: application/vnd.pult.plugin+zip' \
     --data-binary @plugins/dist/my-plugin.pult-plugin.zip
```

**A show carries its plugins.** Installing writes a row naming the bundle by
the sha256 of the whole archive; the bytes go in the same content-addressed
store stage plans use. Every other station in the session sees the row,
fetches the bundle from whoever has it, verifies the digest, unpacks it into
a cache keyed by that digest, and runs it. One install equips the rig.

The digest covers the whole archive rather than the component, so a changed
panel script is a different bundle — half a plugin cannot be versioned while
the other half is not.

Installing a plugin id the show already carries is an **upgrade**: same row,
new bundle, and whatever the operator had chosen — switched off, configured
— is kept.

**A plugin directory still wins.** On a station started with `--plugins`, a
plugin found on disk overrides the show's copy of the same id, on that
station only, and still hot reloads. Otherwise editing a plugin on a console
joined to a session would silently run the show's build instead. The Plugins
panel says when this is happening.

### What installing means

Opening a showfile runs the plugins it carries. That makes a showfile
trusted input in the way a binary somebody hands you is trusted input: there
is no approval step, and the manifest's permissions are granted by opening
the file.

What still bounds a plugin is everything in **Permissions** above — the
wasmtime sandbox, the five-second epoch deadline, data access, the outbound
HTTP allowlist, and env passthrough by name. What it does not bound is a
plugin that legitimately has `data = "read-write"` and a host allowlist:
that one can move show data off the network wherever it opens.

The Plugins panel therefore prints each plugin's permissions in words beside
it, so what a show is asking for is readable without unzipping anything.

## Versions

The WIT package version (`pult:plugin@1.1.0`) is the API version, and the
manifest's `api` says what you built against.

**It is a floor, not a match.** A station runs your plugin when its major
matches yours and its minor is at least yours — so a plugin built against
`1.0` keeps running on a `1.4` station that has since added interfaces you
never import. The other direction cannot work: a station has nothing to
satisfy an import that did not exist when it was built, and says so by name.

This is why the package is `1.x` and not `0.x`. A component's imports are
stamped with the package version (`pult:plugin/data@1.0.0`) and link only
because wasmtime resolves them semver-compatibly — and under semver a `0.x`
minor bump is a *breaking* change, so `0.1` and `0.2` are unrelated and every
import fails to resolve. At `0.x` the contract could never have grown without
stranding every plugin already in a showfile.
`scripts/check-api-compat.sh` checks the claim rather than trusting it.

The host currently links wasmtime 48 (WASI 0.2); guests are ordinary
`wasm32-wasip2` cdylibs — Rust ≥ 1.82 emits a component directly.

## Checking your work

```
cd plugins && cargo test          # the pure crates, on the host target
scripts/build-plugins.sh          # the components
scripts/build-plugins.sh --bundle # and the zips to install
cargo test -p pult-backend --test plugins   # a real station loading them
cargo test -p pult-backend --test roster    # the show carrying them
```

The backend test skips itself when the components are not built, so it never
fails a machine that merely lacks the wasm target.
