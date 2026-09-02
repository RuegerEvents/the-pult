# Roadmap

State of the system, what has been built, and what to work on next. Reconstructed from the code on 2026-08-25, then reconciled against [SPEC.md](SPEC.md).

The spec is the product. This is the build order for getting there, and the gap is still wide: what exists is a synchronised show-state engine with cues, playback, output, and an event system. The spec's 3D programmer, geometric selections, phasers, and waveform timecode are all still ahead.

## Where the system stands

| Layer | State |
|---|---|
| `pult-schema` + `pult-macros` | Working. The derive macro generates entity meta, patch/create types, accessors, and SQL. All 30 workspace tests live here. |
| `pult-codegen` | Working and idempotent. TypeScript types, the `data.ts` proxy, and the SQL migration all come from the `EntityMeta` and `CommandRegistration` inventories. |
| Showfile (SQLite) | Working. Load and save are registry-driven and enumerate no entity types. |
| WebSocket API | Working. Path-pattern subscribe, set, call, and broadcast fan-out. |
| Session discovery | Working. mDNS advertise and browse, create, join, leave. |
| Peer sync | Works and converges. Handshake, bidirectional catch-up from the oplog, live fan-out, heartbeat liveness and latency, vector-clock conflict resolution, and leader failover. Stations publish themselves and are visible in the UI. |
| Frontend | Working for show, session, sequences, cues, patch, the programmer, effects and speed masters. A tiled workspace of resizable panels replaced the sidebar and tabs; layouts are saved in the showfile. Panels that can change the show open read-only behind an Edit toggle and are sized for a finger. The typed proxy runs end to end. Vitest covers the pure helpers; components are untested. |
| Playback engine | Working, and no longer a tick. Playback decides *what is driving* each parameter — fades and effects anchored on the cue's `went_at` — and publishes the descriptions; nothing stores what they are worth. A pass happens when the show changes, so a fade in progress costs the engine nothing. |
| Output plugins | Working for Art-Net, sACN, and OpenHaunt nodes, several at once. Each holds the last patch it was pushed and draws its own frames out of it at its protocol's rate, evaluating rather than being handed values. Configured from the `outputs` collection and editable while the show is up, with per-output status and per-connector frame cost in the UI. Flags only seed an empty showfile. |
| Stage view | Working. A ground plan is uploaded, calibrated against something of known length, and fixtures are dragged onto it — then the same rig in 3D from front of house, beams and all. |
| Flows | Working. The spec's node graph, evaluated as a graph: sources, conditions, boolean logic, delays and actions, with live state on every node. Replaced `triggers`. |
| Devices / events | Working. OpenHaunt nodes are discovered over mDNS and adopted as fixtures; their inputs land in `sensed_values`; flows turn those into cues. A port that says it can trace a shape is handed one descriptor instead of forty messages a second. Tested end to end against `tools/openhaunt-node-sim` and, since task 22, against real firmware on an ESP32. |
| WASM plugins | Working. wasmtime component runtime with a WIT contract, permissions, hot reload, plugin-to-plugin calls and runtime introspection of the schema registries. Two reference plugins in `plugins/`: a command line (grammar built from introspection, console panel with completion and spans) and natural-language control (an LLM over the plugin's own gated HTTP, executing through the command line). Plugin UI is built-in surfaces or plugin-shipped web components. `docs/PLUGINS.md` is the author guide. |
| 3D programmer | Working in outline. A shared programmer buffer beats playback, and pan and tilt are puppeteered by grabbing a ring, an arc, or the beam spot on the floor — in the rig and on the plan. Effects are in, and a selection is a question about the rig rather than a list. |
| Selection | Working as a query over the rig: by type, name, sphere, box or the spec's radial cone, built up by adding, narrowing and removing, and ordered along an axis or outwards from a point. Re-evaluated as the rig changes, so a fixture patched under a live selection joins it. Queries cannot be saved as groups yet. |
| Effects | Working. One primitive covers a shape and a step list, running from the programmer or a cue, at its own rate or a speed master's. Rendered identically on every station from replicated state, and handed to a node that can trace it for itself. No amplitude fade into one yet. |
| Undo / history | Working, per person and across their clients, and by gesture rather than by write — one drag is one Ctrl-Z. The oplog carries the author, the previous value and what an operation reverses, so undo is a query over it rather than a stack — which is what lets a tablet take back what the desk did. A History panel shows what everyone changed. Nothing prunes the log. |
| Timecode | Not started. `FollowMode::Timecode` exists and nothing produces one. |
| Distribution | Working. The frontend is built into the binaries that serve it, the console and the simulator each have a Tauri desktop app, and tagging builds all four for Linux x86_64 and aarch64, macOS arm64 and Windows. Nothing is signed and nothing auto-updates. |

## Task list

Ordered. Each task is meant to end in its own commit with tests passing.

### 1. Cover the engine with tests (done)

Nothing outside `pult-schema` had a single test. The path dispatch and lifecycle routing in `ShowEngine::apply_set` is the most intricate code in the backend and the least protected, and task 2 rewrites it. Write the tests first so the rewrite has something to fail against.

Covers set, get, create, delete, command dispatch, LOCAL vs SYNCED vs PERSISTED routing, ordering, broadcasts, the snapshot round trip, and peer operations.

Writing them found a bug: deleting a sequence or a fixture silently did nothing and reported success, because the generic field-patch arm in `apply_set` came before the `__delete` arm and serde dropped `"__delete"` as an unknown field.

### 2. Drive engine dispatch from the registry (done)

Was the most important task. `CLAUDE.md` promises that adding an entity collection means editing `ShowState` and nothing else. `engine/mod.rs` breaks that promise in four places:

- `ShowState::get_by_path` hand-matches every collection key
- `ShowEngine::apply_set` has per-entity arms for patch, `__create`, and `__delete`
- `ShowEngine::apply_entity_result` matches on the entity key again
- `ShowState::FRONTEND_PATHS` is a hand-kept list

The drift is already visible. `FixtureType` has a `#[pult(table = "fixture_types")]` and a full schema, but the backend never mentions it. It is not in `ShowState`, never loads, and no path reaches it. It exists only as a TypeScript file.

`ShowState` now holds entities as JSON keyed by table and routes every read and write through `EntityMeta`. No entity type is named in `engine/mod.rs`, and `FixtureType` works with no code written for it.

Two more breaks of the same rule turned up and are fixed. Accessor path keys were camelCase while the wire is snake_case, so every field write through the Rust accessor API silently did nothing. And `ShowDataRoot`'s collection accessors were hand-written, listed three of the five tables, and pointed `fixture_types` at a table that has never existed.

One thing left open, worth doing, not blocking:

- Commands do not write to SQLite. That is right for `go_next`, which moves SYNCED playback state and should not touch the disk on every Go press, but a command that changes a PERSISTED field would not survive a restart.

### 3. Playback engine (done)

`model::playback` fades captures into `Fixture::live_values`, marks the played cue active, and fires `FollowAfter` cues. It is a pure state machine driven by the engine's own tick, so its tests run a four-second fade in microseconds.

`Timecode` follows still do nothing. The spec wants waveform-based timecode with beat grids rather than plain SMPTE, so this should wait for that design rather than get a stopgap.

`Show::is_running` is still unused. Its meaning is a product decision, not a code one.

### 4. Output plugin layer (done for Art-Net)

The spec is explicit that this is a plugin layer, not a DMX-shaped core: output plugins translate high-level data into whatever protocol a fixture speaks, DMX among them, and network-based communication is preferred over DMX-centric workflows.

So the first piece is the trait and the registry, not Art-Net. An output plugin takes fixture state and a `FixtureType`, and emits protocol frames. Art-Net is then the first implementation of it, mapping `live_values` through `FixtureType::parameters` to channels at 40 Hz. sACN after that.

`OutputPlugin` and `OutputManager` are in place, `connectors::dmx` renders fixtures and their types into universes, and `connectors::artnet` puts ArtDmx on the wire. Art-Net skips universes whose 512 bytes have not changed and refreshes every 800 ms, so an idle rig stays off the network.

sACN followed in task 11, as predicted: `connectors::sacn` is one file beside `artnet.rs`, and the dedup and refresh bookkeeping both protocols need moved into `connectors::dmx` rather than being written twice.

Still to do here:

- The spec wants fixtures to preload upcoming playback data, which means handing a plugin a description of what is coming rather than only the current frame. Nothing here does that yet.

### 5. Fixture and patch UI (done)

Fixture types with their parameters, and a fixture table with name, type, universe, address, position, and live values. New fixtures land at the next free address, and channel overlaps within a universe are highlighted.

`Fixture::position` went in at the same time, which also turned up two things worth knowing: an added column needed a migration path for existing showfiles, and reading an optional column panicked on NULL. Both fixed.

Left open here:

- ~~`subscribeDeep` re-fetches a whole collection on any change beneath it.~~ Done in `caed1bc`: an update names the path that changed, so the local copy is patched rather than re-read, and only a cold start or an entity the client has never seen costs a round trip. What was left after that was not the network but the work done per delivery — `clashingFixtures` compared every fixture against every other one, on every tick of every fade. It sweeps per universe now: a 500-fixture rig went from 32 ms to 0.9 ms per second of ticks, and stopped growing quadratically.
- Position can only be set to the origin from the UI. Editing coordinates, and the axial form, wait for the 3D view.

### 6. Sync catch-up and conflict handling (done)

Peer identity in `HelloAck`, heartbeat liveness with a 16-second timeout, vector-clock conflict resolution, catch-up from the oplog in both directions, and leader failover. Twenty-three tests run real nodes over real TCP.

Leader election needs no messages: the leader publishes the membership, and every survivor removes the lost node from the same list and picks the lowest remaining id. Lowest id rather than freshest state, because catch-up runs both ways on connect, so whoever leads ends up with everything either side had.

One bug found later, worth writing down because it cost an afternoon and looked like something else entirely. `read_frame` reads a length and then a body, which makes it not cancel-safe, and it sat directly in `run_peer_loop`'s `select!`. When a heartbeat tick won that race the half-finished read was dropped, the bytes it had already taken went with it, and every frame after that landed at the wrong offset — so the connection died and never came back. Rare, load-dependent, and indistinguishable from flaky tests. Reading now happens in its own task and the loop selects on a channel, which is cancel-safe.

Two latent deadlocks turned up while chasing it, both the same shape: a bounded channel awaited from inside the loop that drains it. Dialling a peer no longer runs its handshake inside the event loop, and `fan_out` no longer waits for a peer's outbox. A peer whose outbox fills is now dropped rather than waited for — it reconnects and catches up from the oplog, which is better than the alternative of quietly not sending it a write.

What this does not do:

- The election assumes every survivor received the same membership list. A node that joined and never heard a membership update, or one partitioned from the rest, can pick differently. Real partition tolerance means a consensus protocol, which is a design decision rather than a coding one.
- Followers do not automatically reconnect to the new leader. They find it again through mDNS once it advertises, which is a delay rather than a break.
- The oplog is never pruned. It grows for the life of a showfile.

### 7. Housekeeping (done)

`cargo build --workspace` and `npm run check` are both at zero warnings, so a new one is visible rather than buried. `crates/pult-schema/bindings/` is untracked now.

The one exception is ts-rs printing `failed to parse serde attribute` for the generated Patch structs' `skip_serializing_if`. That is inside ts-rs.

### 8. WASM plugins (done)

Nothing else depends on it, and the plugin API should be designed against a system that already plays back cues and drives output.

That deferral paid for itself. The API landed as one WIT package (`wit/pult-plugin.wit`, wasmtime 48 + wasm32-wasip2 components) with JSON on every edge, so no entity type appears in the contract — a plugin learns the schema from introspection host functions that serve the `EntityMeta` and `CommandRegistration` inventories at runtime, plus a station-RPC table that `handle_local_call` was refactored into (`api/rpcs.rs`) so those six calls stopped being invisible. `PluginManager` mirrors `OutputManager`: an actor, per-plugin instance actors under it, LOCAL `plugins` state telling every frontend what runs. Manifest permissions gate everything — data access, commands, an outbound-HTTP host allowlist enforced in the host's `send_request`, env passthrough by name. Epoch interruption traps a guest that runs five seconds; a changed file reloads its plugin while the show is up, which is the node-sim's stop-and-start-fresh applied to plugins.

Two rules fell out of debugging rather than design. The manager never awaits guest code — a dependency's `init` calling back through the manager deadlocked the first version; now instances init on their own tasks, readiness is a message, and calls to a still-loading dependency queue in its mailbox, which makes mailbox creation order the whole of load sequencing. And plugin writes carry the caller's identity: the surface context's user attributes every write of a call, gathered under one gesture, so a CLI command that fans over a selection is one Ctrl-Z.

Plugin UI is both ways at once: *surfaces* the console draws (a command-line panel, a one-line bar — the plugin implements `surface.exec`/`complete`/`help`, pure Rust, spans and completions included) and *web components* the plugin ships as JavaScript served from its directory. Panel ids are `plugin:<id>:<panel>`, so saved layouts survive a missing plugin the way they survive an unknown panel.

The reference plugins double as the examples: `plugins/command-line` derives its entire grammar from introspection (and reimplements the frontend's derived programmer-entry id, pinned in both test suites so the two FNV implementations can only drift loudly); `plugins/natural-language-control` depends on it, fetches its grammar for the prompt, and runs whatever the model answers back through it — the command line is the safety boundary, and the model gets no other hands. `scripts/build-plugins.sh` builds the separate `plugins/` workspace; `scripts/demo.sh --plugins` loads everything; `docs/PLUGINS.md` teaches it.

Left open: no resource limits beyond the epoch deadline (memory is unbounded), plugins are not release artifacts, and the `is_public` flag on commands still has no reader.

### 9. Output configuration in the web UI (done)

Outputs are `--artnet` flags read once at startup. Nothing in the data model knows an output exists, so an operator cannot see where the show is going, let alone change it without restarting the backend.

Add a PERSISTED `outputs` collection — `OutputConfig { id, name, kind: Artnet | Sacn | OpenHaunt, target, universes, enabled, node_id: Option<NodeId> }` — and build the `OutputManager`'s plugins from it instead of from the command line. `OutputManager` then has to add and remove plugins when the collection changes, which it cannot do today: it takes its `Vec<Box<dyn OutputPlugin>>` once at construction. The flag stays, as a way to seed one output into an empty showfile.

An *Outputs* tab lists them, adds them, and enables or disables them, with per-plugin LOCAL status beside each: last send, frames per second, error count. That status is the reason to do this at all — right now a mistyped Art-Net address is silent.

`node_id` says which station runs the plugin. It is the first ownership concept in the system, and what task 10 displays.

sACN is not part of this. It lands earlier, in task 11, as a sibling of `artnet.rs`.

All of that is in. `OutputManager` reconciles against the collection rather than taking its plugins once, so an output can be re-addressed while the show is up — and it keeps the socket when only the name changed, because rebuilding it resets the dedup cache and puts a redundant frame on the wire for a label edit. Status is LOCAL and measured over a one-second window; `interval` fires immediately, which the first version divided by, reporting a thousand frames a second.

`node_id` turned out to matter immediately rather than in task 10. `outputs` is PERSISTED, so it replicates — and without an owner every station would send the same universes, which is two copies on the wire. The UI fills in the local station and *Every station* is a deliberate choice.

The flags stay as a way to seed an empty showfile, and refuse to add anything to a show that already has outputs.

Left open: nothing decides what happens to an output whose station leaves the session. It simply stops, which is right for a duplicate and wrong for the only path to the rig. That belongs with task 10's partitioning question rather than here.

### 10. Station view (done)

What exists today is thinner than it looks:

- ~~`NodeId` is generated fresh on every process start.~~ Done, and not optional once task 9 landed: an output names the station that sends it, so a fresh id per start meant a saved output belonged to nobody and silently stopped sending. `infra::identity` records it beside the showfile — beside rather than inside, because copying a showfile must not clone a station's identity.
- `SyncManager` knows its `members` and `peers`, but they are reachable only through test-only commands. Nothing publishes them.
- Heartbeats go out every 5 s with a 16 s timeout, and the ack carries a `seq` that is never matched against a send time. Liveness is known; latency is not.
- There are no system stats of any kind.
- Every node computes every fixture. `playback_tick` runs the same fades everywhere, which is what makes output deterministic without extra messages, and also means "which node drives what" has no answer yet.

Add a SYNCED `stations` collection that each node publishes about itself — `Station { node_id, hostname, is_leader, sync_addr, cpu_percent, mem_used, mem_total, uptime_s, output_plugins, computes_fixtures, last_seen }`, refreshed every couple of seconds via `sysinfo`. Round-trip time comes from matching `Heartbeat { seq }` to `HeartbeatAck { seq }` in `infra/sync/peer.rs`, published as a LOCAL `peers` list by the node that measured it, because latency is a property of a link rather than of a station. And persist `NodeId` in the showfile so a station keeps its identity across restarts.

A *Stations* tab then shows who leads, what the latency to each peer is, cpu and memory per node, which output plugins run where, and which fixtures each node computes.

That last column is honest but dull for now: every node computes every fixture, so it is all-or-nothing until parameter computation is partitioned. Partitioning it is the follow-up — the interesting version is a node driving only the fixtures on the outputs it owns, with a defined answer for what happens when that node drops out.

All of that is in. `Station` is SYNCED, and each node writes only its own row, so the collection converges with nobody arbitrating it: a station is the only authority on its own memory usage. It is reported as `computes_fixtures` over `total_fixtures` rather than a flag, so the number already says something true and will say something more interesting once the work is split.

Latency is LOCAL, not SYNCED, and that turned out to be the right call for a reason worth recording: measured across a two-node session the same link read 0.22 ms from one end and 0.33 ms from the other. They are two different paths. A single shared number would have been an average of two things nobody asked about.

Pruning is the leader's job alone — two nodes deleting each other's rows on different schedules is a fight rather than a cleanup. A station goes grey in the UI after three missed reports and its row is removed after thirty seconds of silence.

Persisting `NodeId` came first and separately, because task 9 had already made it urgent: an output names the station that sends it, and a fresh id per start meant a saved output belonged to nobody and silently stopped sending.

Left open: nothing partitions fixture computation, so the fixture column is all-or-nothing; and a station's row says what it is doing rather than what it should be doing, so there is still no answer for an output whose station has left.

### 11. OpenHaunt nodes and the event system (done)

Covers the spec's *Event-Based Control & Automation*: sensors and switches drive playback, and the console drives things that are not lights.

[OpenHaunt/node](https://github.com/OpenHaunt/node) is a PoE-powered modular I/O node — one carrier, one plug-in module — advertising itself over mDNS as `_openhaunt._tcp.local` with an HTTP/JSON control API and MQTT for events. Its guiding principle is that a node is discovered, not configured. There is no firmware yet, so this is built against the written protocol, and what the-pult assumes gets fed back into those docs.

What this delivers:

- Discovery. Nodes appear in a *Devices* panel with their module type, capabilities, and a mains warning where the descriptor says the module switches mains. Adopting one creates a fixture.
- Fixtures that are not DMX. `Fixture::universe`/`dmx_address` become `FixtureAddress::{ Dmx, OpenHaunt }`, and `ParameterDefinition` gains a direction and a binding, so a parameter can be an *input* on a *port* rather than an output on a DMX channel. A contact closure and a temperature reading are then ordinary fixture parameters in `live_values`, replicated like everything else.
- An MQTT broker embedded in the leader (rumqttd). On adoption the-pult POSTs its own broker address to the node, so nothing external has to be installed and the "discovered, not configured" principle holds. Only the leader drives devices, gated the same way `GoNext` already is.
- Input to trigger to cue. A PERSISTED `triggers` collection with a source, a condition, an action and a delay, evaluated by a pure state machine in the engine's own tick next to playback.
- Output to relays, LED strips, and OLED displays through an `OpenHauntOutput` plugin, plus sACN unicast for the DMX gateway module — which is task 4's remaining sACN work, done here because the gateway needs it.
- `tools/openhaunt-node-sim`, a simulator implementing the node side of the protocol, so the whole path is covered by tests without hardware on the bench.

All of that is in. `FixtureAddress` and `ParameterDirection`/`ParameterBinding` went in first, with the two migration paths they needed — a hand-written `Deserialize` for the JSON column and `showfile::upgrades` for the real ones. `DeviceManager` browses, adopts, and drives; `SetLiveValue` merges an input inside the engine actor and replicates it; `model::triggers` evaluates the rules in the engine's own tick beside playback.

**A node describes itself, and this console carries no module catalogue.** The first cut of this had a table in `types::openhaunt` from module id to fixture type, which meant a node newer than the console — or anybody else's module — was a thing the console had to be taught. That is backwards from the principle the project shares with [SHIFTY](https://github.com/oshifty/vision): only the device knows what it is and how it is best controlled. So `GET /api/v1/info` carries a `ports` list in E1.73 UDR's vocabulary — `access`, `dataType`, `unit`, `minimum`/`maximum`/`default`, plus a small `class` hint — and `openhaunt::fixture_type_from` turns that into a fixture type at adoption. A `class` the console recognises becomes the kind it has semantics for; anything else becomes `ParameterKind::Named`, so a device can declare a parameter nobody here has a word for and it still patches, still programs, still stores into a cue. A node that describes nothing is refused rather than guessed at, and the fixture type's id is derived from the description, so two identical modules share one type and firmware that changes its ports gets a fresh one instead of silently mismatching.

What it leaves open:

- ~~The node-graph UI.~~ Task 12.
- Per-pixel WS2812. A strip is one colour and one brightness.
- OSC and MIDI as trigger sources. `TriggerSource` is an enum with one variant so they can be added beside `Parameter` without touching anything else.
- RDM, which the gateway module's `caps` advertises and nothing here uses.
- A running fade and a `SetParameter` trigger writing the same key: last writer wins, and it is the fade, because it writes on every tick. Documented rather than solved — deciding what *should* happen is a product question.
- The broker is started once per process and never stopped. A node adopted by a previous leader is re-configured on promotion, but a follower keeps a broker it started while it was leading.

Assumptions made about the protocol, to be fed back into the OpenHaunt docs since there is no firmware to check them against: `GET /api/v1/info` carries `ports` — one entry per terminal with `port`, `name`, `access`, `dataType`, and optionally `unit`, `minimum`/`maximum`/`default` and `class` — and `dmx` only on a node that forwards a universe; `/api/v1/config` takes `{ mqtt: { broker }, dmx?: { protocol: "sacn", universe } }` and persists it, and accepts `dmx` only where the description declared one; the mains flag is descriptor bit 6, reachable through the same `/info` as `module.flags` (a `mains=1` TXT key lets the panel warn without the round trip); input events are `{ state, edge, ts }` and readings `{ value, unit, ts }` on the same `input/<n>` topic, with `unit` a UDR name; output payloads follow the port's `dataType` rather than the module — `{ state }` for a boolean, `{ value }` for a number, `{ r, g, b }` for a colour, `{ text }` for a string, and no `{ brightness }` anywhere; `POST /api/v1/state` takes `{ outputs: { "<n>": payload } }`; `status` is the literal `online`/`offline`, retained, with `offline` as the will; health is `{ uptime_s, temp_c, poe_class, errors }`; the DMX module lists `sacn` in `caps` and listens on unicast 5568 for its configured universe; and TXT `sn` matches the instance short serial, one module per node. `mod` and `caps` in the TXT record stay an inventory shortlist — filtering and the mains warning — never the basis for control.

### 12. The node graph (done)

Covers the spec's *Node-Based Workflow*: "visually connect triggers, events, playback
actions and automation logic".

`triggers` is gone. A rule was a source, a condition, a delay and an action in a row,
which is a four-node chain — so the graph replaced it rather than sitting beside it,
and `showfile::upgrades` redraws every existing trigger as a flow on the next open.
One evaluator, one meaning, and the thing a row could not say — two contacts into an
`And` — is now drawable.

`flows`, `flow_nodes` and `flow_edges` are three PERSISTED collections rather than two
`Vec`s on one entity, so dragging a node patches one row instead of rewriting the
graph and two operators moving two different nodes both keep their work. Adding all
three needed no edit in `engine/mod.rs`: task 2's registry-driven dispatch held.

The design decision everything else follows from is that a port carries either a
**level** or a **pulse**. A level stays put; a pulse is an instant. Sources emit
levels, `And`/`Or`/`Not` combine them, and a `Condition` is the only thing that turns
one into the other — by noticing a *change*. That asymmetry is what stops a warm room
firing a cue forty times a second, and having it in the type system means the editor
refuses the connection rather than the evaluator refusing the graph.

Node `active` is SYNCED, so a graph lights up as signals pass through it on every
console watching. That is the reason to draw it at all: a diagram that shows its own
state is an instrument.

Two things turned up while building it:

- A button press is a write to `last_fired_at`, not a message. That fell out of
  wanting a press to work from a tablet: the leader is the only node that fires
  anything, and a replicated field change already reaches it by the path everything
  else takes.
- A `Watch` node offered every driven parameter and could never fire for any of
  them, because playback applied fades with LOCAL lifecycle and never queued an
  input event. A cue's own output is show state like any other, so fades now reach
  the flow tick — gated by the set of parameters some `Watch` actually names, since
  this runs at 40 Hz per fixture in a fade.

Left open:

- Nothing decides what a cycle *means*. It terminates rather than hanging, which is
  right for a drawing mistake and not an answer for a graph that wants feedback.
- An action still loses to a running fade writing the same key. Task 11 documented
  it; drawing it does not settle it.

### 13. The stage view (done)

`Fixture::position` went in with the patch in task 5 and could only ever be set to
the origin, so the rig had coordinates nobody could enter and nothing drew. Two
views now read them, and one of them puts them there.

**Assets.** The first bytes in a system that was otherwise all fields. A ground plan
is a few megabytes, and putting it in the oplog would put a copy in every operation,
every snapshot and every catch-up — so `assets` is a blob table beside the show,
addressed by the sha256 of its own contents, with `POST /assets` and
`GET /assets/{sha}` beside the WebSocket that was until now the entire HTTP surface.

Content addressing carries more weight than it looks. The id *is* the check, so a
station that has never seen a plan fetches it from one that has and verifies what
came back; the same drawing uploaded twice is stored once; and the response can be
cached for ever because its contents cannot change. `Station` gained `http_addr` so
there is somewhere to fetch from, and a relayed request is answered locally or not at
all, so a ring of consoles cannot forward one request round between them.

PDFs never reach the backend. Page one is rasterised in the browser, which keeps a
document engine out of both stage views and off the main bundle.

**The plan.** `StagePlan` is the drawing plus the two numbers that make it a map:
where its top-left corner sits in the room, and how many metres one pixel covers. The
second comes from clicking two points and saying how far apart they really are, which
is the whole of calibration. Fixtures are dragged onto it and their live colour and
level fill the symbol, so the view is both where the rig is and what it is doing.

**Axes.** This is the first place in the system that had to commit: **Y up, X to the
right seen from front of house, Z downstage towards the audience.** Z is chosen
rather than inherited, and two things agree on it — a ground plan is drawn with the
audience at the bottom of the page, and the 3D camera looks up −Z. A plan therefore
lies on the floor with no flip anywhere.

**The rig in 3D.** Threlte over three.js, opening at the FOH perspective the spec
calls primary. Fixture bodies, a beam cone each, and a spot light so the floor shows
the state as well as the air. Both views read `lib/stage.ts`, so they cannot disagree
about where anything is.

Left open:

- Pan is taken as 540° about the way a fixture hangs, because `FixtureType` carries
  no real ranges. Right about which way a head is swinging and wrong in detail for
  any particular one.
- Nothing prunes an asset. A replaced plan's bytes stay in the showfile.
- The plan is one drawing on one plane. Sections, multiple decks and a plan per
  level are all the same schema and none of the UI.
- MVR and GDTF. `StagePlan` and the asset store are the two things an import needs,
  so nothing here is in its way.

### 14. The programmer and cue editing (done)

The console could play a cue but nobody could *make* one. `Cue.captures` was written
as `[]` at creation and never touched again, no control anywhere set a fixture value,
and the spec's §Programming — the programmer buffer, parking, the store menu,
puppeteering pan and tilt in 3D — was the part the roadmap called the biggest single
piece.

**The priority rule.** The decision everything else follows from: **for every
parameter the programmer holds, the programmer wins over playback**, until that value
is cleared or stored. This is the first explicit priority rule in the system, and it
is the standard one — MA and ETC both work this way.

What it does *not* do is stop the cue. Fades keep running underneath, and what they
would be showing is kept current as they do, so a value released or stored lands
where playback has got to rather than snapping back to where the cue was when the
operator grabbed it. That is the difference between an override and a freeze, and it
is what lets a look be built during a fade.

Anything else that writes a live value — a flow action, an input off a device — is
not fought with. It writes; the overlay notices on the next tick, treats what it
wrote as the new value underneath, and covers it again. The open question task 11
recorded ("an action loses to a running fade") is still open and still a product
question; what is settled is that neither of them beats the operator's hands.

**Where it lives.** `programmer_values` is a SYNCED collection — replicated to peers
and frontends, never persisted. A showfile that reopened asserting somebody's
half-finished look over playback would be a fault, not a feature. Entry ids are
*derived* from the fixture and the parameter key rather than minted, so two consoles
grabbing the same fader write the same row and converge instead of leaving two rows
that take turns reaching the output.

**Storing and editing.** A store menu shows what is about to be written — fixture,
parameter, value, one checkbox each, as the spec asks — with merge or replace, a new
cue or an existing one, and a *keep the programmer* option. Editing a cue is load,
tweak, Update: the cue is read into the buffer and taken, changed there, and written
back on Update. Not live editing — a cue that rewrote itself as a fader moved would
have no way back from a mistake, and would be doing it on every console at once.
`Show::editing_cue` is SYNCED so the second console shows the same banner rather than
quietly storing over the first one's work.

**Puppeteering.** The spec asks for pan and tilt by grabbing the axis, "just like it
would behave in real life". In 3D a selected head wears a pan ring, a tilt arc lying
in the plane it is currently panned to, and a disc where its beam lands on the floor;
dragging any of them projects the pointer ray onto the plane that axis turns in and
reads the angle off it. The plan view gets the same beam-spot handle, plus a level
ring, in world units. Both write through the programmer, never into `live_values`.

A ring is *turned*, not aimed: the axis moves by however far the pointer has gone
round since it took hold, from wherever the axis already was. Reading the absolute
angle instead — which is what the first version did — snapped the head to the pointer
the moment the ring was touched, which is not what taking hold of a yoke does. The
turn is added up one move at a time, each wrapped to the short way round, so a drag
that passes behind the fixture keeps counting instead of flipping.

The beam-spot handle is capped at a distance worth drawing. A beam near the
horizontal lands arbitrarily far away and one above it never lands at all, so the
honest answer is a point tens of metres off the plan — and a drag that flattened the
beam sent the handle off the screen mid-gesture, which read as the handle simply
coming away in your hand.
`interactivity()` had never been installed, so click-to-select in 3D had been dead
since the view was built; `OrbitControls` became `CameraControls` so that picking a
fixture can animate the camera to it.

Two things about the 3D view worth writing down because both cost an afternoon:

- The orbit controls and a gizmo hear the same `pointerdown` on the same element, and
  by the time any handler could call the camera off it has already begun to move. The
  press is now taken in the *capture* phase, before the event reaches the canvas at
  all, and raycasts the gizmos itself.
- An `<HTML>` overlay with `pointerEvents="auto"` takes pointer events over its whole
  layout box — which, when the panel is moved by a CSS transform, is not where the
  panel is drawn. An invisible rectangle below the quicksheet was eating every click
  on the beam spot. The wrapper is `none` now and the panel itself is `auto`.

And one about the plan view, worth writing down because the symptom pointed
nowhere near the cause. Clicking a fixture filled most of the room with a
white-and-blue rounded band. Nothing in the DOM was that size — every shape measured
a few pixels — and `elementsFromPoint` found nothing there at all, which is the clue:
it was not an element. It was Chrome's own focus ring on the fixture group, which is
focusable, drawn in the element's own coordinates. On a plan those are metres, so the
ring was several metres thick and shaped like the bounding box of the fixture's beam.
`svg :focus { outline: none }`, and keyboard focus is shown by the ring the symbol
already has.

Two things were changed while chasing it that are worth keeping on their own terms: a
full-turn level ring is drawn as two half-arcs, because an arc that ends where it
began does not say which circle it meant; and every stroke in that view is measured in
screen pixels through `vector-effect: non-scaling-stroke`, which is what a hairline
wanted rather than a dozen widths each computed as some fraction of the view.

**`u64` is not a `bigint` on this wire.** ts-rs maps every 64-bit integer to `bigint`,
which would be right if the wire could carry one; it is JSON, so a `u64` arrives as a
`number`. The declared type was a lie about every value that turns up, and
`error_count === 0n` is `false` for the `0` that does — so a working sACN output
reported itself unhealthy for ever. pult-codegen now rewrites `bigint` to `number` on
the way out, which is one place rather than an attribute to remember on every `u64`
somebody writes.

And one about the engine. A subscriber watching `show` never heard a field write to
it: patterns are matched against the path a write names, and `show/editing_cue` does
not match `show`. Collections already answered this by broadcasting the collection
after a create or a delete; singletons now do the same. Entities deliberately do not —
that path is `fixtures/<id>/live_values` at forty a second during a fade, and sending
the whole rig each time is what `subscribeDeep` exists to avoid.

Left open:

- **Every drag is an oplog row.** Writes are coalesced to one per animation frame per
  parameter and skipped when the value has not changed, which makes a fader drag tens
  of rows rather than hundreds. It is a reduction, not a fix: the oplog is still never
  pruned, and that is where this belongs.
- **Pan and tilt travel are constants** — 540° and 270° about the way a fixture hangs
  — because `FixtureType` carries no real ranges. Right about which way a head is
  moving, wrong in detail for any particular one. Centring the travel on the hung
  direction also lets a head hung straight down fold 135° past vertical and point up
  and backwards, which no real one does; per-type ranges are what settles it.
- Parking survives Clear and Store, as the spec asks. Nothing yet parks a value
  *across* a showfile reload, because nothing SYNCED does.
- The programmer holds a parameter or it does not. There is no partial override, no
  release time, and no touch-sensitive designer fader — that last is its own item in
  the spec and needs hardware to mean anything.

### 15. The workspace (done)

The main page was a fixed sidebar and six tabs, so the 3D rig, the values and the cue
list could never be seen at once — which is exactly what programming needs.

Panels now live in a tree of splits and tab groups: drag a tab to any edge of any tile
to divide it, or to the middle to stack it, and drag the gutters to resize. Six
presets ship built in and are always available; an arrangement worth keeping is saved
into the show as a PERSISTED `layouts` row, so the console next to it opens the same
way.

Two decisions worth recording:

- **Layouts are the show's; which one you are looking at is yours.** Two operators at
  two screens plainly want different tiles up, so the active layout and any unsaved
  rearranging live in `localStorage` rather than in the showfile. Rearranging never
  saves on its own — otherwise a busk on a spare screen would rewrite the layout
  everyone else is using.
- **The schema does not know what a panel is.** `LayoutNode` holds panel ids as plain
  strings, and one file in the frontend turns an id into a component. A layout saved
  by a newer build opens on an older one with the unknown panel simply missing.

The tree keeps two invariants after every operation: no split contains a split running
the same way, and no split has fewer than two children. The first matters because a
gutter drags the two tiles either side of it, and nesting would make some gutters move
tiles that are not next to them.

Panels that were the sidebar — show, session, devices — are ordinary panels now, and
the stage tab's two halves became two panels so both can be open together.

Left open:

- A panel can only be open in one tile at a time. Two views of the same plan at
  different zooms is a reasonable thing to want and is not possible.
- Nothing is responsive. A tree of tiles on a phone is a tree of very small tiles, and
  the spec asks for tablets and phones.

### 16. Something to install (done)

Everything up to here was two processes started by hand. `cargo run -p pult-backend`
served `/ws` and `/assets` and nothing else, the console was Vite on another port
reaching across origins to a hardcoded `ws://localhost:7700`, and there was no
release, no CI, no README and no LICENSE. A console nobody can install is a
program, not an instrument.

**The frontend is in the binary.** `rust-embed` over `frontend/build`, served as
the router's fallback so `/ws` and `/assets` are still matched first. One artifact
is the whole console.

The decision that everything else follows from is what that does to *where the
backend is*. The page now comes from the station, so the socket is on the origin
the page came from, and the question the `?port=` query string existed to answer
stops being a question. `endpoint.ts` is the only place that decides, `?port=`
survives as a way to name a second station on the same host, and `/api/config`
answers what a page genuinely cannot work out for itself — which station this is
and what version it is running. It is deliberately not asked *before* connecting:
a console must not wait on a request to make its first one.

The build is precompressed, so the `.br` beside every file was squeezed once here
rather than on every request from every tablet in the room.

**The console is a desktop app.** `crates/pult-gui` is a window around
`pult_backend::start`, which meant splitting `pult-backend` into a library and a
thin binary — worth doing on its own, since `main.rs` had been the only definition
of what starting a station meant.

The window points at `http://localhost:<port>`, the server it has just started,
rather than at a copy of the frontend bundled beside it. That is the choice worth
recording. It means the app and the tablet in the rig are the same page from the
same origin, there is one frontend to build, and the desktop build cannot drift
from the one everybody else uses — which is the MA web-remote arrangement, and
the reason the app defaults to port 7700 rather than asking the OS for one. It
also means Tauri's IPC is not available to that origin without a capability
naming it, so there is no native file dialog yet.

A second console on one machine is an ordinary thing to want, so a taken port
falls back to any port and says so in the title bar rather than refusing to start.

**The simulator has a window.** `tools/openhaunt-node-sim-gui`, and the thing it fixes
is small and real: `openhaunt-node-sim` can only be driven by typing at its stdin, and
`scripts/demo.sh` does not connect one — which is why the input node has to be
started with `--auto 2500`. Here a contact is a button.

It talks to the node over Tauri's IPC rather than over anything on the wire. The
sim implements the node side of the OpenHaunt protocol and nothing else, and a
debug UI is not part of that protocol. The one thing added to the sim itself is a
`Snapshot` — everything a node knows about itself in one value — because a window
should not have to subscribe to five channels and stitch them together. The panel
writes the port layout out again from the same documents rather than importing it
from `pult-schema`, for the reason the sim exists at all.

**Releases.** `.github/workflows/release.yml`, modelled on the one in
`Dein-Ticket-Shop/printer-client-rust`. Four products for Linux x86_64 and
aarch64, macOS arm64 and Windows x86_64, plus `.deb` for the server and whatever
Tauri bundles per platform. The repository going public is what makes the arm64
half cheap: `ubuntu-24.04-arm` runners are free there, so a Raspberry Pi build is
a native build rather than a cross-compilation exercise.

Two things about it worth knowing:

- **The frontend is built in its own job, once.** The generated TypeScript is not
  in the repository — it comes from the schema — so codegen has to run before npm
  does. Building it once also means every artifact ships byte-identical assets.
- **No `package` × `platform` matrix.** GitHub merges an `include` entry whose keys
  are absent from the base matrix into *every* combination, which is not what
  reading that matrix suggests. One job per platform builds both products.

Two footguns found while building it, both of which cost a window that was simply
blank:

- `devUrl` in a Tauri config wins over `frontendDist` in a **debug** build, so
  `cargo run` loaded a dev server that was not running. Removed; both profiles now
  use the bundled panel.
- `rust-embed`'s `$CARGO_MANIFEST_DIR` interpolation needs a feature flag it does
  not have by default. A plain relative path resolves against the crate root and
  wants nothing.

And four more that only a real tag could find, since every one of them passed
locally:

- The changelog headings were the bracketed Keep a Changelog form. The action
  matches `## 0.0.1` or `## v0.0.1` and nothing else.
- `npm ci` failed on Linux and nowhere else. npm's tree differs by platform, and
  generating the lockfile on macOS left `@napi-rs/wasm-runtime`'s own dependencies
  unresolved — they existed only pinned under `@rolldown/binding-wasm32-wasi`.
  The lockfile is generated in the image CI uses now, and `npm ci` is checked on
  Linux x64, Linux arm64 and macOS.
- `tauri build` takes cargo's arguments after a `--`.
- `beforeBuildCommand` runs from a directory Tauri infers from `frontendDist`
  rather than from the config's own, so `npm --prefix ui` landed one level too
  deep. The panel is built by the workflow instead, which depends on no inference.
- `uploadWorkflowArtifacts` is on tauri-action's main branch and not on the `v0`
  the workflow pins, so bundles that built were discarded. Collected from its
  `artifactPaths` output instead — with the backslashes turned round and the
  carriage returns stripped, because on Windows jq ends its lines with one and
  `cp` then goes looking for a file whose name ends in a carriage return.
- `generate-sbom` finds a version in a virtual workspace manifest, under
  `[workspace.package]`, but no name. It has to be told one.
- Building two packages in one job with the same action twice does not leave two
  archives behind: the second clears the first out of `target/<triple>/release/`.
  Every `pult-backend` archive went missing from a release that otherwise looked
  finished, and `upload-artifact` said so in one line — *"there will be 2 files
  uploaded"* — and carried on. Every upload is `if-no-files-found: error` now,
  because a release that is quietly missing its main product is worse than one
  that fails.

That last one had the same cause as the stray `pult-backend.d` in the archives,
and as the second copy of the binary in there that made every download twice the
size it needed to be: **the archive was cargo's output directory**, wiped and
re-tarred wholesale. `scripts/package-binaries.sh` stages what belongs in a
release by naming it — the binary, the licence, the readme — and it is a script
rather than workflow steps so that what goes into an archive can be checked on a
laptop before it is checked on a tag.

The cost of finding them was six tags. The alternative — a Docker container
reproducing the Linux half — found the worst one in a minute, and is worth
reaching for before pushing the next one.

Left open:

- **Nothing is signed or notarized.** macOS and Windows both warn on first run.
  The workflow is laid out so this is adding secrets and a few steps.
- **No auto-update.** `latest.json` and a Tauri updater keypair are the next step.
- **32-bit Raspberry Pi is not built.** The arm64 runner is aarch64, which is
  64-bit Raspberry Pi OS.

### 17. Three fields nothing ever read (done)

`Show.is_running`, `Show.active_sequence` and `Fixture.active_preset` were in the
schema from the first sketch of it and never acquired a reader. Nothing in the
backend ever set `active_preset` to anything but `None`. `active_sequence` was
never written at all — a sequence became active by having an `active_cue_index`,
which is where playback actually looks. And `is_running` had a button in the Show
panel that toggled it and one that nothing consulted: the engine ticks whenever
`Playback::has_work()` says there is work, which is a better answer to "is the
show running" than a flag an operator can get wrong.

Removing them costs nothing, which is the point worth recording. All three are
SYNCED, and a SYNCED field has no SQL column — `db.rs` loads by named columns and
the generated migration is unchanged byte for byte after the edit, which the
codegen run proves rather than asserts. There is no `deny_unknown_fields` anywhere
in `crates/`, so a showfile or a peer that still mentions them deserialises fine
with the field dropped on the floor. An old oplog row naming one is skipped with a
warning on catch-up, in `apply_peer_operation`, which already handles a path that
no longer resolves.

The lifecycle test that proved a SYNCED field is not persisted was written against
`is_running`, so it now runs against `editing_cue` — the SYNCED field on `Show`
that does have a reader, and the one whose loss on reload would actually be felt.

### 18. The shape of an effect (done)

Types only, no engine: `EffectSpec`, `SpeedMaster`, and the fields on `Cue`,
`Sequence`, `Fixture` and `DiscoveredDevice` that carry them. Nothing renders yet,
which is the point — the wire format and the storage format are settled before
anything depends on them.

**One primitive for sine and chase both.** A shape on intensity and a red-green-blue
chase look like separate features and are separate features on most consoles. They
differ only in what a cycle position maps to: a `Curve::Shape` reads a level out of a
function and scales it between `low` and `high`, while `Curve::Steps` looks the
position up in keyframes that carry real `ParameterValue`s. How fast, which way,
where in the cycle this fixture sits — the same question either way, so it is asked
once, in one envelope. That is why `EffectSpec` has no `kind` field.

**Phase is a number, spread is a memory.** `EffectSpec::phase` is this fixture's
absolute offset, worked out when the operator applied it. `Spread` records *how* they
asked, so the GUI can re-apply the same arrangement to a different selection, and the
engine never reads it. Rendering is then a pure function of one entry rather than of
an entry plus its position in a selection that may since have changed — which matters
because two stations must not disagree about what "third of five" means.

**Where an effect is measured from.** `EffectSpec::t0` is `Some` in the programmer,
set when it was applied, and `None` in a stored capture, where the cue's new
`Sequence::went_at` is the anchor. So `go_next` and `go_to_cue` now take an optional
`at`, and the frontend passes `Date.now()`. Commands run per station from the same
arguments, and that is the whole mechanism: a Go carrying its time makes every
console anchor the cue at one millisecond rather than at whenever its own actor got
to the message. A `Rate::Master` ignores both and uses the master's own `t0`, which is
what keeps every effect on one master in step.

**Tempo is a replicated fact.** `SpeedMaster` is mixed lifecycle like `Cue`: name,
bpm and multiplier persist, `running` and `t0` do not. Every tap and every bpm edit
rewrites `t0` along with the tempo, so a tempo change is a bounded step in phase
rather than a drift that grows — the new rate and the anchor it is measured from
arrive together. A `t0` of 0 after a reload is a defined anchor, not a missing one.

**The first LOCAL entity field.** `Fixture.live_effects` and `live_fades` are LOCAL,
which the macro has supported since task 2 and nothing had used. Every station works
them out for itself from replicated state, so broadcasting them would be sending each
console a slower copy of what it already has. They exist for the two readers that
cannot compute them: an output plugin deciding whether to hand a shape to a node
instead of streaming samples at it, and a panel drawing where each fixture sits in
its cycle. The generated migration confirms it — `speed_masters` appears with its
four PERSISTED columns and `fixtures` gains none.

**The trap worth naming.** `fixture_type_id` is a UUIDv5 over the serialised
`NodeDescription`, so a port's effect capability must not be a field on
`PortDescription`: firmware that started advertising effects would give every adopted
node a fresh fixture type and orphan every parameter already patched against the old
id. `effect_capability_from` reads it out of the raw `/info` body instead, the way
the mains flag already is, and a test pins the id across a description that gains an
`effects` block.

No migration anywhere. `captures` is one JSON column, `#[serde(default)]` covers the
new keys, and a cue or sequence written before any of this reads back with no effect
and no anchor.

### 19. The effect pass (done)

`model/effects.rs` renders; `playback.rs` decides what is running and what wins.
Nothing has left the console yet — an effect is still turned into a stream of values
like everything else — but the console now has something periodic in it.

**Two clocks, because there are two questions.** `Playback::tick` takes both an
`Instant` and a wall-clock millisecond now. A fade's progress is an elapsed duration,
and `Instant` is the only clock that cannot go backwards underneath one. An effect's
phase is a position on a clock that every station shares, and `Instant` is not shared
with anything — it is not even comparable between two processes on the same machine.

**The anchor is the cue, not this station's arrival.** `start_cue` places the
sequence's `went_at` on the monotonic clock and starts the cue's fades from there, so
a console that processed the Go 600 ms late starts its fade 600 ms in rather than at
the beginning. Before this, two stations were visibly out of step for the length of
every fade. For an effect it would have been worse: out of step for good, because an
effect never arrives anywhere to resynchronise at.

**Precedence, which falls out of the write order.** The overlay writes last, so the
rule is: a programmer effect beats a cue effect beats a cue value, and a plain
programmer value covers all of them. That last one is how an operator takes one light
out of a chase by grabbing its fader, and it is why `Overlay::held` became an enum
rather than gaining a second map. A cue effect under a held key keeps rendering into
`beneath`, exactly as a fade already did, so releasing lands on where the chase has
got to rather than where it was when the fader was grabbed.

**What the plugins are told, and what they are not.** `SetLiveEffects` and
`SetLiveFades` describe *why* a value is moving, and only for the winner on each key.
A plain programmer value over a chase produces no entry at all, and the absence is the
message: it is what tells a node to stop tracing a shape and take values again. Unlike
`emit`, this one keeps a cache of what it last said, because these two fields are
LOCAL and playback is their only writer — there is no other hand for the cache to be
wrong about, which is exactly the thing that made a cache wrong for `live_values`.

**Determinism, concretely.** The inputs to a rendered value are the spec, the speed
master, `went_at` and the wall clock, and all but the clock are replicated. Nothing
accumulates per station, so there is no drift to accumulate. A tempo edit rewrites bpm
and anchor together and is re-resolved on the next tick, which makes it a bounded step
in phase rather than a slide. The rate is resolved every tick rather than stored
resolved, so editing a master reaches every effect following it without anything
having to go looking for them.

One thing worth writing down because it was got wrong first: an effect the programmer
holds with no anchor of its own falls back to the epoch, not to now. Anchoring on the
current tick moves the anchor forward exactly as fast as time passes, and the effect
sits perfectly still.

Left open:

- **No amplitude fade into an effect.** An effect starts at full size the moment its
  cue goes. Fading a chase up is a real thing to want and it needs a second envelope.
- **Inter-station clock skew is assumed, not measured.** Every station is taken to be
  NTP-close. A 50 ms skew on a 2 Hz chase is a tenth of a cycle, which is visible. The
  peer heartbeat already measures round-trip time in `infra/sync/peer.rs`, so an offset
  could ride along on it; nothing does yet.
- **A tick never idles while an effect runs**, on every station, and a 500-fixture rig
  is 500 writes a tick. `emit` drops the no-ops, but the work is still done. This is
  the same question task 10 left open about partitioning, arriving from another
  direction.

### 20. Telling a node the shape instead of the samples (done)

The console side of the wire. A node that can trace a shape for itself is handed a
description and then left alone; one that cannot sees exactly what it always saw.

The arithmetic is the argument. A three second fade at 40 Hz is a hundred and twenty
MQTT messages to a node that could have been told "go to 1.0 over three seconds"
once. A chase is worse: it never stops. `drive_the_ports` now sends each port the
least it needs to hear — a shape, or a timed `set`, or the stream of values — and the
tests mostly assert what was *not* sent.

**Capability is per port and per shape.** A relay that can chop a square wave has no
way to trace a sine, and a port that lists its shapes lets the console find that out
without trying. Absent means the old behaviour, so nothing is negotiated and firmware
that has never heard of any of this is unaffected. It is read out of the raw `/info`
body rather than through `PortDescription`, for the reason task 18 records, and a
test now pins that end to end by adopting the same node twice — once advertising,
once not — and asserting one fixture type.

**Two messages, in order, to stop.** Taking a light out of a chase is `{"clear":true}`
and then a value, and the plugin drops its record of what the port was last sent so
the value goes out even when it looks unchanged. The node has gone back to holding
whatever the shape left it on, which is not what the console last recorded sending it.
Either message alone leaves the light wrong.

**A clock, because a phase needs one.** `openhaunt/clock`, retained, once a second
while this station is driving. Retained matters: a node connecting between ticks gets
an answer on subscribe rather than rendering against a guess for up to a second.
`seq` counts up so a node can tell a fresh sample from a retained one replayed after
the broker restarted. This is the first periodic thing in `DeviceManager::run`, which
until now was entirely event-driven.

Only the driving station publishes it. A follower putting its own idea of the time on
the leader's broker would give every node two answers, and the entire point of the
topic is that there is one.

**A timed `set` degrades to an untimed one.** The destination sits at the top level in
the port's ordinary payload shape and the timing rides beside it, so a node that
ignores `fade_ms` still lands on the right value — just immediately. That is the
fallback the whole design leans on: unknown keys are harmless, so no version has to be
agreed.

Nothing on the console renders differently. Art-Net, sACN and the universe cache still
read `live_values` and are untouched.

### 21. The simulated node renders for itself (done)

The other end of task 20. `openhaunt-node-sim` now advertises what each of its ports
can trace, accepts a descriptor, and animates on its own while nothing on the network
says anything to it.

**Written from the documents, not shared with the console.** `src/motion.rs` is a
second implementation of the five shapes, the five easings and the cycle arithmetic,
and it deliberately imports nothing from `pult-schema`. That is the reason the
simulator exists: two implementations that agree because they were both written from
the protocol prove the protocol is unambiguous, and two that agree because they share
a module prove nothing. Both test suites assert the same numeric table, and
`test_curve.c` in the firmware will be the third.

The one shared constant is the clock topic, and the end-to-end test asserts the two
crates' spellings of it match — a shape is only as good as the clock under it, and a
clock published where nobody is listening is no clock at all.

**Every write to a port goes through one place.** `Node::write_port` is the single
answer to "where is this port", used by a `set` off MQTT, by `POST /state`, and by
`run_renderer` forty times a second. So `GET /api/v1/state` and the panel both see a
port that is genuinely moving rather than a description of one that might be, and the
end-to-end test can prove the node moved by reading it twice.

**A `set` always cancels.** A console that has decided to send a value has taken the
port back; a shape still running underneath would overwrite it on the very next tick.
Clearing an effect leaves the port exactly where the shape had got to, which is why
the console follows a clear with a value — this node has no opinion about where a
stopped chase should leave things, and inventing one would be inventing state the
console does not know about.

**The clock estimate is smoothed and slew-safe.** The first live sample is taken
outright; later ones move a fifth of the way, because the error being corrected is
one-way network latency and a jump straight to each sample would jog a running effect
by however much that varied. A retained sample only ever seeds: the broker replays it
on subscribe and it was published at an unknown time in the past. A `seq` that goes
backwards means the broker restarted, and the estimate starts again rather than being
dragged through a number from before the gap.

**The editor can lie, and the config checker says so.** A `readonly` port that
advertises effects is a promise nothing will ever ask this node to keep, and a string
port that lists shapes is claiming there is something between two strings for a sine
to trace. Both are `problems()`, not deserialisation failures, so a config file with
one still loads into the editor to be fixed. `configs/fog-machine.json` and
`configs/mirror.json` both advertise now, and the mirror's engraved line advertises
steps and no shapes, which is the honest answer for a string.

### 22. The firmware renders too (done)

Two commits in [the firmware repo](https://github.com/OpenHaunt/node), not this one:
a pure-C core with host tests, then the integration. `oh_curve.c` is the third
implementation of the numeric table, after `model/effects.rs` and the simulator's
`motion.rs`, and the three share no code by design.

**What the bench showed.** A 0.5 Hz sine, applied from a running the-pult to an
adopted ESP32-1732S019, is *one* MQTT message and then twelve seconds of silence
while the strip port moves on its own. At 40 Hz that window used to be 480 messages.
A three second ease-in-out fade arrives as one timed `set` and the node walks it:
0.003, 0.10, 0.33, 0.66, 0.90, 1.0.

**The clock is not a formality.** Between an NTP-synced node and an NTP-synced
console on one LAN, the measured offset settled around -100 ms. That is well over a
tenth of a cycle on a 2 Hz chase, and it is exactly the skew `openhaunt/clock` exists
to absorb — the assumption that two NTP clients agree closely enough to share a phase
turns out to be worth checking, and does not hold.

**A plain `set` always cancels.** Stopping an effect is a clear and a value, and
`MqttLink::publish` spawns a task per publish, so the two can arrive either way
round. On the bench they did: the value landed before the clear. Both orders leave
the port in the right place, because a value takes the port back on its own — which
was designed in rather than discovered, but it is good to have watched it happen.

**A node upgraded in the field advertises nothing until its module is re-applied.**
The description is stored, and one written before the `effects` bits existed has none.
`oh vmod preset led` fixes it; a real module's EEPROM would carry the new field.
Worth knowing before wondering why the console is still streaming values at a node
whose firmware plainly supports better.

Three smaller things the compiler and the bench found, in the order they hurt:
a `-Wformat-truncation` error the host clang build does not produce but the Xtensa
GCC one does; a 1 KiB default MQTT RX buffer that would have dropped a sixteen-step
chase silently, now 4 KiB; and a `display_dirty` flag added and then removed on the
spot, because the display already marks itself dirty on any `OH_EVENT` and the field
had no reader — which is the same fault task 17 spent a commit removing.

### 23. View by default, edit on purpose (done)

The rule, the shared controls it needs, and the Patch panel as the worked example.

**Why a lock at all.** A console is a tablet gaffer-taped to a truss as often as it
is a desk with a mouse, and every panel written so far is two things at once: a view
of the show and a way to change it. On a laptop that costs nothing. On a tablet, a
delete button eight pixels from a row selector is a fixture unpatched during a show.
So a panel that can change the show opens read-only and says so, and the operator
unlocks it deliberately.

**The toggle is in the tile chrome, not in the panel.** It is the same control with
the same meaning everywhere it appears, and a panel's own buttons are exactly what it
must not be mistaken for. `PanelMeta.editable` marks which panels get one; closing a
panel locks it again, so reopening it an hour into a show does not put an unlocked
delete under a thumb with nothing having said so.

**Controls are removed from the DOM, not disabled.** A greyed-out button invites a
second, harder press. One that is not there says what it means. In view mode the
Patch panel is text cells and the row selectors — because selecting a fixture changes
nothing about the show and is what an operator does most.

**`styles/controls.css`, beside `tokens.css` and with the same policy.** Tokens say
what the colours are; this says what buttons, inputs and rows look like. Panels
written before it keep their own styles rather than being churned. `--hit: 44px` is
the target every converted control is built to, but a control reaches it with padding
rather than by *being* 44 pixels — a row of 44px faders puts four on a tablet screen.
The exception is the fader itself, which is dragged rather than pressed and so has to
be the size of the gesture: that is what `--fader-h` is for.

**Two kinds the picker was missing.** `Raw` and `Named` were left off the parameter
list on the reasoning that neither is chosen by name — a raw channel is addressed by
its binding, and a named parameter comes from a device describing itself. Both
arguments are right about where those kinds *come from* and wrong about who has to
type one: a light nobody has written a profile for is mostly raw channels, and
building its type by hand was impossible without editing JSON. So `kindFromLabel`
takes a name, and `kindOption` joins `kindLabel` — the two answer different questions
and a `<select>` needs the other one. A named parameter shows its own name, because
the word "Named" tells an operator nothing about which port they are looking at.

**Two things the browser found.** Per-parameter `default_value` had a schema field, a
reader in the output plugins and no way to set it; it is a `ValueControl` in the type
editor now, so a moving head can rest centred rather than hard left. And seeding a
fresh show broke: `fixtures/__create` refused a body that omitted `live_effects`,
which no client can know. LOCAL fields are `#[serde(default)]` now — a field the
station works out for itself is not one a client should have to name.

### 24. Speed masters (done)

A tempo several effects follow, and a big button to tap it with.

**Tap writes the tempo and the anchor together.** That pairing is the whole reason a
tempo change is a bounded step in phase rather than a slide: every station re-resolves
from the new bpm measured from the instant of the tap, so they all land in the same
place rather than each drifting on from wherever it happened to be. Editing the bpm by
hand re-anchors for the same reason, and starting a stopped master starts its beat
*here* rather than resuming a cycle that has notionally been running the whole time it
was off.

**The average is over the run, not the last gap.** A hand is not a metronome, and at
120 bpm a single 40 ms slip is 10 bpm — plainly audible. Anything before a two-second
pause is dropped, because that is the operator stopping rather than tapping very
slowly, and counting the pause as an interval would drag the tempo down for the next
several taps. The panel shows how many taps are in the run, which is the difference
between a tempo you trust and a number that appeared.

**Tap and Run/Stop stay live when the panel is locked.** Both are done during a show,
at speed, and a lock that stopped an operator following a tempo change is a lock
nobody would leave on. What the lock covers is renaming, the multiplier, and delete.

One beat clock for the panel rather than one per master: a dozen masters would be a
dozen animation frames doing the same arithmetic against the same millisecond.

`beatPhase` wraps rather than truncating, so a master whose anchor is slightly in the
future — which clock skew between two consoles makes ordinary — puts its dot at the
top of the beat rather than at a negative position.

Found by driving it: the create form used `autofocus`, which browsers apply on page
load and not reliably to an input that has just been inserted. Every other panel here
uses the `focusOnMount` action, and now so does this one.

### 25. The effects panel (done)

Where a chase is built. No Edit toggle — this panel *is* an editor, and it writes to
the programmer, which is the scratch buffer.

**The waveform is the panel.** An operator asking for "a chase across these six"
wants to see six dots moving round one curve, not read six phase numbers. The dots
are drawn from the same arithmetic `model/effects.rs` renders with, so they are where
the lights are; a browser that worked it out differently would be drawing a lie, and
`effects.test.ts` asserts the same numeric table the engine's tests do.

**Spreads are where "make them chase" becomes a number per fixture.** Even, Chase,
Reversed, Centre out, Wings, Groups and Random, each tested at one, four and five
fixtures — a selection of one has to survive every one of them, because "chase these"
with one light selected is an ordinary thing to do by accident and dividing by n − 1
would give NaN. Chase is `i / n` rather than `i / (n − 1)` on purpose: the last
fixture should be one step short of the first, not on top of it, or a four-light
chase looks like three.

**A random spread is a seed, not a roll.** `Math.random()` would make the phases a
fact about this browser at this moment, which is exactly what they must not be. The
seed is stored and the phases are rebuilt from it, so two consoles chase identically
and a reload does not reshuffle the rig. Reseeding is how you ask for a different
arrangement.

**One `effect_id` across the selection**, so the panel can gather a selection's worth
of specs back into one editable effect rather than the operator finding six unrelated
sines to change one at a time. The anchor is set on Apply, not on opening the panel:
one set when the panel opened would start the effect part way through its first cycle.

`setEffect` writes straight through rather than staging like `setValue`. Applying an
effect is one deliberate act, not a fader being dragged, so there is no burst to
coalesce — but it does clear any pending value for the same key first, or a drag
still in flight would land after it and cover it.

**A chip in the values panel, not a number.** The value under an effect is only where
it falls back to, so showing it would show a number the light is not at. The chip says
what has the parameter — `sine · 0.5 Hz` — and opens the effects panel, for which
`revealPanel` is new: bring a panel to the front wherever it lives, or open it if it
is not on screen. Where in the workspace a panel sits is not something an operator
should have to know to follow a link.

Two things the browser found. The parameter picker bound a null while the effect used
a fallback, so it drew a blank box over a working effect. And `bpm` is an `f32`, so a
tapped 56.1 comes back as 56.099998474121094 and a number input shows every digit —
rounded for display now, which is a console that can count.

### 26. Cue timing, and what a cue is actually doing (done)

The backend has had per-capture fade, delay and follow-after since task 3, and the
frontend has never had anywhere to type them. It does now, along with the two things
fractional cue numbers were always for.

**A cue list says what was asked for; the running strip says what is happening.**
During a three second fade or a running chase those are different things, and the
difference is exactly what an operator wants to see. It reads `live_fades` and
`live_effects` — the LOCAL fields task 19 added — and shows only what *this cue* put
there: a programmer effect over the top is the operator's, not the cue's, and saying
otherwise would be a lie. Fades are blue and arrowed because they are on their way
somewhere and will stop; effects are amber because they will not.

**Insert, at last.** `Cue.number` has been a float since the first sketch and nothing
ever inserted, so cues were numbered by counting. `insertNumber` takes the midpoint,
which is how a list survives being inserted into repeatedly without renumbering
everything below — and appending to a list that ends at 4.75 gives 5, not 5.75.

**Dragging a cue changes the order and not the numbers.** `cue_ids` is `ordered` in
the schema, so that is what a drag rewrites. Renumbering would make a cue an operator
calls "cue 5" stop being cue 5 because somebody moved cue 2, which is the one thing
cue numbers exist to prevent.

**GO and reset stay live when the panel is locked.** Running a show is what the panel
is for. What the lock covers is rewriting the cue list while it is being run from:
rename, delete, insert, reorder, and the timing — a fade time changed by a mis-hit is
a look arriving at the wrong moment with nothing on stage to say why.

The store menu gains a fade, a delay and a curve per capture, cue-level fade in and
out, and a follow mode. Zero on a capture means "use the cue's", which is what
`start_cue` already did, so leaving every row alone gives exactly the behaviour the
console had before there was anywhere to type these.

`DEFAULT_FADE_MS` replaces the hardcoded `500` and the `0, 0, 0` that every stored
capture used to get.

### 27. Devices, plans, positions and a flow's name (done)

The remaining gaps where the backend had a field and the frontend had no way in.

**A device's detail.** Address, host, firmware and protocol version, module id and
serial — and a port table with a *Can trace* column reading the capability the node
advertised in `/info`. Behind a disclosure because it is what you look at when
something is wrong and clutter the rest of the time. Adopt and Forget go behind the
lock; Find and Select stay live, because neither changes the show.

**More than one plan.** A show has as many as it has rooms, and the panel showed the
first and offered no way to reach the others — worse, the upload button *replaced*
the plan you had, so a second room cost you the first. There is a picker now, `New
plan` adds, and `stores/stage.ts` holds which one this browser is looking at so the
3D rig draws the same room's floor. Not show data, for the same reason the layout is
not: two operators at two screens want different rooms up.

**Plan rotation.** `stage.ts` has rotated plans since positions landed and nothing
could set the angle. A drawing squared up to the page is rarely squared up to the
room.

**Positions by typing.** Dragging in the plan is right for a whole rig at once and
useless for the one light that has to be at exactly 4.2 metres because the drawing
says so. `PositionEditor` gives x, trim and z — trim, because that is the word on the
plot an operator is copying — and a checkbox that turns a `Point` into an `Axial` one
with a direction vector. `splitPosition`/`joinPosition` keep the two forms from
drifting apart: they are the same fact with one detail added, and an editor should
not make somebody choose a variant before they can type a number. A new direction
points straight down, because a light with no aim yet is hanging, and aiming it at
the origin would point an upstage light across the room.

**A flow's name.** How you find one in a list of twenty, and there was no way to
change it. Double-click, behind the lock beside delete.

Found by driving it: the maximised view draws its own chrome and had no Edit toggle,
so filling the screen with a panel — exactly when there is room to edit it — was the
one place the lock could not be undone. And the device row was a flex row, which laid
the new detail panel out *beside* the name instead of beneath it; it is a grid now.

That leaves, from the gap list: device rename and configuration, blind, highlight and
fan, go-back, release and rate, and a timecode source. All of them want backend work
first except blind, which wants a second programmer buffer.

### 28. The demo, and what is left (done)

`scripts/demo.sh` starts a third node on `configs/fog-machine.json`, whose fog output
advertises every shape — so the demo has something to point at when it claims an
effect leaves the console as one message. Its "Try, in order" gains the effect, the
tap, the fade and the read-only patch, and now ends at thirteen steps rather than
eight.

`scripts/demo-seed.mjs` seeds a 120 bpm master at half speed and a third cue,
*Possession*, with a colour sine on the two heads half a cycle apart. Verified by
running it: both resolve to 1.0 Hz off the master, and the two heads' blue channels
sum to 1.00 at every reading, which is what "half a cycle apart" means when you can
see it.

Two stale things removed. The state table said effects and timecode were "not
started" with no schema; they are now separate rows because one is done and the
other is not, and `FollowMode::Timecode` has existed unimplemented since task 3.
And task 16 still listed "the simulator's panel cannot restart the node" as open,
which `Stopper` fixed.

**What is left of the GUI gaps**, all of which want something before they can be
built rather than being merely unwritten:

- **Device rename and configuration.** A node's name comes from its TXT record and
  `POST /api/v1/config` takes one. Nothing asks.
- **Blind.** Programming a look nobody sees needs a second programmer buffer, and
  the priority rule in `playback/programmer.rs` currently has exactly one overlay.
- **Highlight and fan.** Highlight wants a temporary override above the programmer;
  fan wants the spread arithmetic from task 25 applied to values rather than phases,
  which is a smaller job than it was before that landed.
- **Go back, release, and rate.** Going back needs the cue *before* this one on the
  same anchor arithmetic; rate is a live multiplier on a running fade, which the
  `Fade` struct has no room for yet.
- **A timecode source.** `FollowMode::Timecode` is matched and ignored. It should
  wait for the beat-grid work rather than sit beside it.

Task 8 — the WASM plugin runtime — is still the largest thing not started, and is
next.

### 29. What a tick actually costs (done)

Task 19 left this open as a worry rather than a number: an effect never arrives
anywhere, so a station running one never idles. Before effects existed a settled show
stopped ticking; now a show with a chase up ticks at 40 Hz for as long as it is up, on
every station. Measuring it found a real bug.

**`ShowView` scanned the fixture slice for every lookup**, which made the tick
quadratic in the size of the rig. `emit` looked up each fixture that moved, and
`live_value` looked one up for every key a fade or an effect started on — both by
walking the slice. Nothing noticed while a settled show stopped ticking, because the
scans only happen on ticks that do work, and before effects there were very few of
those in a row. The struct already indexed cues by id and did not index fixtures; it
does now. A thousand fixtures went from 29% of the tick budget to 16%.

**The numbers, release build, one effect across the whole rig:**

| Rig | `Playback::tick` | Process CPU |
|---|---|---|
| idle | — | 0.2% |
| 500 fixtures | 2.0 ms (8% of the 25 ms budget) | 24% of one core |
| 2000 fixtures | 7.9 ms (32%) | ~137%, over one core |

`Playback::tick` is the small half. The rest is the engine doing one `apply_local`,
one broadcast and one output push per fixture that moved — which is the cost task 19
actually named, and it is linear.

**The failure mode is graceful, and by construction.** At 2000 fixtures the process
is over one core and the tick cannot keep 40 Hz — but the effect still reaches the
top and bottom of its range on time, because a value is computed from the wall clock
rather than accumulated from the last one. A slow tick loses smoothness, not
correctness, and two stations under different load still agree. That is the same
property that makes stations agree at all, arriving somewhere it was not designed for.

Left open, and unchanged: nothing partitions fixture computation, so every station
computes every fixture. Task 10 named that and it is still the answer for a rig where
24% of a core is not acceptable. The obvious cheaper win first is that `emit` clones a
fixture's whole `live_values` map per tick and the engine then serialises it whole; a
per-key write would cut both. Neither is worth doing on these numbers.

### 30. Selection as a question about the rig (done)

The spec has asked for this since the first read and said why: a selection should be
*generated* from the rig by geometric functions and re-evaluated as the rig changes,
"useful for festivals, changing fixtures". Task 14 built the list of ids that comes
first. This is the query underneath it.

**A list of ids is a photograph of a rig that has since been rebuilt.** "The four
movers on the downstage truss" is still true after somebody patches a fifth; a list
of four ids is not. Verified the way it matters: with "every Spot" selected, patching
a third Spot took the count from two to three with nobody touching the panel.

**Read left to right, because that is how an operator says it.** A query is a list of
clauses that each add, narrow or remove — "all the movers, of those the downstage
ones, but not the broken one". A boolean tree would be more general and nobody wants
to type one. Nothing here forecloses a tree if a query ever needs one.

**Hand-picking is a query too.** A click builds an `Ids` clause, so clicking and a
geometric selection are the same kind of thing and combine: shift-click one more
light onto "every Spot" and the geometry still picks up the next Spot patched. There
is one representation rather than a list *and* a query with rules about which wins.

**Order is part of the selection, not decoration.** An effect spreads along it, so
this is what makes a chase run left to right rather than in patch order. Ties break
by name, so two fixtures at the same point come out the same on two consoles rather
than in whatever order each happened to list the rig.

**An unplaced fixture fails every geometric term**, and sorts last rather than to the
origin — where it would sit in the middle of the rig pretending to be somewhere. That
is why `Everything` and `OfType` exist: they are how a rig in flight cases is
reachable at all.

**`pruneSelection` is gone.** It existed because a list could hold a fixture that had
left the rig; a query cannot. The panel that called it no longer needs to know the rig
changed. That is the shape of the whole change: a question does not go stale.

Left open: a query cannot be saved. Groups are the obvious next thing and the moment
they exist the types belong in `pult-schema` rather than the frontend, with an
evaluator beside this one. Until something needs that, one implementation is better
than two that can disagree — the comment at the top of `selection.ts` says so, so the
next person knows it was a decision.

### 31. Taking it back (done)

Every console has an Oops key and every operator reaches for it before they can name
what they meant to do. This is that, plus the shared list of what everyone has been
doing, which turns out to be the more useful half on a two-operator tech.

**There is no undo stack.** The oplog already holds every write in order, and a
second list beside it would be a second thing to keep in step. So an operation
carries three more fields — who asked for it, what was there before, and which
operation it reverses — and undo is a query over the log. Three consequences fall
out rather than being built: an undo replicates to peers like any other write, so a
second station is not left showing a value that has been taken back; the same
person's other client sees it; and redo is undoing an undo, so one mechanism covers
both.

**Undo is per person, not per browser.** That was the ask, and it is the reason the
log rather than a stack: an operator with a desk and a tablet is one person, and
either device takes back what the other did. A browser-local stack could not do that
however carefully it was written. So the show gained a `users` table, a client says
who it is when it connects and again on every reconnect, and every write it makes is
attributed. A socket that came back anonymous would keep working and quietly stop
being undoable, which is the kind of fault nobody notices until they press Ctrl-Z.

**What undoes: everything editable, and nothing that moves lights.** The programmer
counts, patch counts, cue timing counts. A `goNext` does not. Commands are excluded
in `Operation::is_undoable` rather than at the call site, so a new command is
non-undoable by default — the safe direction. An operator who pressed Ctrl-Z
expecting to take back an edit would not thank a console that jumped the sequence
instead; going back a cue is a different gesture and has a different name.

**Undo and redo are told apart by chain depth, not by a flag.** An operation's depth
is how far along a chain of reversals it sits: a change is 0, an undo of it 1, a redo
of that 2. Undo takes the newest thing in effect at even depth, redo at odd. The
first attempt used "does it point at something" and quietly took the change away
again when you redid twice — an undo wearing the wrong label. And a fresh change ends
the redo branch, checked separately, because putting an old value back on top of work
done since is not what anybody means by redo.

**Whether something is still in effect is recursive.** An operation is reversed when
something points at it that is *itself* still in effect, so an undo that has since
been redone no longer hides what it undid. Flat set membership says a change stays
undone after it has been put back, and the second undo of a run then finds nothing to
do.

**The window counts edits, not rows.** Undo reads the most recent 500 authored
operations. Counting every row instead would have made it a window of about a quarter
of an hour, because a station writes its own telemetry into the log twice a second —
so a rename made twenty minutes ago would have stopped being undoable while its
author was still thinking about it. The same filter is what makes the History panel
readable: it lists what people did, and the console's own doing does not appear,
because nobody did it.

**Seeing is shared even though undoing is not.** The History panel shows everyone,
colour-coded and named, and marks which entries are yours to take back. On a
two-operator tech the useful question is usually "what just happened", and the answer
is often somebody else. Undos appear as themselves rather than being tidied away,
which makes the list a true account.

Left open at the time: a group undo, which is task 32. Nothing prunes the log yet,
so a long show's history grows without bound. And a user is a name and a colour, with
no notion of a station being signed into: two people at one desk have to remember to
switch.

### 32. One drag, one press (done)

Task 31 shipped with a hole in it: dragging a fader wrote a few hundred operations
and Ctrl-Z took back the last one. Right for a rename, useless for a move. This
closes it.

**Only the client knows where an act begins.** The backend sees a stream of writes,
and no amount of guessing at the gaps between them tells a drag from two quick edits
— a slow drag has longer pauses in it than a fast pair of clicks. So the frontend
says: everything written between a pointer going down and coming up carries one
gesture id, stamped on the message in one place, `PultWsClient.set`. A drag reaches
the socket through the programmer's staging, a path proxy and a panel's own handler,
and any of the three remembering to pass a gesture along would eventually be the one
that forgot.

**A gesture is not closed when the pointer lifts.** The programmer stages a move and
writes it on the next frame, and a fan across twenty fixtures is twenty round trips
after that, so a gesture that ended with the pointer would leave its own tail outside
it — and one drag would want three presses to take back. It closes 400 ms later
instead. Closing late is free: the only thing a stale id could spoil is the *next*
gesture, and beginning one replaces the id outright. The same timer gives a held
arrow key a gesture, which is a drag with no pointer to say when it stopped.

**An ordinary write is a gesture of one, keyed by its own id.** That is what keeps
`undo.rs` a single code path rather than two, and it is why a row written before
gestures existed still behaves exactly as it did. `undoes` came to name a gesture
rather than an operation for the same reason, and needed no migration to do it.

**Reversing a gesture writes one operation per thing it touched, not per operation.**
Otherwise taking back four hundred writes would put four hundred rows in the log, and
the log would grow faster the more of it you took back. The value it goes back to is
the one from before the *first* write at that path, which is what "before the drag"
means.

Three rules found by getting them wrong first. **A create is keyed by what it made,
not by where it was written** — two fixtures patched in one gesture are two writes to
the same `fixtures/__create` path, and collapsing them by path deleted one and left
the other standing. **A field written into something the gesture created is dropped**,
because the entity is going away and putting a value back into it first describes a
state nobody will ever see. And **the paths are unpicked newest first**, or a delete
runs before the rename above it and the rename writes into a hole.

**A gesture had to become the unit of `in_effect` and `depth` too.** Reversing a drag
writes one row against four hundred, so three hundred and ninety-nine of the drag's
operations have nothing pointing at them — and undo, looking at operations, found one
of those still standing and took the same drag back again.

Two things fixed on the way past. A peer received `user_id` and `previous` but not
`undoes`, so an undo replicated to another station landed in its log as a fresh
change and the next Ctrl-Z there took back the wrong thing; all four fields travel
now, as one `Authorship`. And the codegen owns `001_initial.sql`, so the oplog's
columns are declared in `tools/pult-codegen/src/main.rs` — a hand-edit to the
migration is overwritten by the next `generate`.

**And a gesture is what finally let the log stop growing at forty rows a second.** A
write inside one replaces that gesture's earlier write to the same path rather than
landing beside it: the row keeps the value the drag started from and takes the one it
ended on, which is exactly the pair undo wants and exactly what a peer catching up on
that path needs. Two seconds of dragging across a selection of twenty went from 2,400
rows to 20. The row takes the new sequence number as well as the new value, and that
is the part that makes it safe rather than merely smaller — catch-up asks for
everything past a sequence number, so a row that kept its first one would be
invisible to a peer that had already caught up mid-drag and would leave it sitting at
whatever value the drag was passing through when the two of them last spoke. Only
within one gesture, because two separate edits to the same path are two things
somebody did and folding them would leave the second with nothing to go back to. And
never a create, for the same reason creates are keyed by what they made.

Left open: a gesture is per client, so two operators dragging the same fader from two
consoles interleave into two gestures that each own half the writes. Undo still does
the right thing for each of them, but "before the drag" means before *their* first
write, not before the pair started. Nothing prunes the log. And nothing outside the
pointer controls opens a gesture — a panel that writes several fields from one button
could, and none does yet.

### 33. Two kinds of setting (done)

The console had no settings. A show had a name; everything else about how a station
behaved was a flag decided before it started and unchangeable from the desk. The
first thing that actually needed one was how far back Ctrl-Z reaches, and it turned
out to need both kinds at once.

**A show setting travels with the show.** `Show.history_depth` is PERSISTED and
replicates, because two consoles working one show have to agree about how far back
undo goes — a station-local number would not be a preference, it would be a
disagreement, and the two desks would give different answers about the same press.

**A console setting belongs to the machine somebody is sitting at.** A
`preferences.toml` in the platform's config directory, read and written over
`GET`/`PUT /api/preferences`. It decides what a *new* show starts with and then stops
mattering, which is exactly what keeps the first rule true. Machine-wide rather than
beside the showfile, which is the opposite of task 17's identity file and for the
opposite reason: an identity must not travel with a copied showfile, and a preference
about new shows has no showfile to sit beside. It is stateless in the router, so a
second console on the same machine sees a change without either being restarted and
there is no copy in memory to go stale.

**The number is changes, not presses.** An undo is a change too and shares the window
with the ones it reverses, so a run of them meets itself around half way: five hundred
changes is on the order of two hundred and fifty consecutive presses. Found by writing
a test that expected ten presses from a window of ten and got five. The panel says so,
because a number whose meaning has to be derived is a number people will get wrong.

**`#[serde(default)]` does not reach the showfile.** A SQL read does not go through
serde, so the new column — added nullable, as the additive pass must — came back as
zero on every existing show, and the clamp would have quietly left them with a depth
of ten. Fixed with a backfill in `upgrades.rs`, which is what that module is for, plus
a `const _: () = assert!(...)` tying the literal in the SQL to the constant the rest
of the console uses, so changing one without the other is a build error rather than a
surprise a year later.

Left open: this is one setting, and the panel is shaped for more. Nothing else has
moved into it yet — the output, session and device panels still own their own
configuration, which is right while each is a page of its own and will stop being
right when the second console-level preference arrives. The preferences file has no
version field.

### 34. A show carries its plugins (done)

A plugin was a directory on one
station's disk named by `--plugins`, which is right for developing one and
wrong for everything else — a show authored with the command line's panels in
its layout degraded on every other console in the session, and installing on a
ten-station rig was ten copies.

**A `plugin_packages` roster is PERSISTED, and the bundle is an asset.** The row
names its bundle by the sha256 of the zip; the bytes go in the same
content-addressed store task 13 built for stage plans, so `fetch_from_peers`,
the digest check and the dedup were all already written. Adding the collection
needed no edit outside `pult-schema`, which is task 2's promise still holding
thirty-two tasks later.

**The digest covers the archive, not the component.** A plugin is a manifest
*plus* a component *plus* the scripts a panel loads; hashing the wasm alone
would leave two thirds of it unversioned, and a changed panel script would be
indistinguishable from no change at all.

**Stations reconcile, keyed by `(plugin_id, sha256)`.** The diff is a pure
function in `roster.rs`, so the interesting cases are tested without a station,
a showfile or a wasm engine. The action worth having a name for is `Publish`: a
row carries a display name and a stage hint as well as a digest, and editing
those must not restart the plugin. That is task 9's lesson about outputs
arriving from another direction — rebuilding a live thing for a label edit put a
redundant frame on the wire, and here it would be a plugin restarting mid-show.

**A fetch never happens inside the event loop**, for the reason task 8 recorded:
the manager awaiting something that can call back into it is how the first
version deadlocked. It runs on its own task and reports back as a message, and
the test stands up a peer that accepts the request and then says nothing.

**`Fetching` is a state, not a failure.** A station that has just joined and is
downloading is working; saying "failed" would send an operator looking for a
fault that is not there.

**A `--plugins` directory beats the show**, on that station only. Otherwise
somebody editing a plugin on a console joined to a session would silently be
running the show's build — the most confusing thing the runtime could do to the
person least able to explain it.

**Configuration is three layers**, most specific winning: the manifest, the
show's row, then `[plugins.<id>]` in the station's `preferences.toml`. Station
last because what a station overrides is what is true of that machine and no
other, and a show cannot know which console holds the key. A plugin is handed
its configuration in `init` and never again, so a change is a restart.

**The trust assumption, stated rather than left implicit.** Opening a showfile
runs the plugins it carries: no approval step, and the manifest's permissions
are granted by opening the file. The sandbox, the epoch deadline and every
permission gate still bound it; what they do not bound is a plugin that
legitimately has `data = "read-write"` and an HTTP allowlist. So the Plugins
panel prints each plugin's permissions in words, and an approval gate is a
recorded open question with the schema laid out so adding one is a field and a
status rather than a redesign.

Three bugs the tests found, in ascending order of how much they mattered:

- The disk-override flag was set after an early return that fires when the plan
  is empty — which is exactly when the only roster row is the overridden one.
  The flag was never set in the one case it exists for.
- `PUT /api/preferences` built a fresh `Preferences` from the request body,
  which would have erased every plugin's station configuration — an operator's
  API keys among them — on the next change to the history depth.
- **A station that successfully fetched a bundle never ran it.** The
  placeholder row saying "fetching" already carried the digest, so the next diff
  saw a match and decided there was nothing to do. Every earlier test either had
  the cache already warm or had the fetch fail, so none of them went down the
  path the whole change exists for. Found by writing the two-station test last
  rather than first, which is the wrong order and is why it survived that long.

Left open: install over HTTP carries no user, so it is a change nobody can take
back; nothing prunes an unreferenced bundle from the asset store, the same way
nothing prunes a replaced stage plan; and the `stage` hint groups the panel and
gates nothing.

### 35. A plugin can remember things (done)

`lifecycle.init` ran on every start and every reload and nothing survived one,
so no plugin could offer a saved macro, a remembered provider or a snippet
library. Now a manifest declares `[[stores]]`, each `scope = "show"` or
`scope = "station"`, and four calls — `get`, `set`, `delete`, `keys` — reach
them. Declaring the store is the permission: a plugin can address no store it
did not declare and no other plugin's, because the host derives where the data
lives from the plugin id and the store id rather than from anything the guest
passes.

Three decisions worth keeping, and one thing that turned out to be false.

**`scope`, not `lifecycle`.** In this codebase LOCAL means *not persisted* —
state a station holds and shares with its frontends and which does not survive
a reload. A station-scoped store is the opposite: persistent and not
replicated, a fourth combination the enum has no name for and which `identity`
and `preferences` already are two instances of. Naming the axis `scope` says
the true thing — whose data is this — and leaves `Lifecycle` alone.

**Attribution is the switch, so nothing learned what a plugin is.** The design
was going to teach `Operation::is_undoable` to exclude `plugin_data` and add a
matching filter to the History panel. Neither was needed: `is_undoable` already
requires a user and the history reads `WHERE user_id IS NOT NULL`, so a write
made with `user_id: None` is non-undoable and invisible with nothing edited in
`pult-schema`, in the oplog's SQL, or in the frontend. That made the *opt-in*
nearly free as well — a store declaring `undoable = true` gets its writes
attributed to the operator instead, so a macro the operator asked to save
undoes like any other edit while a cache does not. The gesture is kept either
way, because coalescing keys on the gesture rather than on the user. And a
write with no operator behind it stays unattributed however the store is
declared, because `ctx.userId` is absent — the rule falls out of the mechanism
instead of being a second one.

**A key's identity is its row's identity.** `PluginDatum`'s id is a UUIDv5 over
`(plugin_id, store, key)`, not a fresh v4. `create_entity` takes the id from
the value the caller supplies, so a random one would have meant two stations
each writing `macros/opening` created two rows holding the same key — not a
conflict the vector clock resolves, but a duplicate it has no reason to notice,
and a plugin reading back two values for one key. The spec said "last writer
wins by vector clock", which is only true if both stations write the same
entity.

**The WIT version bump could not work as designed, and the package moved to
1.0.** Adding an interface was supposed to take the package from `0.1` to
`0.2`, with `API_VERSION` becoming a supported-versions list. It fails, and not
because of the new import: a component's imports are stamped with the package
version, so a `0.1` guest asks for `pult:plugin/data@0.1.0` and a `0.2` host
offers `@0.2.0` — nothing resolves, and wasmtime's semver-compatible linking
cannot help because under semver a `0.x` minor bump *is* breaking. At `0.x` the
contract could never have grown without stranding every plugin already in a
showfile. So the package is `1.0.0` and the manifest's `api` is a **floor**:
same major, station's minor at least the plugin's. Verified rather than
assumed — a `1.0` component does run on a `1.1` host — and
`scripts/check-api-compat.sh` reproduces the check, because a wasmtime upgrade
could quietly change the answer.

Two smaller things the tests pinned down. Within-gesture coalescing leaves
**two** oplog rows for ten writes to one new key, not one: the first write is a
create, and `fold_into_the_gesture` refuses creates because every create in a
collection shares the `__create` path and folding two would lose a row. And
`describeChange` names ids from fixtures, cues and sequences, so an undoable
store write would have rendered `plugin data → a1b2c3 → value` until the panel
learned to call a row by its plugin, store and key.

Also settled: an earlier draft worried that every browser would receive every
plugin's store, since `frontend_paths()` is derived and has no opt-out. That
was wrong. Subscriptions are demand-driven and per collection, reference-counted
by `stores/show.ts`, so a browser gets a store only if a panel asks for it. The
real cost of choosing an entity is the showfile, the backups and the snapshot a
joining station swallows — which is what the quotas are sized against.

Left open: nothing prunes the oplog, which store writes add to and telemetry
already dominates at twice a second; there is no change notification, so a
plugin holding an undone value in memory learns about it on its next read; and
a plugin that changes its own data's shape is on its own, as any application is.

### 36. A show always has somebody (done)

Undo shipped in task 31 and did not work on a new show. `users` was a PERSISTED
collection that nothing ever seeded, so a fresh showfile had none; the frontend's
`userId` started `null`, "before anybody has said"; and `Operation::is_undoable()`
requires `user_id.is_some()`. The first change an operator made therefore carried
`Authorship { user_id: None }` and could never be taken back — not later, and not
once they finally said who they were. Three pieces of UI existed to describe that
hole rather than fix it: a toast reading "Say who you are first", a history panel
empty state saying the same, and a chip bordered in `--live` whose tooltip was
"Nobody is signed in — changes cannot be taken back". All three are gone.

**There is no no-user now.** The engine seeds a default user at the end of
`load_from_showfile`, so it happens with no browser attached — a station runs
headless, and plugins and station RPCs write too. And the WebSocket write path
falls back to that user for a socket that never identified, so the guarantee does
not rest on a well-behaved client.

**The id is a fixed constant, and that is the load-bearing decision.** A UUIDv5
over the show's id would have matched how a `PluginDatum` derives its own, but a
v5 needs the `Show` row to exist at seed time and the load path promises no such
thing for an empty showfile. The stronger reason is the frontend: it has to be
working as *somebody* before its first write, and anything it had to fetch — or
wait for the `users` collection to deliver — would leave a window in which a
change is attributed to nobody, which is the exact bug. A constant has no window.
So `frontend/src/lib/users.ts` holds it too, as it already held `USER_COLOURS`
for a weaker version of the same argument, and a Rust test reads that file and
asserts the two agree. Duplication with a guard rather than duplication with a
comment; the guard was checked by breaking it.

**One per show, not per station.** `user.rs` opens by arguing that identity is
*chosen* rather than derived from the machine, because a person's desk and tablet
are both them — deriving the default from `node_id` would have contradicted the
reason the type exists, and put a login-shaped fact into a file that travels. So
two stations opening one show converge on one row.

The trap was `create_entity`: it validates, persists and inserts with **no
existence check**. An unconditional seed at every load would rewrite the row every
start, and on a second station replicate "Operator" over a name somebody chose.
The seed is conditional, and two tests hold it down — one that three loads write
no oplog rows at all, one end to end across two stations.

Left open, and accepted rather than solved: a station whose copy of the show
predates the default user, loading while disconnected, creates it concurrently
with another station's rename, and the sync layer breaks that tie rather than
intent. Bounded and self-healing — the ids match, so the worst case is a name
reverting to "Operator" and somebody renaming it again. There is no duplicate
user and no lost operation. Engineering around it would mean making the seed wait
for a session that may never come, which breaks the headless case it exists for.

Two smaller things. The seed is unattributed, like the engine's other writes, so
an operator pressing Ctrl-Z on a fresh show reaches their own first change rather
than the console's act of inventing them. And *Sign out* survives — it was a real
gesture for the end of a session — but lands on the default rather than on
nobody, and forgets the stored identity so the next visit does not resume as
whoever left.

### 37. The log has an end (done)

Nothing deleted an operation. `oplog.rs` had `append`, `since`, `len` and
`recent_by_people` and no `DELETE` anywhere in the backend, so a showfile grew for
as long as it was ever used. `history_depth` was already the number that should
have bounded it and bounded the wrong thing: the `History` command clamped its
limit to it and `recent_by_people` passed it as a SQL `LIMIT`, while everything
past it stayed on disk, invisible and unreachable.

Two facts made it a cost rather than untidiness. **The bulk of the table is not
what anybody did** — a station replaces its own row every two seconds
(`REPORT_INTERVAL`) as a SYNCED whole-row write, so it is logged: around 43,000
rows per station per day, each carrying a whole `Station` struct. And **catch-up
read the whole table**: `since` selects every row with no `WHERE` and filters in
Rust, so every reconnecting peer deserialized the entire log to find the handful
of operations it missed.

**Two retentions, counted differently on purpose.** Authored rows are bounded by
the show's `history_depth`, because that is already its promise about how far
Ctrl-Z reaches. Unattributed rows — nothing undoes them, they never reach the
history panel — are bounded by a *duration*, a station preference defaulting to an
hour. One rule over both would have broken what `recent_by_people`'s filter exists
for: at a row every two seconds, five hundred rows is a few minutes of edits. A
count for one and an age for the other is not an inconsistency: `history_depth`
counts changes because an operator counts changes, and an absence is a duration.

**The floor is the part that makes deleting safe, and it is a seq per node.**
Catch-up compares a peer's vector-clock entry for the node that *wrote* each row,
so "everything up to here is gone" is only meaningful about one node's sequence; a
single pruned-before timestamp would have compared two different kinds of thing.
`operations_since` gained its third reason to answer `None` — it already had "an
empty clock" and "replaying most of the log is a slow snapshot" — and the snapshot
path it falls back to is the one every joining station already takes. Without it a
peer behind the cut receives the surviving rows, sees no error, and believes it is
caught up.

The floor is written **before** the rows go, because the two failure directions
are not symmetric: a floor recorded for rows that were not deleted costs
unnecessary snapshots and stays correct, while rows deleted with no floor recorded
is the silent half-answer. And a floor only ever rises — `MAX` in the upsert, so
two prunes racing cannot interleave into a lower value.

**Pruning is local and never replicated.** Two stations legitimately hold
different amounts of history and each serves catch-up from what it has. Making
deletion a replicated operation would have invented a new kind of write for the
sync layer and let one station's disk pressure delete everyone's history.

**It runs on open and then every thousand appends, spawned rather than awaited.**
The engine is one actor, and this is the only place in it that issues a `DELETE`
over what can be a million rows — awaiting that would be a stalled tick, which is
a far worse failure than a log briefly too long. A flag stops a second prune
starting beside one already running, since two concurrent deletes racing on the
floor is the one way to get its ordering wrong. Driven by appends rather than a
timer because what should pace the work is how much has been written; a timer
wakes to do nothing on an idle show and still lands mid-burst on a busy one.

**The first open of a long show was measured** rather than guessed, since it is
the largest cut this will ever take and it happens at startup: a log of 25,160
rows — a fortnight of two stations' telemetry plus five thousand edits — took
**36 ms** to cut down to 500. The measurement is an `#[ignore]`d test rather than
a threshold, because a number asserted there would fail on a slower disk without
saying anything true.

**`since` was deliberately left alone.** Its missing `WHERE` looks like an obvious
win and is not: the predicate is a vector clock, so the query would be
`(node_id = ? AND seq > ?) OR ...` built per request from the asking peer's clock,
plus every row from a node that clock has never heard of. A query whose shape
depends on the number of peers, to replace a filter that is only expensive because
the table is unbounded, would be treating the symptom of the thing being fixed in
the same change. The retention bounds the table, so it bounds the read.

Two consequences worth holding on to. `HISTORY_DEPTH_MAX` now bounds what is
*kept* as well as what is read, so a larger value is a larger showfile rather than
only a longer query — its comment said "nothing prunes the log yet" and has been
rewritten. And the History panel shows its oldest entry as a boundary: the rows
past it are deleted rather than merely unlisted, and a list that simply ended
would read as a bug.

Left open: nothing vacuums the showfile, so SQLite keeps the freed pages and the
file stays at its high-water mark — a `showfile-management` question, and not what
this was paying down. **A station running a build without this change can
short-change a peer from a pruned showfile**, since it serves catch-up without
consulting a floor it does not know about; a session should not mix builds across
it.

### 38. A question worth keeping (done)

Task 30 made a selection a *question* about the rig — "every mover on the downstage
truss" — and then had nowhere to keep one. The types lived in
`frontend/src/lib/selection.ts`, which said so: if saved groups ever became show
data, they would move to `pult-schema` and the backend would get an evaluator beside
this one. Both have happened. `groups` is a PERSISTED collection whose row is a name
and a `SelectionQuery`, and nothing outside `pult-schema` was edited to make it
replicate, persist, appear in `data.ts`, or reach a plugin.

**A group stores the question, not the answer.** Resolving it reads the rig as it is
at that moment, so a fixture patched this afternoon is in this morning's group with
nothing re-saved, and a deleted one leaves it without anything having to prune. That
is the whole reason task 30 made a selection a query in the first place, and storing
the ids would have thrown it away at the moment it finally mattered.

**The evaluator is written twice, and the price is a checked-in corpus.** The
frontend re-evaluates on every change to the query, and a box or a cone dragged
across the rig changes it per frame — evaluation is on the interaction path, and a
round trip per frame is not available. So `evaluate` exists in `pult-schema` and in
`selection.ts`, and `testdata/selection-queries.json` holds one rig and 21 cases that
both test suites read. A case that disagrees fails on the side that is wrong, at the
commit that made it wrong. It is the arrangement `model/effects.rs` and
`frontend/src/lib/effects.ts` already had for the same reason.

**The trap was `Manual` order.** "Whatever order the operator dragged into" lived in
a Svelte store *beside* the query — deliberately, since an in-flight drag is not a
fact about the show. A group has no store behind it: it is read on a station that
never saw that drag, so a group saved left-to-right would have come back in patch
order everywhere else. `SelectionOrder::Manual` now carries the ids, `asSavedQuery()`
bakes the hand order in on the way out, and the evaluator takes `previous:
Option<&[Uuid]>` — `None` means "use what the query carries", and an *empty* hand
order is not the same as no hand order. That distinction is a corpus case, because it
is exactly the kind of thing that is right for a month and then quietly wrong.

**Resolving is a read, so it is a station RPC and not a command.** A
`#[pult_command]` mutates an entity and writes an operation; asking what is in a
group must not appear in anybody's history or undo stack, and a test asserts the
oplog length is unchanged across five resolves. That put a read in `api/rpcs.rs`,
whose header said these were "calls against LOCAL state" — the honest reframe, now
written there, is that these are the calls that *answer* rather than *change*.
`LocalRpcDeps` gained an engine handle to do it.

**And it is called `selection.resolve`, not `group.resolve`.** The command line's
parser checks RPC prefixes before collection names, so an RPC named `group.*` takes
the word `group` out of the grammar: `group 1` parsed as an unknown command on the
`group` RPC. **An RPC's prefix is a reserved word in the command line**, and naming
one after a collection deletes that collection's spelling. Worth remembering the next
time an RPC is added.

The backlog guessed that `group 3 at 50` came free from introspection. It did not:
`fixture` is a keyword in `parse.rs` and `group` had to become one too.
`Command::Select` now carries a `SelectTarget` — a range of the rig or one group —
and either word switches what a bare number counts, so `fixture 1 thru 5 + group 2`
composes with no second code path. Generic entity addressing did give
`rename group 1 "Movers"` for nothing.

**A selection effect can carry a query.** `group 3` on its own hands the browser the
group's *question*, so what the command line leaves is exactly what clicking the
group in the panel leaves — a live selection rather than a photograph of one. Mixed
lines (`fixture 1 + group 2`) resolve to ids instead, because a group's own clauses
may narrow or subtract and appending them would narrow the whole line. `at` needs the
ids either way, which is the one thing `selection.resolve` is called for.

Left open: a group cannot appear *inside* a query (`Term::InGroup`), which would give
live composition and needs cycle detection and an answer for deleting a referenced
group. Recall-then-refine covers the workflow; nothing here forecloses the term.

### 39. How much, rather than what to (done)

Every write was absolute. There was no way to say "ten percent brighter", only "at
62%", and the thing that had kept it that way was not the arithmetic but the
question of *where* a relative write becomes an absolute one. On the client it is
racy in exactly the case that matters: two operators reaching for one fader read
the same number, compute the same destination, and one of the two nudges is lost.

**Resolution happens at the front door and nowhere else.** `resolve_relative` runs
at the top of the `EngineCommand::Set` arm, before `authorship.previous` is read.
Everything below it — the apply, the oplog, the broadcast, `sync.broadcast_synced`
— is code that has never heard of the verb. That one placement is what buys the
whole property list: `previous` is the absolute before, the log holds a
destination, undo reverses it by writing `previous` back, and **a peer receives the
number rather than the delta**. It has to: a peer adding ten percent to whatever it
was showing would part company with the station that sent it on the first press.

The verb is `__by`, beside `__create` and `__delete`, so it arrives over the
existing `Set` message and needed no new protocol, no new host function and no new
permission. `data.set(path + "__by", delta)` is a relative write from a plugin, and
`.by()` on both accessors is the same thing from Rust or the browser.

**Two shapes, and the second one names a collection.**
`[table, ref, field, "__by"]` is the primitive: relative to what that field says.
`["programmer_values", "__by"]` with `{fixtureId, parameterKind, by}` exists
because the programmer's ordinary case is *not already holding the key* — `at +10`
on a light nobody has touched has no row to name, and what it has to be relative to
is then what playback is showing rather than a row that does not exist. So the
engine names one collection for a reason of its own. That is a real cost and it was
taken deliberately: the rule it looks like it breaks is "adding a collection needs
no edit outside pult-schema", and it does not break that — a test nudges a
`speed_masters` field, which the resolver has never heard of, to keep it honest.

What it resolves against is task 14's stack **read rather than re-implemented**:
the programmer's own value where it holds the key, `Fixture::live_values` where it
does not, and the fixture type's `default_value` where nothing has ever driven the
parameter. There is no second priority rule anywhere in this.

**A shape refuses.** `Overlay` holds a key as either a value or an effect, never
both, and nudging a shape would have to mean moving its offset — a different
feature wearing the same word. So does a switch and so does a line of text:
`ParameterValue::nudged` lives in the schema and says no by name, because quietly
doing nothing is worse than an error an operator can read.

**The trap was integers.** `nudge_json` first tried to answer `4500.0` for a
`fade_in_ms` of 3000 plus 1500, and `u64` will not take a float — a nudge on a
timing field failed at the patch rather than at the arithmetic. An integer field
stays one.

`programmer_entry_id` moved into `pult-schema`, where `ProgrammerValue`'s doc
comment already explained why the id is derived. There are now three
implementations of it — the frontend cannot run Rust and the plugins workspace
builds guests for `wasm32-wasip2`, which the console's schema does not belong in —
and all three are pinned to the same two literal examples, so any change to one
fails two suites.

The command line gained a sign rather than a word: `at 10` and `at +10` are
different commands, and `Level::To` / `Level::By` keeps them apart from the parser
down. `+10` is one token — the tokenizer only splits a *lone* sign off — so the
sign is read off the number's text, which is the sort of thing that is right for a
month and then quietly wrong. And the natural-language plugin needed no change at
all: it speaks by emitting command-line text, so "a bit darker" is now `at -10` and
the audit trail stays one grammar deep. That is `nl-show-context`'s option (b),
answered without giving the model any show data.

Left open: no fan, no multiplying sibling, and no relative value stored in a cue —
that last is tracking, which is its own design.

### 40. Four flaky tests, and one of them was the console (done)

`cargo test` failed about one run in three, in a different place each time. None of
it was flaky code in the usual sense; three were tests lying about their isolation,
and the fourth was the console giving a wrong answer that only a busy machine made
visible.

**A process has one environment.** `StationStore::open` read `PULT_PLUGIN_DATA` at
open time, and two stations inside one program cannot each have their own that way —
whichever set it last decided for both. `stores.rs` had already met this and worked
around it with a `OnceLock`, complete with a comment that a file per test would be
"a race over one variable rather than the isolation it looks like". Worse than the
flake: `plugins.rs` starts two stations and only one set the variable, so the other
opened the **developer's real `plugin-data.db`** and wrote to it. `Config::plugin_data`
now names the file, the way `showfile` and `plugin_dirs` already do, and the
environment variable is the fallback for a station started from a shell.

**A test that samples a fade has to sample it early.** The node sim's timed-set test
started a 400 ms fade, slept 200 ms and asserted the level was between the ends. A
saturated machine spends longer than that between the sleep and the read, so the
fade had finished and the assertion read "it jumped" when it had done nothing of the
kind. Three seconds, sampled a sixth of the way in: two and a half seconds of
overrun would now be needed to fool it. The test's own doc comment had said "a three
second fade" all along.

**Connecting is not free on a loaded machine.** Eighteen stations come up in
`roster.rs`, and a connect to a listener whose accept queue has backed up times out
rather than being refused — `ETIMEDOUT` on `127.0.0.1`, arriving as a panic in
whichever test was unlucky. The HTTP helpers are now as patient about connecting as
`eventually` already was about state, with a two-second connect timeout so a dropped
SYN is retried in a moment rather than after the operating system's minute.

**And the one that was not a test problem at all.** The same load made a station
fail to fetch a plugin bundle from a peer, and what it told the operator was *"no
station in this session has the bundle"* — because `fetch_from_peers` folded a
transport error into the same `continue` as a peer answering 404. Those are opposite
diagnoses: one sends you to install the bundle somewhere, the other sends you to the
network. `Fetched` now says which, and `begin_fetch` **asks again** when somebody
could not be reached, four times backing off from a quarter second.

That retry is a real behaviour change and worth stating plainly: nothing re-drove a
fetch, so a station that came up while a peer was still loading its own show had a
permanently failed plugin until the roster changed again. Only an unreachable peer
is worth asking twice — peers that all answered "no" will go on answering no.

The trap in writing it was that `break` left the *previous* attempt's message in
place, so a fetch that ended in "nobody has it" reported "could not reach one
station". Every arm sets the answer before deciding whether to go round again.

Reviewing that retry turned up the thing underneath it. **Every station publishes
its own row into `stations`**, and there were two `peer_addresses`: the one in
`api/rest` filtered self out, and the one in `infra/plugins` did not — while its doc
comment said "the other stations". So a fetch asked *itself* for a bundle it had
just established it did not have, spending a round trip to be told 404 by its own
HTTP server. Harmless-looking until the retry, which made a self-ask that failed
under load cost four attempts instead of one, and which is what put "could not reach
1 station" in front of an operator about their own console. There is now one
`peer_addresses`, in `assets.rs`, and both callers use it.

Three smaller things from the same reading. The constant's comment said "under two
seconds" for backoffs summing to 3.75; it sleeps after the last attempt no longer,
since there is nothing left to wait for; and an `Err` from `fetch_from_peers` is
this station's disk failing to store what came back, not a network, so it stops
rather than asking the same disk the same question four times.

The retry is held by a test that fails without it: a peer that drops the first
caller's connection and serves properly after. Closing a connection before any
response is a transport error rather than an answer, which is the shape of the real
thing and costs milliseconds rather than the ten-second timeout that black-holing
the port would have.

### 41. Where a parameter rests (done)

Nothing in the console ever put a parameter back. `emit()` merged each tick's changes
onto the map a fixture already had and no path ever wrote one back down, so a look
built by a sequence outlived the sequence, and clearing the programmer on a fixture
nothing had touched landed on a hardcoded zero of the right shape. `default_value`
had been sitting on every parameter since the node protocol arrived — derived from
what the device said about its own ports — read only by the connectors, and only as a
fallback for a key that was absent.

**The home value.** A parameter's own resting place: the fixture's `home_values`
override if it has one, and its type's `default_value` otherwise. Resolved once, in
`pult-schema`, because three callers wanted it — a relative write starting from
nothing, sending a selection home, and the programmer letting go of a key nothing was
under. No TypeScript twin: `fixture-groups` pays for two evaluators because a cone
being dragged re-evaluates per frame, and nothing here is on that path. The values
panel still carries a type's `default_value` per row, but that is a display fallback
for an empty readout rather than an answer about the rig.

The override is PERSISTED and on the *fixture*, not the type. A house light that
comes up when nothing is controlling it is a fact about this rig, and a type is
derived: the node describes its ports again, the console rebuilds it, and an override
living there would go with it.

**`__home`, a third path verb.** `["programmer_values", "__home"]` with
`{fixtureId, parameterKind?}`, beside `__by` and resolved in the same place — the top
of the `Set` arm, before `previous` is read — so the oplog, the broadcast and every
peer see ordinary absolute programmer writes. Omitting the kind means every output
parameter of that fixture, enumerated by the station, which is what lets the command
line's `home` and the natural-language plugin ask for it without reading the rig. The
same argument `relative-values` made for `at +10`.

One verb is several writes, which the `Set` arm did not previously allow: it now
resolves to a `Vec` and loops. And when a user's single request expands to more than
one write, the engine mints a **gesture** so one Ctrl-Z takes back the whole fixture
rather than one parameter of it.

**Off, and the change underneath it.** `Sequence::off` was the easy half. The hard
half was that `go_next` wrapped to `None` at the end of the list, so "the operator ran
out of cues" and "the operator turned it off" were the same state and playback could
not tell them apart. **Go at the last cue now stays on the last cue** — a breaking
behaviour change, and the better one: the comment at `playback.rs:283` had always
wanted a light not to go dark for want of cues, and leaving the last cue *active*
does that honestly, where leaving the values behind under a cue marked inactive did
not. A second SYNCED field saying "off" would have been two fields encoding one thing.

**What a release releases** is read from the show, never remembered: the parameters
any cue of that sequence captures, minus those a cue of another live sequence
captures, minus those the programmer holds. The tempting alternative — remember what
this sequence has actually written since it went on — is per station, and a console
that joined at the interval would release less than the one that ran act one. That is
two rigs with no way back. Reading the cues is stateless, identical everywhere, and
right for the same reason: a parameter no cue of any live sequence captures is a
parameter nothing is driving. It errs conservatively in both directions — a cue that
never ran still contributes, and a parameter another sequence merely *could* drive is
left alone.

**How long it takes** is `Show::home_fade_ms`, PERSISTED, zero by default so nothing
an operator was used to changed. Show data rather than a station preference, against
the first instinct, because `types/show.rs` had already written the argument for
`history_depth`: a station default that keeps applying lets two stations give
different answers about one show. For undo that is a disagreement; for a rig two
consoles are both driving, it is one the audience can watch. The station preference
decides what a *new* show starts with and then stops mattering.

Three things fell out of the reading. `zero_like` is gone, and with it the console's
last guess about where a parameter belongs; where nothing can say — a fixture whose
type has gone — a cue now lands rather than fading from a zero nobody vouched for.
`parameter_key` moved into `pult-schema` beside `programmer_entry_id`, because the
home resolution is the fourth thing that needed it and three spellings of one key was
already one too many. And `home_values` needed a showfile upgrade of the same shape
as `history_depth`'s: the additive pass adds a column nullable, and a JSON column read
as NULL is a parse failure rather than an empty map.

### 42. Four things left open (done)

Not a feature but a sweep: the four "left open" notes the last three changes wrote
down, closed together because each was one decision that had already been argued and
deferred rather than a design nobody had started.

**A cue fades two ways.** `Cue::fade_out_ms` and `ParameterCapture::fade_out_ms` had
been declared since cues existed and read by nothing; task 41 explicitly declined to
say what a cue *out* time meant. It means the classic split: a capture takes the out
time where its destination is below where it starts, and the in time otherwise, with
a capture's own time beating the cue's on each axis independently.

Two decisions inside that. **Zero out means "this cue does not split its fade"**,
not "snap" — an unset field cannot become an instruction, or every show ever written
would start slamming its levels down. And **only a value with an order can be coming
down**: `Float` and `Int`. A colour has three orders and no agreed way to rank them,
a relay has none, and a fixture whose parameter changed kind mid-show is a mistake
rather than a direction — all of those take the in time. A colour fading to black
*looks* like a fade out, and treating it as one would be the console inventing a
ranking to give some cues a time nobody asked for.

The cost, found by a test rather than by reading: `a_cue` in the engine tests carried
`fade_out_ms: 3000` from before the field was inert, so `an_intensity_cue(_, _, 0)` —
a cue asking to snap — began taking three seconds to come down. Data written when a
field meant nothing is data that means something now, and the fixture was the thing
that was wrong.

**Where it rests, from where it is.** `["fixtures", "__set_home"]` with
`{fixtureId, parameterKind?}`, the fourth path verb and the exact inverse of the
third: `__home` sends a parameter to where it rests, this makes where it rests be
wherever it is now. Which is how an operator actually sets a house light's — aim it,
look at it, keep it — where the Home column in the patch panel wants a number typed
in about a light that is already right in front of them.

A verb rather than a plain write to `home_values`, for the reason `__home` is one: a
browser could read `live_values` and write the map itself, and the command line and a
plugin with no data access could not. The whole argument of `__by` and `__home` is
that a caller able to *act* should not have to be a caller able to *read the rig*.
One write of the whole map, because `home_values` is one JSON column and taking a
fixture should be one Ctrl-Z. A parameter putting nothing out is not taken — refused
by name when it was the one asked for, quietly skipped when the whole fixture was.

**A station that gave up asks again.** `plugin-fetch-retry` left a fetch failing
permanently until somebody touched the show, which meant the console holding the
bundle walking in five minutes late changed nothing and the operator's move was to
edit the roster until the reconcile ran — a workaround for a show that was never
wrong.

What re-drives it is **somewhere new to ask**, and nothing else. The manager watches
`stations` beside `plugin_packages`, compares the peer *addresses* against what it
last saw, and re-drives only failed fetches and only on growth. A station leaving is
not new information; neither is a heartbeat, which is why the filter is in the
watcher rather than the handler — a station row carries cpu, memory and a timestamp,
and waking the manager for each of those per console per second to learn nothing is
the version of this that would have been worse than the bug. No timer, because a
station that is not there will not be there in thirty seconds either, and a session
has as many re-drives in it as it has consoles arriving.

It does not contradict `ASK_PEERS_TIMES`' "an answer is an answer". That rule is
about asking *the same stations* twice inside one fetch. A station that was not there
has not answered.

**A plugin can be told.** `plugin-datastores` left a plugin holding a value in memory
learning about an undo on its next read, which for anything cached is never.
`store.subscribe(store)` closes it, delivered through the **existing**
`lifecycle.on-update` as `[store, key]` with a null value for a key that was
forgotten — one token space with `data.subscribe`, so a plugin that uses both tells
them apart the way it already does.

Built on the engine's broadcast rather than a hook in `show_store_set`, which is the
decision that matters. A hook would see only this station's guest writing — exactly
the case a plugin does not need telling about — where the broadcast also sees an
operator's undo, another station's copy of the same plugin, and a showfile catching
up. So a plugin hears its own echo, which is easy to ignore because it knows what it
just wrote, and hears the three things it could not otherwise learn at all.

The mechanical wrinkle: a `plugin_data` row is a UUIDv5 over `(plugin, store, key)`
and that is one-way, so a field write broadcasting `plugin_data/<id>/value` says
nothing about which key it is. The subscription task keeps `id -> key`, seeded at
subscribe and kept up as rows arrive — which is also what lets a *delete* report the
key it was, since by then the row is gone. A create or delete broadcasts the whole
collection, so those are a diff rather than a lookup.

A station-scoped store hands back a dead token: this machine's file, this plugin its
only writer, nothing to report. Said with a token rather than an error, the way
`data.subscribe` answers a plugin with no data permission.

**And the contract moved to `pult:plugin@1.1.0`** — the first exercise of the floor
rule task 35 went to 1.0 for. Adding a function to an imported interface is
additive; a plugin built against 1.0 never calls it. That claim was checked rather
than trusted, and not with the synthetic bump `check-api-compat.sh` performs: the
reference plugin was built from the previous commit in a throwaway worktree, its
imports confirmed as `pult:plugin/data@1.0.0`, and run against this station. It comes
up Running. `store-probe` is the one manifest that moved its floor to `1.1`, because
it calls the new function and a 1.0 station could not serve it — which is what a
floor is for.

### 43. A show with a size, and a tick that says what it cost (done)

Task 29 measured what a tick costs and found a real bug doing it. Three years of
that work survived as a table in this document and nothing else: the rig was ad-hoc,
the instrumentation was added to get the number and taken out again, and the only
show anybody could start in one command had five fixtures in it. `multithreading` is
judged entirely on numbers, so it needed both back before it could begin.

**The demo has sizes.** `scripts/demo.sh --size small|big|huge`. `small` is the
hand-made show and the default, so every existing invocation seeds exactly what it
did before; `big` and `huge` add a generated rig on top — 500 or 2000 six-channel
heads addressed across as many universes as they need, a stack of cues over several
sequences, and effects left running so the station does not settle.

Two shaping decisions. **A cue captures a slice, not the rig**: 300 cues times 2000
fixtures is 600,000 captures, which measures JSON rather than lighting, and a real
cue stack does not touch everything in every cue either. And **the fixtures are
placed**, because half of a tick is what leaves the console once something has moved,
and because a rig with no positions draws nothing in the panel most likely to be the
reason somebody wanted a big rig.

**Seeding stayed on the WebSocket API.** `demo-seed.mjs` says in its own header that
nothing in it is privileged and that this is the point, and a 2000-fixture seed is
the largest exercise of the write path anything in this repo performs — worth more
than the time it costs. What changed is that writes go in flight together through a
bounded window of 64 rather than one awaited round trip at a time. Bounded, because
the engine is one actor behind a channel 256 deep: firing two thousand writes at once
does not go faster, it fills the channel, and the backpressure then arrives as a
per-request timeout on writes that were only ever waiting their turn. A `huge` seed
takes 43 s in release, 119 s in debug. `--keep` exists so nobody pays it twice.

**A station publishes what its own tick costs**, in the `stations` row beside
`cpu_percent` — which answers `system-stats-panel`'s open question (extend the row,
or a new LOCAL stats collection?) in favour of the row, on the grounds that a station
is already the sole authority on its own numbers there. Accumulated into relaxed
atomics by the engine and drained by the reporter every couple of seconds, so a
figure always describes the window just gone. Six nanoseconds a tick, measured, which
is what "measuring does not change what it measures" had to mean.

**Two figures, not one, and this is the finding.** The tick has two halves that scale
differently: computing what playback wants, and everything else — reading the show it
needs, then applying the result as one write, one broadcast and one output push per
fixture that moved. Task 29 put the split at roughly one to three. It is far past
that:

| Rig | Tick | Worst | Playback | Share of tick | CPU | Ticks/2 s |
|---|---|---|---|---|---|---|
| small — 5 fixtures | 0.42 ms | 1.00 ms | 0.03 ms | 7% | 3% | 52 |
| big — 505 fixtures, 4 stacks up | 10.6 ms | 12.2 ms | 0.20 ms | 1.9% | 55% | 80 |
| huge — 2005 fixtures, 12 stacks up | 65.2 ms | 70.0 ms | 0.73 ms | 1.1% | 115% | 30 |

Release build, this laptop, every sequence driven to a cue carrying an effect. At two
thousand fixtures **playback is one percent of the tick** and the other ninety-nine is
the engine around it. A single figure would have credited all of it to playback and
sent the next optimisation to the wrong code.

**And a third counter, added afterwards for one run, says where that ninety-nine
actually goes.** Not where this entry first assumed:

| Rig | Whole tick | Reading the show | Computing | Applying |
|---|---|---|---|---|
| huge — 2005 fixtures | 35.2 ms | **33.8 ms (93%)** | 0.07 ms (0.2%) | 2.2 ms (6%) |

`playback_tick` calls `read_collection` six times — fixtures, cues, sequences, fixture
types, programmer values, speed masters — and each one clones the collection out of
`ShowState` as `serde_json::Value` and then deserialises it whole into a `Vec<T>`. Two
thousand fixtures go through that forty times a second. **Applying the effects is six
percent of the tick; reading the show to compute them is ninety-three.**

That is the correction worth having before `multithreading` starts, because it says
the work is not threads: parallelising a render that costs 0.07 ms would win nothing,
and the cheap win task 29 named — `emit` cloning a fixture's whole `live_values` map —
is in the six percent. The engine re-deserialising the show at 40 Hz is the cost, and
it is not a concurrency problem at all.

**Mean and worst, because the tick has a budget.** An average over a couple of seconds
answers a different question and hides an overrun happening several times a second.
The `huge` row is ten times over its 25 ms budget and the failure is still the graceful
one task 29 described: 30 ticks in a two-second window instead of 80, so the picture
loses smoothness, and nothing loses correctness, because a value is computed from the
wall clock rather than accumulated from the last one.

**"The tick" is `playback_tick`, and the definition is load-bearing.**
`push_output_config` and `flows_tick` share the timer arm and are outside the number.
Not because they do not cost anything, but because they run *whether or not playback
had work* — so a number including them would make every timer firing a tick, and "this
station is not ticking" would stop being a state anything could report. Which matters,
because a settled show stops ticking and a window with no ticks in it publishes
**nothing rather than zero**: zero reads as "instant" when what happened is that
nothing happened. That distinction is why the field is an `Option<TickCost>` rather
than a struct of zeroes, and it is what a station taken off has to be able to say
instead of republishing the last figure it managed to measure.

The trap, found by a test that was wrong before the code was: "settled" means no
playback work *and* nothing written since the last tick. A write bumps `state_version`,
so exactly one tick runs after it to reconcile. A console being programmed therefore
reports a cost; only one left alone reports nothing.

What this number is not, said here because it is the way it will be misread: it is
what *playback* costs, not what the process costs. That is `cpu_percent`, in the same
row, which is why `--measure` prints both and says so underneath.

**No CI budget.** The backlog asked; the answer is not yet. A threshold needs a number
that holds still, shared runners do not give one, and a gate that flaps gets disabled,
which is worse than no gate. The A/B above is the shape of the problem: two *identical*
runs of `huge` varied by more than a percentage point of CPU and fifteen milliseconds
of tick. Revisit once `multithreading` has moved the numbers and we know how stable
they are.


### 44. Values as functions (done)

Task 43 measured a tick and the number came back wrong-shaped. At 2005 fixtures a
tick cost 35.2 ms, and of that **33.8 ms was reading the show, 2.2 ms was applying the
answer, and 0.07 ms was computing it**. The conclusion written down then was that the
tick was not a concurrency problem. It was not a *computation* problem either. It was
that the answer became state.

The console evaluated a fade forty times a second and stored the result in
`Fixture::live_values`, and every one of those samples was then read, written,
versioned, broadcast and read again. Stop storing it and 99.8% of the tick goes with
it, along with the reason for the engine to have a tick at all.

**A fade was already an object.** `RunningFade { from, to, t0, duration_ms, easing }`
anchors in absolute console milliseconds and needs nothing else to be evaluated;
`RunningEffect` is the same shape. One output path already took that offer — an
OpenHaunt node advertising `transitions` is handed the description and left to it, so
a three-second fade leaves the console as one message instead of a hundred and twenty.
This change is that offer taken everywhere.

**What the numbers came to**, on the same machine, `--release`, `--size huge`:

| 2005 fixtures | before | after |
|---|---|---|
| whole | 35.2 ms per tick, 40 Hz | **2.86 ms per output frame**, 34 Hz |
| reading the show | 33.8 ms | — |
| applying it | 2.2 ms | 0.26 ms (emitting) |
| computing it | 0.07 ms | 2.60 ms (evaluating) |
| updates to a connected browser | thousands a second | **0 in four seconds** |

The evaluating figure went *up*, and that is not a regression: the old 0.07 ms rendered
only what had moved since the last tick, and the new 2.60 ms works out every parameter
of every patched fixture from scratch, every frame, plus assembling twenty-four
universes. Paying it thirty-four times a second instead of paying thirty-five
milliseconds forty times a second is the trade. Eleven percent of the frame budget
against a hundred and forty percent of the tick budget.

The last row is the one an operator feels. A cue on a 2000-fixture rig used to put a
few thousand messages a second onto every connected console; it now puts none at all,
because nothing about the rig changes while a fade runs.

**One evaluator, compiled twice, and no TypeScript twin.** The arithmetic moved to
`crates/pult-render` — `serde` and `uuid` and nothing that touches an OS — and is
linked natively by the station and compiled to `wasm32-unknown-unknown` by
`crates/pult-render-wasm` for the browser. This is a *second* wasm toolchain: the
plugins are `wasm32-wasip2` components run by wasmtime on the host, which is the wrong
tool entirely for code that has to run inside a tab.

The alternative was what `fixture-groups` did for `SelectionQuery`: write it twice and
hold the two together with a corpus. It was declined, and the reason is the size of the
surface and the shape of the failure. Easings, curves, step lists, spread, phase,
direction, width, master rates, priority, home fallback and split fades are an order of
magnitude more than a selection query, and a drift between them shows up as the screen
disagreeing with the lamps — which an operator cannot work around and may not notice
until it matters. What is guarded instead is that the two *compilations* agree:
`testdata/driven-values.json` is 47 cases read by a native Rust test and by a vitest one
that loads the wasm build and asks it the same questions.

**A landed fade stays.** `live_fades` stopped meaning "in flight" and started meaning
"what is driving this parameter", including the fades that have arrived. That follows
from the removal rather than being a decision beside it: once nothing stores the number
a fade landed on, the finished fade is the only thing that remembers it, and evaluating
one gives exactly that constant. An effect that stops without anything taking its key is
*parked* the same way — a fade of no length at the value it was showing — because
stopping a chase should freeze the look, which is what leaving the number in a map used
to do by accident.

**Connectors own their rate; the engine pushes the show.** `OutputPlugin::send` gained
a moment and lost the assumption that somebody had already computed the values. Each
connector holds the last patch it was pushed and draws its own frames out of it — the
DMX family at 40 Hz while anything is moving and at its 800 ms keep-alive when nothing
is; an OpenHaunt node told once. The engine pushes when the *show* changes, which after
this change is the only kind of change there is.

**The 25 ms timer went, and what replaced it is a deadline.** `ShowEngine::run` now
sleeps on the soonest thing it actually has to do — a follow cue coming due, a *Watch*
node wanting a sample — and on nothing at all when there is neither. A settled station
and a station running a chase are now the same amount of work: none. The one thing that
still samples is edge detection, which cannot be done from a function; it was already
gated by a `watched` set and is now *proportional* to it, so a graph watching one lamp
of two thousand costs one evaluation a sample rather than two thousand thrown away.

**The browser has to learn the station's clock, and say when it has not.** This is the
one genuinely new mechanism the change needed. The objects are anchored in console time,
so a page evaluating against an unadjusted `Date.now()` runs every fade out by however
wrong its own clock is — and does it silently, because every individual value is
plausible. The offset is estimated the way a round-trip time is, kept as the best of
five samples, and maintained rather than taken once, because clocks step. The rule that
makes the failure visible rather than silent: **a client with no offset yet presents
nothing**. `consoleNow()` answers `null`, and a panel shows a gap.

**What stayed state.** `Fixture::live_values` did not become nothing; its two halves
were separated. What a device *reported* — a contact, a temperature, a humidity — is
now `sensed_values`, and it stays SYNCED for the reason it always was: the wire it came
off is attached to this station and nobody else can work it out. Driven outputs are
functions of time; sensed inputs are facts the console was told. Removing the old field
rather than deprecating it was deliberate — an unwritten `live_values` would have left
every reader compiling and silently seeing nothing move, where a removal makes the
compiler and `svelte-check` produce the list of things to fix.

**What a tick cost became what a frame costs.** The published figures moved from the
engine to the output frame, one entry per connector, keeping the mean and the worst and
the rule that a window with no frames in it reports **nothing rather than zero**. Per
connector because their rates are their own: Art-Net at 40 Hz beside a node told once
are not two samples of one number.

**The trap, for whoever is next.** The show clock advances monotonically from one wall
reading, so `tokio::time::pause()` does not fast-forward a fade. A test that needs one
to advance has to let real time pass or drive `Playback` directly. Several tests here
run a short fade in real time for exactly that reason.

**And one bug found while verifying it, fixed here because it was in the way.**
`scripts/demo.sh --two` could not sync at all on this machine: the two stations
discovered each other over mDNS, joined the session, and never connected.
`infra/session/mod.rs` took `info.get_addresses().iter().next()` — and that is a
`HashSet`, so it was not the first address the other station advertised but whichever
the hash order gave, differently on different runs. Half the time that was a
link-local `fe80::` address, which cannot be dialled at all: it is only meaningful
together with the interface it was learned on, and mdns-sd does not say which that was,
so the `SocketAddr` built from it carries no scope id and the connect fails with "No
route to host".

Two halves to the fix, because ranking alone would still be a guess. `reachable_at`
orders what was advertised by how far it reaches — the network, then the segment, then
this machine — and drops what cannot be dialled rather than ranking it last, since
spending a timeout on it only delays the address that was going to work. And
`sync::dial` works *down* that list with a two-second budget each, because which address
reaches a given machine is not something either end can know on its own.

And the half of it that was not about addresses at all: `Join` used to answer `Ok`
before the dial had been tried, so a session that could not be reached was one the UI
said had been joined — the Sessions panel has had a toast for that since it was written
and it could never fire. It waits now, and answers what happened, naming every address
it tried. Safe to wait on despite the deadlock the sync manager is careful about: that
manager spawns the dial and goes on draining its channel, so nothing the session actor
waits for is waiting on it. Bounded at three seconds across every candidate, split
between them, because somebody is on the other end of the answer — an answer that
arrives after the caller has given up is the same as no answer.

## What is next

This document is the whole of the planning, again. The numbered tasks above are
finished work with the decisions and the traps recorded; the entries below are
what has not been started, each one carrying the questions it has to answer
before it can be built. That is the part worth keeping: an entry records what was
asked and what is true of the code today, so the questions do not get
re-discovered from scratch every time somebody picks the item up. When one is
built it becomes the next numbered task and leaves this list.

Verified against the code on 2026-08-31 unless an entry says otherwise.

### The order

Sections below are themes, not sequence. This list is the sequence, and it is the
one thing here meant to be rearranged. `→` names what has to exist first.

Reordered on 2026-09-02, into four phases with a reason each, rather than into a
ranking. **Get a real rig in, be able to see what the console is doing, measure
it, then fix what the measurement found.**

**A real rig first.** Everything downstream reads better against one. The viewer
gets a beam angle it cannot otherwise have and a plan worth drawing; the
measurement gets a rig somebody actually hung rather than a generated one; and
the 540°/270° constants task 14 complains about get real numbers. This is also
the answer to a question asked and got wrong earlier the same day: a
`default_beam_angle` on `FixtureType` does **not** free the viewer from
gdtf-import, because fixture types are built by `fixture_type_from` off a node's
port description and by the demo seed and by nothing else, and there is no
fixture type editor in the frontend, so the field would be written by nobody.

**Then the console can be seen at all.** Three panels that share one open
question — where a per-station diagnostic lives, and whether it reaches a peer —
so the decision gets made once across the three of them. The log panel leads
because it is the only one whose audience has no workaround: `logging.log`
promises a plugin author a log and writes it to a stdout that does not exist
under `pult-gui`, under a packaged `.app`, or in a browser.

**Then measure**, with the instruments built and a real rig to point them at.
The browser half of the stats panel is the instrument that decides this, because
performance-tests doubts the browser and has no other way to look at one.

**Then act on what it found**, as one piece of work rather than three. The viewer
rewrite, disk off the actor, per-source admission and the parallel-render
question are all answers to the same measurement, and doing them together is what
stops each being decided on taste. Note that all three of rig-viewer-fidelity's
arrows resolve before it for the first time: gdtf-import for the beam angle,
mvr-import for a rig worth drawing, performance-tests for whether instancing is
needed.

1. **gdtf-import** — fixture definitions from a file, and the only source of a
   beam angle or a real pan and tilt range there is. → none
2. **mvr-import** — fixtures, positions and geometry into `StagePlan` and the
   asset store. → gdtf-import, for the definitions MVR references
3. **system-logs-panel** — the console cannot show its own log, and on a desktop
   app or a tablet there is nowhere else for it to be. → none, and it leads the
   block because its audience is the one with no workaround
4. **system-stats-panel** — the browser's half is the figure that does not exist:
   frame rate, evaluator time per frame, clock offset. The station's half is a
   read of `frame_costs`, which task 44 publishes and nothing displays. → none,
   and it is what makes item 6 able to see a browser
5. **outputs-viewer** — what actually leaves the console, per universe and per
   node. → none, and it closes the block 3 and 4 open
6. **performance-tests** — 5000 fixtures, and whether the console is still
   comfortable. → system-stats-panel, for the browser figure; and better after
   mvr-import, which is what lets it measure an imported rig rather than only a
   generated one
7. **rig-viewer-fidelity** — beams that read as light, and the two live defects
   in the code it rewrites. → gdtf-import, mvr-import and performance-tests, all
   of which are now behind it
8. **engine-admission** — disk off the actor, per-source admission, and the
   parallel-render question that task 29 answered "no" against a tick that no
   longer exists. → performance-tests, which says which of those is on the path
   of a real show. Partitioning across stations is the fourth question here and
   stays unnumbered: it is worth asking only if item 6 finds a rig one station
   cannot carry
9. **typed-plugin-sdk** — codegen into `plugins/sdk` from the same inventory the
   frontend proxy comes from; the wire stays generic. → none
10. **showfile-management** — versioning, save-as, autosave, backup. → none, and
    what blocks it is a decision rather than code: a checkpoint is either
    session-wide agreed or explicitly per-station, and everything else follows
11. **showfile-assets-folder** — a folder with an assets directory, or one file.
    → decided with showfile-management, not separately
12. **paperwork-export** — patch lists, cue sheets, rider paperwork. A read-only
    plugin over introspection, which is what introspection is for. → none, and
    much better after gdtf-import, which is what puts a real patch in the show
13. **3d-programmer-remainder** — blind, highlight, fan, and modifiers that are
    themselves dynamic. → rig-viewer-fidelity, for anything that happens in 3D
14. **voice-input** — speech to the command line, grammar first and NL on parse
    failure. → none
15. **nl-show-context** — what relative syntax cannot reach, and whether it is
    worth the permission it costs. → voice-input, which is what shows which
    utterances actually arrive
16. **open-control-interfaces** — OSC, MIDI, control surfaces. → none
17. **timecode-workflow** — waveform and beat-grid timecode, timed playback,
    audio import. The biggest item here and the one the spec is most opinionated
    about. → none technically
18. **llm-cost-overview** — token and cost accounting out of the NL plugin.
    → none
19. **openhaunt-as-plugin** — output connectors as WASM, if a connector's own
    frame rate survives the boundary. → the benchmarks from tasks 43 and 44 and
    from performance-tests, which are what decide it
20. **video-mapping-ndi** — NDI output. Scope carefully, it hides a media server.
    → openhaunt-as-plugin, as the first proof the plugin API carries heavy output
21. **plugin-language-hosts** — TS plugins, via a host plugin or as components.
    → a real TS plugin wanting to exist

One thing does not belong to any phase. The `<T.SpotLight>` mounted inside `{#if
beam.output.level > 0.01}` recompiles every material in the scene when a fade
crosses 1%, which is a fade from black, and the fix is one line. It is written up
under rig-viewer-fidelity because that is where the context is, and it should not
wait for item 7.

### Plugins

#### typed-plugin-sdk

Introspection is the right wire and a poor thing to program against. A plugin
learns the schema from `introspection::entities()` as JSON and navigates it by
hand, with no types, no compile-time field names, and stringly-typed paths
(`&["cues", id, "fade_time"]`). Every plugin author pays for that.

The fix is codegen into the **SDK**, not the WIT. `pult-codegen` already
generates `frontend/src/lib/ws/data.ts` from the `EntityMeta` inventory, giving
the frontend `data.sequences[5].cues[3].fadeTime.set(4)` while the WebSocket wire
stays generic path-plus-JSON. `plugins/sdk` can have that same split, from the
same inventory and the same tool: `sdk::data::cues().nth(3).fade_time().set(4.0)`
over an unchanged `data.set(path, json)`.

- Codegen'ing the WIT itself is ruled out, and task 35 records why. A
  component's imports are stamped with the package version, and a record type's
  fields are part of every signature using it, so `Cue` gaining a field would be
  a breaking ABI change. The schema changes daily and a show now carries its
  plugins between machines, so a bundle built against schema-of-Tuesday would
  refuse to load on a station from schema-of-Wednesday. Today no plugin notices
  the schema growing, and that property is worth keeping.
- Introspection stays. It answers the runtime question, which is what *this*
  station has including collections the SDK never heard of. A command-line plugin
  building its grammar and a sync plugin walking unknown tables both need that.
  Typed codegen is for what is known at build time.
- How does the SDK version relate to the station's? A plugin built against a
  newer SDK writing a field an older station lacks gets a per-path runtime error,
  the same graceful failure the frontend already has. Worth confirming the
  message is good.
- Where does the generated code live, checked in beside `sdk/src/lib.rs` or
  emitted into `OUT_DIR` by a build script? Checked in matches how the frontend
  does it and keeps the plugins workspace buildable without the console's.

#### openhaunt-as-plugin

Should the OpenHaunt output path, and Art-Net and sACN with it, be WASM plugins
rather than built-in connectors? The spec calls output a plugin layer already;
today `OutputPlugin` is a Rust trait inside the backend.

- Measure the 40 Hz output path through a WASM boundary before deciding. Tasks
  43 and 44 have the numbers to compare against, and they are not the numbers
  this question was first asked against: a connector's cost is now its own frame,
  2.86 ms for 2005 fixtures, not a share of a tick.
- Discovery over mDNS and the embedded MQTT broker are harder to host in a guest
  than frame emission is.
- Middle ground: keep the connectors native and expose the same registration to
  plugins, so a *new* protocol can be a plugin without moving the built-ins.

#### plugin-language-hosts

A plugin that hosts other plugins in another language, say a TypeScript host
embedding a JS runtime so plugins can be written in TS on top of it.

- Does the host API, meaning permissions, introspection and surfaces, pass
  through cleanly, or does the host become a second plugin API that drifts?
- Alternative: componentize JS directly with jco or StarlingMonkey, so a TS
  plugin is a component and no host plugin exists.
- Defer until a real TS plugin wants to exist.

### Natural language and voice

#### voice-input

Voice as an input path to the command line. The question that shapes it: an
utterance may already be valid command-line syntax, and with the NL plugin
installed it would go to the LLM anyway, costing money and latency for nothing.
Without the NL plugin it should parse directly.

- Route: try the grammar first and fall back to NL only on parse failure?
- Where speech-to-text runs, in the browser through the Web Speech API, on the
  station, or in a plugin.
- Push-to-talk against a wake word, and confirmation before destructive commands.

#### nl-show-context

"A bit darker" needs the current value and the NL plugin has none.
`plugins/natural-language-control/pult-plugin.toml` grants no data access
(`commands = false`) by design; everything goes through the command line.

Task 39 answered most of this. `at +10` and `at -10` are command-line syntax, so
"a bit darker" is an utterance the plugin can answer with no show data and no new
permission, and the one grammar and one audit trail survive. What is left is the
part relative syntax cannot reach, "make it look like the second verse", which
needs the show, and whether that is worth the safety story it costs. The
alternative is still read access with state in the prompt, which weakens the
story and grows the prompt with the rig.

#### llm-cost-overview

Token and cost accounting for the NL plugin, visible over the REST API.

- Where it is measured. The plugin sees the usage fields on each response; the
  host sees only bytes. So the plugin reports, into LOCAL state, and a
  `GET /api/...` beside `/api/config` and a panel read the same numbers.
- Per session, per show, or per station? And cost tables per provider and model
  live where, kept up to date by whom?

### Programming model

#### 3d-programmer-remainder

What is left of the spec's §Programming once the rig view (task 13), programming
in it (task 14) and effects over a selection (task 25) are done: **blind**,
**highlight** and **fan**, and modifiers that are themselves dynamic, meaning an
effect whose rate is an effect.

- Blind wants a second programmer buffer that does not reach the output. Is that
  a second `programmer_values` collection or a flag on the existing one? It is
  SYNCED either way, so two operators can be blind separately or not at all,
  and which of those is right is the decision.
- Highlight is a temporary output override for the selection, the same shape as
  home (`__home`) pointed the other way. Reuse that machinery or not?
- Fan needs an order over the selection, and `SelectionQuery` now carries one
  from task 38. Does fan reuse it, and what does fanning an unordered selection
  mean?
- A dynamic modifier is a graph rather than a value, and nothing in the schema is
  recursive yet.

### Visualisation

#### rig-viewer-fidelity

The 3D rig viewer draws a beam as a `ConeGeometry` wearing a flat additive
material (`frontend/src/lib/components/stage/Rig3D.svelte`). That is enough to
say where a light is pointing and not enough to look like light. Prior art read
in full on 2026-09-01: ASLS Studio's visualizer (`src/plugins/visualizer/`, about
2.7k lines), which is this problem solved a level up. It is GPL-3.0 and we are
MIT, so what travels is the technique, not the code.

What they do, in the order it matters to us.

- The beam is not geometry. One 100 m open-ended cylinder, instanced, and the
  beam angle is vertex displacement: the far ring is scaled by `tan(angle)` in
  the vertex shader. Zoom costs a float in an attribute and nothing is rebuilt.
- Brightness depends on where you stand. Four terms multiply together: how
  side-on the beam is seen, how near the camera is to looking down the barrel, an
  inverse-square-ish falloff along its length, and a power term on the silhouette
  so a cylinder stops reading as a tube.
- Haze is four octaves of 3D simplex noise sampled in world space with *time as
  the third axis*, so it drifts. Density and turbulence are the two knobs.
- The beam smoothsteps out over the last centimetre above the deck instead of
  clipping through it.
- Colour is scaled in HSV, value only, so a dim beam keeps its hue rather than
  crushing towards grey the way scaling RGB does.
- Base, yoke and head are three `InstancedMesh`es sharing one material, with
  per-fixture state in `InstancedBufferAttribute`s, and the model articulates:
  the yoke swings on pan, the head nods on tilt.
- Selection is one per-instance float that an `onBeforeCompile` patch turns into
  emissive. No material swap and no extra draw call.

Two things about them are worth not copying. Their README credits the
`postprocessing` library and there is no `EffectComposer` anywhere in their
`src/`; every bit of glow is additive blending in one fragment shader, which is
the cheaper lesson. And their fixture bodies are pure black, so the render cannot
tell you what is hanging up there. Our emissive body tinted by its own output is
the better call and should survive whatever else changes.

Two defects in ours turned up while comparing, and both are still there.
Corrected on 2026-09-02: an earlier version of this entry said the second one
went with the task 44 rewire, and it did not. Both are still in
`Rig3D.svelte`.

- **A geometry per fixture per frame.** `<T.ConeGeometry args={[beam.length *
  0.12, beam.length, ...]}>` has reactive `args`, so Threlte rebuilds the
  geometry whenever the throw changes. Dragging a beam spot allocates a fresh
  cone per fixture per frame, and since task 44 the throw is re-evaluated every
  animation frame, so a fade does it too.
- **Every material recompiled at 1%.** The `<T.SpotLight>` is inside `{#if
  beam.output.level > 0.01}`, so crossing that threshold changes the scene's
  light count, which changes three.js's program cache key and recompiles every
  material mid-fade. It fires on the most ordinary thing a console does, a fade
  from black, and the fix is one line: keep the light mounted and drive its
  intensity to zero, so the count is constant.

They were briefly split into an item of their own on 2026-09-02 and folded back
the same day, and the reason is worth keeping. **The first one is deleted by the
work above rather than fixed by it**: the beam stops being geometry, so there are
no reactive `args` left to rebuild. Fixing it separately is writing code this
entry throws away. The second is a one-line fix that the rewrite may or may not
subsume, depending on whether the floor pool survives as a real light or becomes
part of the beam shader, so it is worth doing whenever somebody is next in the
file rather than waiting for anything here.

What does *not* depend on any of this is the three cheap wins at the end of the
open questions. They touch the gizmos, the camera and the grid, not the beam,
and nothing above deletes them.

Open questions.

- Beam angle has nowhere to come from. `FixtureType` carries no beam angle and
  `ParameterKind` has no `Zoom`, so everything is drawn at the hardcoded
  `length * 0.12`, a 6.8° half-angle, and a wash looks like a beam. That is a
  `pult-schema` change and it is the same one gdtf-import wants. Do they land
  together, or does a `default_beam_angle` on `FixtureType` come first?
- Where does haze live? A station preference seeded into the show the way
  `home_fade_ms` is, or a per-browser view setting? How hazy the room is is a
  fact about the room, which argues for the show, but two operators on two
  tablets may reasonably want different pictures.
- Instancing against the derived `beams` array, which performance-tests should
  decide rather than taste. Every frame rebuilds a
  `Quaternion`, an `Euler` and a `Color` per fixture, sixty times a second now
  rather than forty, since the viewer draws its own frames rather than waiting to
  be pushed values. Instanced attributes are the fix, and they sit badly with
  Threlte's declarative `#each` and with picking, which raycasts against
  per-fixture objects. Does the viewer drop to imperative three.js inside one
  Threlte component, and what happens to the gizmos if it does?
- What is already done for you. The evaluator is in the page: `stores/output.ts`
  registers what a panel is showing and evaluates all of it in one wasm crossing
  per frame, 200 parameters in about 17 µs, and `Showing.at` is `null` while the
  browser cannot place itself on the station's clock. A beam that is drawn is a
  beam that was evaluated for the moment it is drawn at, which is what this item
  wanted.
- Their singletons do not survive the move. `SceneManager`, `Controls` and
  `AnimationManager` are module-level globals over shared mutable buffers, fine
  for one viewport and broken in our tiled workspace, where two `rig` panels can
  be open at once. Anything we take has to be per-panel.
- Placement, as opposed to aiming. They have `TransformControls` with a 0.5 m
  translate snap, keyboard modes, and multi-select through a bounding-box group
  so a whole truss moves together. We have pan, tilt and spot gizmos for aiming a
  head and nothing for rigging one in 3D. Same change or its own?
- Strobe needs a `ParameterKind` before it can be rendered at all; theirs is a
  square wave against the animation clock driving the intensity attribute. Out of
  scope here, or the reason to do that schema work once?
- Three cheap wins need no design and could go in ahead of the rest.
  `depthTest: false` on our existing gizmo rings so they are never buried inside
  a fixture body. Cancelling a `follow` camera transition on any pointer or wheel
  input. And an infinite grid shader, with `fwidth` line antialiasing, two scales
  and a distance fade, to replace the fixed `GridHelper`, which aliases badly
  past about 40 m and stops at the edge of the plan.

### Showfiles

#### showfile-management

Versioning, backup, automated backup to an external drive. Today there is one
SQLite file, written on every PERSISTED write, with **no explicit save at all**:
no `save` RPC, and nothing defers a write.

- **Save should mean checkpoint, not flush.** The want is committed intent. Try
  something in rehearsal and discard it, name a version, get back to the show as
  it was at the end of yesterday. The want is *not* deferred durability. A show
  that loses an evening's programming because nobody pressed Save is the worst
  failure this console has, and it happens exactly where people forget, on a long
  tech, late, everyone tired. So keep writing continuously as the crash journal
  and let Save mark a point, rather than making the write wait for a keypress.
- There is no performance case for deferring either. Task 44 took the tick off
  the write path entirely, and operator edits happen at human rate.
- **Revert-to-last-save wants the oplog, not a second history.** The log is
  already per-node sequenced and already bounded by task 37's retention, so a
  checkpoint is a marked seq and reverting is a rewind, the same machinery undo
  uses.
- The hard part, and the reason this cannot be a small change: **the show is
  replicated live.** If one console defers or reverts while another saves, what
  got saved? A checkpoint is either session-wide agreed or explicitly
  per-station, and that decision drives everything else here.
- Save-as, snapshots, autosave cadence, and what a "version" even is when the
  show is also replicated live to peers.
- Backup target configuration is a station preference; task 33's
  `preferences.toml` is the home.
- Restore: open a backup read-only, or roll the working file back?
- Whether a backup is also an oplog prune point. Task 37 answered pruning on its
  own; this only has to say how the two meet.

#### showfile-assets-folder

Assets are a blob table inside the SQLite file (task 13), addressed by sha256.
The question is whether a showfile should be a *folder* with an assets directory
instead, zipped on export, and what that does for dedup across versions.

- Content addressing already gives dedup, and versioned backups of a folder share
  unchanged assets naturally through hardlinks or a store-once layout.
- A single file is robust against half-copies; a folder is friendlier to rsync
  and to looking inside. Export-as-zip can exist either way.
- Decide it together with showfile-management, not separately.

### Interop

#### gdtf-import

GDTF fixture definitions, native or as a plugin. `FixtureType` is derived data
today, because OpenHaunt nodes describe themselves; GDTF is the same idea as a
file, where the description becomes a fixture type.

- Real pan and tilt ranges fix the 540°/270° constants task 14 complains about.
- **It is the only source of a beam angle there is.** Adding
  `default_beam_angle` to `FixtureType` instead was tried on paper and does not
  work: fixture types are built by `fixture_type_from` off a node's port
  description and by the demo seed, and by nothing else, so a field nobody writes
  is the hardcoded cone with extra steps. A hand-authored catalogue would also
  answer it, and is the alternative worth naming, but no catalogue exists and
  this entry is what would bring one.
- Channel-mode selection, wheels, and physical data: how much of GDTF maps onto
  `ParameterDefinition` before it has to grow?

#### mvr-import

MVR, My Virtual Rig. Task 13 noted that `StagePlan` and the asset store are what
an import needs and that nothing is in its way.

- It brings fixtures, positions and 3D geometry, mapping onto `Fixture::position`
  and plans. The GDTF references inside an MVR need gdtf-import first, or stubs.

#### paperwork-export

Patch lists, cue sheets, rider paperwork.

- A read-only report is an ideal plugin, since introspection already exposes all
  the data. The open part is print CSS against generating a PDF.

#### open-control-interfaces

OSC, MIDI and control surfaces alongside the existing WebSocket API.

- Native connector or plugin? This argues with openhaunt-as-plugin, and the
  answer may differ: a control surface is input rather than a 40 Hz output path,
  so the latency case against a WASM boundary is much weaker here.
- What does an OSC address map onto, the path API directly
  (`/pult/sequences/3/go`), or the command line, which already has one grammar
  and one audit trail?
- MIDI needs a device on a particular station, so a surface is LOCAL to whoever
  it is plugged into while the thing it drives is SYNCED. Same shape as a fixture
  connector.
- Learn mode, press a fader and bind it, is the UX that makes it usable, and it
  is a write to the show. Which collection?

### Observability

#### outputs-viewer

A live view of what leaves the console: a DMX sheet per universe, OpenHaunt
messages per node. The dedup caches in `connectors::dmx` already hold the current
universe images; OpenHaunt sends are discrete messages worth a ring buffer.

- LOCAL state on the owning station, with the viewer subscribing cross-station.
  The latency numbers set the precedent: a link property is published by whoever
  measured it.
- 40 Hz times 512 bytes should not hit the WebSocket unthrottled. Snapshot on
  demand, or diff at panel rate.

#### system-stats-panel

The Stations panel (task 10) has CPU, memory and uptime. Missing: network
throughput, sync backlog, WebSocket client counts, broker stats.

- **Frame cost is done and the shape question is answered.** Task 44 put it on
  the `Station` row as `Vec<FrameCost>`, one entry per connector, each with the
  mean, the worst, the evaluating half of each and the frame count for the
  window, on the grounds that a station is already the sole authority on its own
  numbers there. So extend the row rather than adding a LOCAL collection, unless
  something arrives that a row genuinely cannot hold. A ring buffer of recent
  frames would be that.
- What is left is the panel: nothing in the frontend reads `frame_costs` yet.
  Absent has to render as absent, because a settled connector is not an instant
  one, and a station with two connectors shows two rows rather than an average.
- Sample rates for the rest. `REPORT_INTERVAL` is two seconds and everything on
  the row shares it.
- **The browser's load belongs here too, not just the backend's.** Since task 44
  a console *is* a browser evaluating a rig at frame rate in wasm, and that is a
  real cost on a real machine. A tablet at the back of the room can be the thing
  that is struggling while every station is comfortable.
  - What a browser can honestly report about itself: frame rate and dropped
    frames from `requestAnimationFrame` deltas, time spent in the evaluator per
    frame, how many parameters it is evaluating, `performance.memory` where the
    browser offers it, and its measured clock offset from the station, which is
    the one number that says whether what it is showing can be trusted at all.
  - Where it lives: a browser is not a station and must not appear in `stations`.
    A LOCAL collection keyed by WebSocket session is the obvious shape, published
    by the client and owned by the station it is connected to, which also makes
    it disappear correctly when the tab closes.
  - Open: does a client's report replicate to peers, so any console can see that
    the tablet is struggling, or is it LOCAL to the station serving it? Seeing it
    from anywhere is the useful version and costs a row per client per session.

#### system-logs-panel

Nothing in the console shows the console's own log. `tracing` writes to stdout,
in `pult-backend/src/main.rs` and `pult-gui/src/main.rs` alike, filtered by an
`EnvFilter` built once at startup with `pult_backend=debug` and whatever
`RUST_LOG` says. Nothing captures it, and `scripts/demo.sh` redirecting each
component into `.demo/*.log` is the only place a line is ever kept.

**Which means that on every way of running this that is not a terminal, the log
does not exist.** `cargo run -p pult-gui` writes to a stdout nobody is looking
at, a packaged `.app` from the release workflow has nowhere to write it at all,
and a browser on the network, which is a whole console by design, has no access
to the station's stdout on any machine. A rig is consoles in racks and tablets in
the room.

**And plugins are already logging into it.** `wit/pult-plugin.wit`'s
`logging.log` says its message "lands in the station's log, prefixed with the
plugin id", and `host_impls.rs:799` puts it through `tracing` with
`[plugin:<id>]` in front. So a plugin author debugging a plugin is debugging into
a void unless they happened to start the station from a shell. That is the
strongest argument for the panel, because it is the audience with no workaround.

**Not the History panel.** That is the oplog: who changed what, per person,
undoable, replicated, pruned on its own retention. This is diagnostics, per
station, not replicated, nobody's to undo, and hundreds of lines a second at
`debug`. Two panels, and this says so because "we have a history panel" is the
obvious wrong answer.

What made it worth writing down: task 44 ended with two failures whose only trace
was a `WARN`, one of them the address bug at the end of it. The join now answers
for itself, but a peer lost
mid-show, an output whose socket would not bind, a node that stopped answering, a
showfile migration that complained, all of them are lines nobody sees. The cases
that matter are exactly the ones where the console *keeps working*, because a
crash at least announces itself.

Open questions.

- **Where do the lines live?** Not the oplog, for the reasons above. A LOCAL ring
  buffer published like `output_status` is the obvious shape, but LOCAL state is
  replaced whole on every write and this is an append-only stream. Replacing a
  thousand-line buffer per line is not a mechanism, it is a mistake. Does this
  want a subscribe-only stream over the WebSocket instead, which is a new shape
  in the protocol and should be resisted until it is plainly needed?
- **Kept where, and for how long?** In memory only, or a file beside
  `preferences.toml`? A file survives the crash that is the reason somebody went
  looking; memory does not. `.demo/*.log` is the shape of the file version and it
  is per run, which is probably right.
- **What level, and who chooses?** `pult_backend=debug` is loud, a line per write
  and a heartbeat every five seconds per peer, and a panel showing all of it is
  unreadable. A `log_level` station preference is the obvious home, this
  machine's business the way `oplog_retention_minutes` is. Changing it while the
  show is up means `tracing_subscriber::reload`, since the filter is built once
  at startup. Worth it, or is a restart acceptable for a diagnostic setting?
- **Does a peer's log reach this console?** Reading the roof station's log from
  the booth is the useful version, and it is the same argument system-stats-panel
  makes about a browser reporting its own load. It is also a great deal of
  traffic, and a question about what a log line carries: a path, a hostname,
  whatever a plugin chose to say.
- **Filtering by plugin is nearly free**, because the prefix is already there.
  Worth making a first-class filter rather than a search box, given who needs it.
- **The browser's own errors.** A console is a browser, and an exception inside a
  panel is invisible to the operator and to the station. Same panel, or out of
  scope? It is the same question system-stats-panel asks about frame rate and
  evaluator time, and the two should probably be answered together.

### Performance

#### performance-tests

The target is a number, and it is **5000 fixtures with the show still feeling
immediate**: a cue taken without a visible stutter, an output frame inside its
budget, and a rig panel holding its frame rate on a machine somebody would
actually put in a booth. Nothing has been measured above 2005.

**The instrument mostly exists.** `scripts/demo.sh --measure --release --size
huge` seeds 2000 fixtures across 24 universes, drives every sequence to a cue
with an effect running, seeds an Art-Net output at loopback so there is a frame
to measure at all, and prints what one cost. It reads the station's own published
`frame_costs` over the same WebSocket a browser uses, so the figure printed is
the figure the Stations panel shows and the figure a peer sees. Where it is
wrong, it is wrong everywhere, which is the property worth having.

What that instrument said on 2026-09-02 at 2005 fixtures, in release: 2.86 ms per
output frame at 34 Hz, of which evaluating is 2.60 ms and putting it on the wire
0.26 ms, against a 25 ms budget. And zero updates to a connected browser across
four seconds of a running fade.

**The naive extrapolation, which is the thing to go and disprove.** Evaluating
looks linear in the rig, so 5000 fixtures is around 6.5 ms of a 25 ms frame:
comfortable. 5000 six-channel heads is about 59 universes rather than 24, and
per-universe assembly and 59 socket writes are the part of the frame that does
not shrink. So the prediction is that the station is fine and the *browser* is
not, and the point of the work is to find out where that prediction is wrong
rather than to confirm it.

What to measure, roughly in the order the answers matter.

- **A bigger preset.** `--size` takes small, big and huge today, at 5, 500 and
  2000. Either a fourth name or `--size <n>`, and `<n>` is more useful here
  because the shape of the curve is the answer, not one point on it. Whether the
  cue count and the slice share scale with the rig or stay put has to be decided
  deliberately: 300 cues times 5000 fixtures is a million and a half captures,
  which measures JSON rather than lighting, and task 43 already made that mistake
  once on purpose to see what it looked like.
- **Where the frame goes at 5000.** The evaluating and emitting halves are
  already published separately, and that split is what saved the last round of
  this work from being spent on the wrong half. A third figure, per-universe
  assembly against the socket write, is probably what this round needs, and it
  should be added the way the reading/computing/applying split was: temporarily,
  by hand, and then permanently if it turns out to be the one that matters.
- **The write path, not only the frame.** Seeding 2000 fixtures takes about 43
  seconds in release through the WebSocket API, pipelined through a window of 64.
  That is the largest exercise of the write path in the repo and it is a real
  measurement, not overhead to be tolerated. Patching 5000 fixtures is something
  somebody does, and so is taking a cue that touches all of them at once.
- **The browser.** No figure exists at all, because `--measure` deliberately
  stops the dev server and the sims so they are not taking the CPU being
  measured. Which is why system-stats-panel now sits ahead of this item rather
  than behind it: the browser reporting on itself is the instrument, and there
  is no other one.
- **An imported rig, not only a generated one.** gdtf-import and mvr-import are
  ahead of this item now, which means a real plan with real fixture types can be
  the thing measured. Worth doing both: the generated rig is the one whose shape
  can be dialled, and an imported one is the only check that the generated shape
  resembles a rig anybody hangs. The two numbers wanted are the evaluator crossing per frame
  (`stores/output.ts` evaluates 200 parameters in about 17 µs, so 5000 fixtures'
  worth of a rig panel is roughly 2.5 ms of a 16.7 ms frame at 60 Hz, if it is
  linear) and everything the viewer does around it, which is where the doubt
  actually is.

**How this meets the two items either side of it.**

- **engine-admission.** If a cue over 5000 fixtures stalls behind a plugin's
  write loop or a peer's catch-up, per-source admission is the fix and this is
  what proves it. If it stalls on an fsync, the writer task is. If it stalls on
  neither, engine-admission is still worth doing and stops being urgent.
- **Threads, which have been answered "no" twice and deserve asking again.** Task
  29 rejected parallelising the render because it was 0.07 ms of a 35 ms tick,
  and task 44 removed the field the other half of that proposal wanted cheaper
  writes for. But evaluating is now **91% of an output frame** rather than 0.2%
  of a tick, and it is embarrassingly parallel across fixtures: `pult-render` is
  pure, takes no locks and touches no OS. So rayon over fixtures inside a
  connector's frame is a genuinely different question from the one that was
  refused, and 5000 fixtures is where it gets asked. Measure before deciding, and
  note that a connector's thread is already off the engine, so this costs nothing
  architecturally.
- **rig-viewer-fidelity.** The viewer rebuilds a `Quaternion`, an `Euler` and a
  `Color` per fixture per frame. At 5000 fixtures that stops being untidy and
  starts being the frame budget. But it also still allocates a cone per fixture
  per frame and recompiles every material when a fade crosses 1%, and both of
  those are in code that entry rewrites. So measure the **evaluator crossing**
  hard, because that figure survives whatever the viewer becomes, and treat what
  the current beam drawing costs as the disposable half. The one question this
  measurement should still settle for that entry is instancing, which is a
  question about 5000 `Quaternion`s and not about the cone. Whether the viewer has to go imperative and
  instanced, which is that item's hardest open question, is a decision this
  measurement should make rather than leave to taste.

**This is where the acting phase begins.** rig-viewer-fidelity and
engine-admission both sit immediately after this item and are both answers to it,
which is deliberate: the viewer's instancing question and the engine's
parallel-render question are the same kind of question, and neither should be
settled on taste. Whatever this measures, those two are what it is measured for.

**Not a CI gate, and task 43 explains why.** Two identical `huge` runs varied by
more than a percentage point of CPU and fifteen milliseconds of tick. A threshold
in milliseconds on a shared runner flaps, a flapping gate gets disabled, and a
disabled gate is worse than none. So this is a script somebody runs before a
release and records the numbers from.

What *could* be a gate is a figure that does not flap, and task 44 produced
exactly one: **zero updates to a connected browser during a four-second fade on a
2000-fixture rig.** Counts of messages, of allocations, of universes touched, of
oplog rows written are machine-independent, and a regression in any of them is
the kind of thing that used to be found in a theatre. Worth finding the two or
three that are worth asserting on, and asserting on those instead of on
milliseconds.

Open questions.

- Is 5000 the right number, or is the honest target "as many as one Art-Net
  network can carry"? 59 universes is already past what a single 100 Mbit segment
  is comfortable with, which makes this partly a networking question rather than
  a CPU one.
- Does the target mean 5000 on one station, or 5000 across a session? Splitting a
  rig between consoles is the partitioning question below, and if the answer to
  this item is "one station cannot", that question stops being hypothetical.
- Which machine is the target? A figure with no machine attached means nothing,
  and "a machine somebody would put in a booth" needs naming: the release
  workflow builds for four targets and an aarch64 Linux box is a very different
  answer from an M-series laptop.
- What does the tablet at the back of the room have to manage? A browser is a
  whole console by design, and the weakest one in the building is the real
  target. That is the same argument system-stats-panel makes, and this is the
  item that gives it a number.

#### engine-admission

What survives of a plan called tick-isolation, which was written against the
architecture task 44 left behind. It is worth reading that history before picking
this up, because most of what it proposed is now unnecessary rather than
optimised: the typed `PlaybackView` existed to make a per-tick read of the show
cheap and there is no per-tick read; batching per-tick writes has no per-tick
writes to batch; and playback on its own thread became a loop inside each output
connector, which already had one. The engine has no periodic work left except the
sampling flow `Watch` nodes need, and that is proportional to what is watched.

Two things survive, and they are worth doing on their own terms.

- **Disk off the actor.** `persist`, `oplog::append` and `order::save` are
  awaited inside the actor's command arm against a pool of `max_connections(1)`,
  so one operator's edit waits behind another's fsync. Lower priority than it
  looked, since the disk is no longer anywhere near the show, but it is still an
  operator waiting on a disk. The fix is a single writer task with an ordered
  queue: still ordered, still durable, no longer between a command and its reply.
- **Per-source admission.** Plugins reach the engine through the same
  `EngineHandle` a browser does (`host_impls.rs:207`), into one 256-deep channel
  with no priority. A plugin in a write loop, a browser fetching the whole show
  and a peer catching up all queue together. Give each source its own budget so
  no one of them can crowd out an operator. This is the largest thing left here,
  and nothing since has touched it.

`OutputHandle::push` is the model for both: a `try_send` that drops when the
consumer is behind, documented as "Never blocks the engine" in
`connectors/mod.rs:82`. Frames keep leaving whatever the engine is doing.

One non-goal from the original plan still holds: no new durability guarantee, and
a write that was acknowledged before still is. The other, no parallel render, has
stopped being obvious. It was written when evaluating was 0.07 ms of the engine's
tick; it is now 2.60 ms of a connector's 2.86 ms frame, which is somebody else's
thread but is still the frame. performance-tests is where that gets asked again,
and it is also what says whether either half of this item is on the path of a
real show or merely untidy.

#### Partitioning computation across stations

The one part of the old multithreading item that nothing has answered.
Parallelising the render was answered with **no** (there was nothing there to
parallelise, and task 44 removed the field the other proposal wanted cheaper
writes for), but splitting the work of a show between consoles is task 10's
question and also the redundancy one. Worth asking again only when there is a
workload a single station cannot carry, and the numbers for that will be
different from any measured so far.

### Media and time

#### timecode-workflow

The big one, and the spec is opinionated about it: waveform and beat-grid
timecode, plus "timecode without timecode" meaning timed cue playback, with audio
import and playback. `FollowMode::Timecode` has existed unimplemented since task
3, deliberately waiting for this design rather than getting a stopgap. It should
subsume that follow mode rather than sit beside it.

- Audio import lands in the asset store. Playback happens on which station, and
  what do the others chase? The OpenHaunt clock topic and `went_at` anchoring are
  the prior art for shared time.
- Beat grids relate to speed masters; tap tempo is a degenerate beat grid.
- Is external SMPTE or MTC in scope, or explicitly out?

#### video-mapping-ndi

NDI output for video mapping. Almost certainly a plugin, which would be the first
real test of whether the plugin API can carry a heavy output, or else a sibling
connector.

- Frames come from where, a pixel-mapped fixture array rendered by the engine, or
  media playback? Scope this carefully. It hides a media server.
