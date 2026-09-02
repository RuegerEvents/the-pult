# the-pult

Distributed lighting console system.

## Planning lives in the roadmap

`docs/ROADMAP.md` is the whole of it: the numbered tasks are finished work with
the decisions and the traps recorded, and *What is next* at the end is the
candidate list, each entry carrying the questions it has to answer before it can
be built. A new feature starts by reading its entry there and updating it, and
ends as the next numbered task.

## Architecture

- **`crates/pult-macros`** — `#[derive(PultSchema)]` proc macro. Generates `PultEntity` impl, `{T}Patch`, `{T}Create`, `{T}Accessor` from annotated Rust structs.
- **`crates/pult-render`** — The evaluator: what a parameter is doing, worked out from what is driving it and a moment. `serde` and `uuid` and nothing else — no clock, no OS — because it is compiled twice.
- **`crates/pult-render-wasm`** — The same crate for a page: `wasm32-unknown-unknown` + `wasm-bindgen`, built by `scripts/build-evaluator.sh` into `frontend/src/lib/evaluator/`.
- **`crates/pult-schema`** — Data model + path accessor infrastructure. All entity types live here. Source of truth for the WebSocket protocol and sync protocol.
- **`crates/pult-backend`** — A station, as a library and a binary. Axum WebSocket server, SQLite showfiles, peer sync (mDNS + TCP), the WASM plugin runtime (`infra/plugins/`), fixture connectors. `pult_backend::start(Config)` brings a whole station up and is what both the binary and the desktop app call.
- **`crates/pult-gui`** — The console as a Tauri desktop app. A window around `pult_backend::start`, pointed at the server it just started.
- **`tools/pult-codegen`** — CLI that triggers ts-rs TypeScript export and writes `frontend/src/lib/generated/`.
- **`tools/openhaunt-node-sim`** — The node side of the OpenHaunt protocol, in software. A node *is* a `NodeConfig` — identity, module descriptor, and the ports it describes — so a JSON config file is the whole of what makes one node different from another. `configs/` holds worked examples of modules that are not in the catalogue at all.
- **`tools/openhaunt-node-sim-gui`** — A Tauri window onto a simulated node: buttons for its inputs, and an editor for its config. Talks to the sim over Tauri IPC, so nothing about the OpenHaunt protocol changes to accommodate a debug UI. Applying a config stops the node and starts a new one in its place, without the window closing.
- **`plugins/`** — WASM plugins: its own cargo workspace (own lockfile; guests build to `wasm32-wasip2`, which does not belong in the console's dependency graph). `sdk/` is what plugins are written against; `command-line` and `natural-language-control` are the reference plugins and the worked examples for `docs/PLUGINS.md`.
- **`frontend/`** — SvelteKit static-adapter frontend. Built into the binaries that serve it.

## A driven value is evaluated; a sensed one is stored

**Nothing keeps what a parameter is doing.** The console keeps what is *driving* it —
`live_fades` and `live_effects` on the fixture, anchored in console milliseconds, the
`programmer_values` entry over them, the home value beneath — and every consumer works
out a number for the moment and at the rate it needs one. That is the whole of the
model, and it is why the engine has no tick.

The arithmetic is **one implementation compiled twice**: `pult-render` natively for the
station, its connectors and its plugins, and `pult-render-wasm` for the browser. There
is no TypeScript translation of it, deliberately — easings, curves, step lists, spread,
phase, direction, width, master rates, priority and home fallback are a large enough
surface that two of it would drift, and the visible form of that drift is the screen
disagreeing with the lamps. What holds the two *compilations* together is
`testdata/driven-values.json`, read by `crates/pult-render-wasm/tests/corpus.rs` and by
`frontend/src/lib/evaluator.test.ts`.

```
scripts/build-evaluator.sh          # the browser's copy → frontend/src/lib/evaluator/
```

Three consequences worth holding on to.

**A landed fade stays.** `live_fades` is not a list of what is in flight; it is the
record of where each parameter got to, because nothing else remembers. A fade that has
arrived is a constant function of time, and evaluating it gives exactly the number it
landed on.

**Connectors own their rate.** `OutputPlugin::send(patch, changed, now_ms)` is handed
what is driving the rig and a moment; the DMX family draws at 40 Hz while anything is
moving and drops to its keep-alive when nothing is, and an OpenHaunt node that can run
a fade itself is told once. The engine pushes when the *show* changes — a cue taken, a
fixture patched, a fader grabbed — and says nothing at all in between. A three-second
fade over two thousand fixtures is one push.

**The browser has to know the station's clock.** The objects are anchored in console
time, so a page evaluating against an unadjusted `Date.now()` runs every fade out by
however wrong its own clock is, silently. `frontend/src/lib/ws/clock.ts` estimates the
offset the way a round-trip time is estimated, maintains it rather than taking it once,
and — this is the rule that matters — **says nothing until it has one**: `consoleNow()`
answers `null` and panels show a gap rather than a plausible wrong number.

What is *sensed* is the exception and stays state. `Fixture::sensed_values` holds what a
device reported — a contact, a temperature, a humidity — because the console cannot work
that out: it was told it. Driven outputs are functions; sensed inputs are state.

## Lifecycle System

Every field in the data model has one of three lifecycles:
- `LOCAL` — stays on this backend node; synced to connected frontends but NOT to peer backends, not persisted.
- `SYNCED` — broadcast to all peer backends AND all connected frontends; not persisted.
- `PERSISTED` — written to SQLite AND replicated to peers AND frontends.

Frontend-only UI state (selections, hover, expanded rows) lives in Svelte stores — not in the schema.

## Path-Based Access API

Everything is accessed via a path-proxy:
- Rust backend: `data.sequences().nth(5).cues().nth(3).fade_time().set(4.0).await?`
- TypeScript frontend: `await data.sequences[5].cues[3].fadeTime.set(4)`

## Design Principle: pult-schema is the single source of truth

All entity types live in `pult-schema`. When the data model changes, **no other location should need a manual update**. Specifically:

- Do not enumerate entity types or collection names in the sync protocol, snapshot structures, or codec logic. Use serde-derived serialization of `ShowState` as a whole.
- Adding a new entity collection needs **no** edit outside `pult-schema`. `ShowState` holds entities as JSON keyed by table and `ShowState::frontend_paths()` is derived from the `EntityMeta` registry, so a `#[derive(PultSchema)]` type with a `table` is readable, writable, persisted, synced and visible to the frontend with nothing added to `engine/mod.rs`.
- **`frontend/src/lib/ws/data.ts` is generated by pult-codegen** from `EntityMeta` + `CommandRegistration` inventories. Never hand-edit it. It is NOT the maintenance point for the frontend proxy types — those follow from the schema automatically.
- **Commands** (`#[pult_command]`) carry their TypeScript arg signature via `args_ts` in `CommandRegistration`. Set it with `#[pult_command(args = "{ foo: string }")]` in the schema crate. No TypeScript file needs to be updated manually.

## After Changing Schema Types

Run the TypeScript codegen after any change to types or commands in `pult-schema`:
```
cargo run -p pult-codegen -- generate
```

## The frontend is served by the backend

The SvelteKit build is embedded with `rust-embed` (`api/spa.rs`) and served as the
router's fallback, so **one binary is the whole console**. Two things follow:

- **The page and the socket share an origin.** `frontend/src/lib/ws/endpoint.ts` is
  the only place that decides where the backend is, and the answer is
  `window.location` — `?port=` survives only as a way to name a second station on
  the same host. `GET /api/config` answers the rest (station id, version).
- **Any browser on the network is a console.** A tablet at `http://<station>:7700`
  gets the same app the desktop window does.

In dev, Vite proxies `/ws`, `/assets` and `/api` through to `PULT_BACKEND`
(default `http://localhost:7700`), so dev is same-origin too.

A debug build reads `frontend/build` off the disk; a release build embeds it. If
the directory is missing, `build.rs` leaves a placeholder page behind so a fresh
clone still compiles.

## WASM plugins

The plugin API is `wit/pult-plugin.wit` plus runtime introspection — never a
list. A plugin learns entities, commands and station RPCs from the
`introspection` host functions (served from the `EntityMeta` /
`CommandRegistration` inventories and `api/rpcs.rs`); nothing about the
schema is enumerated in a plugin, the WIT, or the runtime, so the data model
grows without touching any of them. Station RPCs live in
`crates/pult-backend/src/api/rpcs.rs` — adding one there makes it callable
from the WebSocket, callable from plugins, and visible to introspection at
once.

```
scripts/build-plugins.sh                     # plugins/ workspace → components
cargo run -p pult-backend -- --plugins plugins   # load them; edits hot-reload
cargo test -p pult-backend --test plugins    # a real station loading them
cargo test -p pult-backend --test roster     # a show carrying them
```

`docs/PLUGINS.md` is the author guide. Plugin panels reach the frontend as
LOCAL `plugins` state; the workspace reads the merged `allPanels` store
(`frontend/src/lib/stores/plugins.ts`), so no frontend file lists plugin
panels either.

**A show carries its plugins.** `plugin_packages` is a PERSISTED collection
naming each bundle by the sha256 of its zip; the bytes live in the same
content-addressed asset store as stage plans, so a station that lacks one
fetches it from a peer and verifies it. Every station reconciles what it runs
against that roster while the show is up — one install equips the rig.

```
scripts/build-plugins.sh --bundle    # → plugins/dist/<id>.pult-plugin.zip
curl -X POST http://localhost:7700/api/plugins \
     -H 'content-type: application/vnd.pult.plugin+zip' \
     --data-binary @plugins/dist/command-line.pult-plugin.zip
```

Two consequences worth holding on to. **Opening a showfile runs its plugins**
— a deliberate choice, bounded by the sandbox and the manifest permissions
and nothing else; the Plugins panel prints those permissions in words.
And **a `--plugins` directory beats the show** for that id on that station,
so the dev loop is unchanged and a console editing a plugin says so.

Plugin configuration is three layers, most specific winning: the manifest's
`[config]`, the show's roster row, then `[plugins.<id>]` in the station's
`preferences.toml`. Credentials belong in the last one or in env passthrough,
never in the first two — those travel with the showfile.

**A plugin can remember things.** A manifest declares `[[stores]]`, each
`scope = "show"` (a PERSISTED `plugin_data` entity, so replication and the
showfile come free) or `scope = "station"` (SQLite beside `preferences.toml`;
`Config::plugin_data` moves it, and `PULT_PLUGIN_DATA` is the fallback for a
station started from a shell — an env var is one per *process*, so two stations
inside one program have to be told separately). Declaring the store is the permission — the
host derives the location from `(plugin_id, store)`, so no guest can spell a
name that reaches another plugin's data. A row's id is a UUIDv5 over
`(plugin_id, store, key)`, which is what makes two stations writing one key
write one row. Removing a plugin does not delete its stores; what is left over
shows up in the Plugins panel under *Left behind*.

A store write is **not** undoable and not in the History panel unless the store
says `undoable = true`. Both come from whether the host attributes the write, so
neither `Operation::is_undoable` nor the oplog's SQL knows what a plugin is.

And a plugin can be **told** when a show-scoped store changed under it —
`store.subscribe(store)`, delivered through the existing `lifecycle.on-update`
as `[store, key]`. Built on the engine's broadcast rather than a hook in the
store's own write path, deliberately: a hook sees only this station's guest
writing, where the broadcast also sees an undo and a peer's copy of the same
plugin, which are what a plugin holding a value in memory cannot otherwise
learn about. A station-scoped store hands back a dead token, having nothing to
report.

The WIT package is `pult:plugin@1.1.0` and a manifest's `api` is a **floor**:
same major, station's minor at least the plugin's. It cannot be `0.x` — a
component's imports carry the package version, and under semver a `0.x` minor
bump is breaking, so every import would fail to resolve. `scripts/check-api-compat.sh`
checks that a plugin built against an older minor still runs.

```
cargo test -p pult-backend --test stores   # what a plugin remembers
scripts/check-api-compat.sh                # an older plugin still runs here
```

## Running

```
cargo run -p pult-codegen -- generate     # after any schema change
scripts/build-evaluator.sh                # the browser's copy of the evaluator
npm --prefix frontend run build           # once; the backend serves this
cargo run -p pult-backend                 # then http://localhost:7700
```

As a desktop app — the same station, in a window, still serving the network:

```
cargo run -p pult-gui
```

For frontend work, Vite with hot reload beside a running backend:

```
cd frontend && npm run dev
```

The simulated OpenHaunt node has a window too. Its panel is built separately —
there is no `beforeBuildCommand`, because Tauri runs that from a directory it
infers rather than from the one the config sits in:

```
npm --prefix tools/openhaunt-node-sim-gui/ui install
npm --prefix tools/openhaunt-node-sim-gui/ui run build
cargo run -p openhaunt-node-sim-gui -- --module relay --serial 4d5e6f
```

A node the catalogue has never heard of is a config file rather than a code
change — the console builds its fixture type from what the node says, so there is
nothing to teach it:

```
cargo run -p openhaunt-node-sim -- --config tools/openhaunt-node-sim/configs/fog-machine.json
cargo run -p openhaunt-node-sim-gui -- --config tools/openhaunt-node-sim/configs/mirror.json
cargo run -p openhaunt-node-sim -- --module env --write-config mine.json   # somewhere to start
```

The frontend opens onto a **tiled workspace** rather than a sidebar and tabs. Panels
live in a tree of splits and tab groups: drag a tab to a tile's edge to divide it or
to its middle to stack it, drag the gutters to resize, and pick a layout from the menu
in the top bar. Presets are built in; *Save as…* writes an arrangement into the show
as a `layouts` row. Which layout this browser is looking at is kept in `localStorage`,
not in the show.

The **`values` panel** is the programmer: it sets fixture parameters into a shared
SYNCED `programmer_values` buffer that takes priority over playback until the values
are cleared or stored into a cue. Programming also happens in the `plan` and `rig`
panels, where a selected head can be aimed by dragging where its beam lands.

**A write can say how far instead of where.** A `__by` sentinel on a path — beside
`__create` and `__delete` — is a change rather than a destination, and the station
resolves it against what it holds at the moment it applies it. That happens at the
top of the engine's `Set` arm, above the oplog and the sync layer, so history, the
showfile and every peer only ever see the absolute; a peer adding a delta to its own
copy would diverge. `["programmer_values", "__by"]` with
`{fixtureId, parameterKind, by}` is the programmer's form, and takes the key if
nothing is holding it. `at +10` in the command line is this, and it is why the
natural-language plugin can answer "a bit darker" with no access to the show.

**A parameter rests somewhere when nothing is driving it.** Its **home value**: the
fixture's own `home_values` override where it has one, and its type's `default_value`
— derived from what the node said about its own ports — otherwise. Resolved in
`crates/pult-schema/src/types/fixture.rs` and nowhere else; the browser never works
one out for itself, and asks with a third path verb, `["programmer_values", "__home"]`
with `{fixtureId, parameterKind?}` — no kind means every output parameter, enumerated
by the station. So `home` in the command line, like `at +10`, is a destination a
caller can ask for without being able to read the rig.

Two acts reach it. **Taking a sequence off** (`Sequence::off`) puts back everything
its cues capture that no other live sequence captures and the programmer is not
holding — read from the show rather than remembered, so a station that joined at the
interval releases exactly what one that ran the act releases. And **sending a
selection home**, which is a programmer act and so replicates, undoes and clears like
any other. `Show::home_fade_ms` says how long either takes, seeded from a station
preference the way `history_depth` is. Consequence worth knowing: **Go at the last cue
stays there** rather than wrapping to no active cue, because "off" has to be a state
playback can tell apart from "ran out of cues".

And the verb backwards: `["fixtures", "__set_home"]` with the same
`{fixtureId, parameterKind?}` makes where a parameter rests be wherever it is now,
evaluated at the instant it is asked. Which is how a house light's actually gets set —
aim it, look at it, keep it — and a verb rather than a write to `home_values` for the
reason `__home` is one, sharpened by this change: working out what a parameter is doing
means holding the whole stack and evaluating it, so a caller able to act would otherwise
have to be a caller able to read the rig. One write of the whole map, so a fixture is
one Ctrl-Z.

**And a read, for asking rather than acting.** `parameter.value` is a station RPC —
`{fixtureId, parameterKind?}`, answering a map keyed by parameter key — for the plugin
or command line that wants to know what a light is doing and cannot evaluate for
itself. An RPC rather than a command, deliberately: asking what a lamp is at must not
write anybody's history.

**A cue fades two ways.** `fade_in_ms` is what a parameter takes going up and
`fade_out_ms` what it takes coming down, on the cue and per capture, the capture
winning. Zero out means "this cue does not split its fade" rather than "snap", so a
show that never sets one runs exactly as it did. Only values with an order to be on
can be going down — a colour has three and a relay none, and those take the in time
rather than have the console guess a ranking.

**A selection is a question about the rig**, not a list of ids — "every mover on the
downstage truss" stays true after somebody patches a fifth one. What is selected
*right now* is one operator's and lives in a Svelte store; a **saved group** is the
show's, a PERSISTED `groups` row holding the query itself. Recalling one takes on the
question, so a fixture patched afterwards joins it, and `group 3` in the command line
leaves exactly what clicking the group leaves.

Which means `SelectionQuery` is evaluated twice — `crates/pult-schema/src/types/group.rs`
for the station and plugins, `frontend/src/lib/selection.ts` for the browser, because
a cone being dragged re-evaluates per frame and cannot be a round trip. The two are
held together by `testdata/selection-queries.json`, which both test suites read; a new
term or order needs a case there or it is only half implemented. A station resolves a
group through the `selection.resolve` RPC — a read, so deliberately not a command:
asking what is in a group must not write history.

Or all of it at once — backend, two simulated OpenHaunt nodes, and the frontend —
with a seeded show and Ctrl-C to stop everything:

```
scripts/demo.sh              # a fresh show with something to look at
scripts/demo.sh --keep       # carry on from the last run
scripts/demo.sh --two        # a second station, joined to the first's session
scripts/demo.sh --help       # the other options
```

It works in `.demo/`, which is gitignored, so it never touches a real showfile.
Logs for each component land there too.

**A show can be a size instead of a scene.** `--size small` is the hand-made demo
and the default; `big` and `huge` add a generated rig on top — 500 or 2000 fixtures
across as many universes as they need, a cue stack over several sequences each
capturing a slice of the rig, and effects left running so the station has something
moving in it. They exist to be measured rather than looked at, and `--measure` is how:
it seeds, drives every sequence to a cue with an effect on it, seeds an Art-Net output
at loopback so there is a frame to measure at all, and prints what one cost — then
stops, with no sims and no dev server, because both would be taking the CPU being
measured. `--release` with it, or the figures mean nothing next to anybody else's.

```
scripts/demo.sh --size huge                        # 2000 fixtures, 300 cues, three plans
scripts/demo.sh --measure --release --size huge    # seed it, read it, print it, stop
```

**A station knows what its own output frames cost** and publishes them in the
`stations` row beside `cpu_percent`, so the figures `--measure` prints are the ones the
Stations panel shows and the ones a peer sees. **One entry per connector**, because
their rates and their costs are their own: Art-Net drawing at 40 Hz beside an OpenHaunt
node that was told about a fade once are not two samples of one number.

Two figures per connector rather than one, because a frame has two halves that scale
differently — evaluating, and putting it on the wire. That is not a hypothetical: a
two-figure split is what showed that evaluating was 0.2% of what a *tick* used to cost,
and finding what the other 99.8% was still needed a counter added by hand.

What it is *not*: what the process costs. That is `cpu_percent`, in the same row, which
is why anything printing one prints the other. And a connector that emitted nothing in a
window reports **nothing rather than zero**, since zero would read as "instant" when the
truth is that nothing happened.

## Releases

Tagging `v*` builds all four products for Linux x86_64 and aarch64, macOS arm64
and Windows. Two things are worth knowing before changing that workflow:

- `scripts/package-binaries.sh` decides what is in a release archive, and can be
  run directly (`VERSION=0.0.1 TARGET=aarch64-apple-darwin scripts/package-binaries.sh`).
  It stages files by name on purpose: archiving cargo's output directory instead
  sweeps in the dep-info file beside the binary.
- The version comes from `[workspace.package]`, the tag has to match it, and
  `CHANGELOG.md` needs a `## <version>` heading — plain, not bracketed, which is
  the only form the release action matches.

## Testing

```
cargo test                     # the workspace's default members
cd plugins && cargo test       # the plugins workspace's pure crates (CLI grammar)
cd frontend && npm test        # vitest, pure helpers and the wasm evaluator
cd frontend && npm run check   # svelte-check
```

Not `--workspace`: `pult-gui` and `openhaunt-node-sim-gui` are workspace members so
that one lockfile covers everything and CI can build with `--locked`, but they are
excluded from `default-members` so that a plain `cargo build` does not need
webkit2gtk on the machine. Build them by name (`-p pult-gui`).

`pult-render-wasm` *is* a default member, despite being the browser's half: its tests
are the corpus that holds the two compilations of the evaluator to each other, and a
guard outside the default suite is a guard nobody runs. Its vitest half needs
`scripts/build-evaluator.sh` to have been run, and says so loudly rather than passing
quietly when it has not.

Both the Rust build and `svelte-check` are kept at zero warnings, so a new one is
visible rather than buried.
