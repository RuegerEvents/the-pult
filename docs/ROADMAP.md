# Roadmap

State of the system and what to work on next. Reconstructed from the code on 2026-08-25, then reconciled against [SPEC.md](SPEC.md).

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
| Frontend | Working for show, session, sequences, cues, patch, and the programmer. A tiled workspace of resizable panels replaced the sidebar and tabs; layouts are saved in the showfile. The typed proxy runs end to end. Vitest covers the pure helpers; components are untested. |
| Playback engine | Working. Fades, active-cue tracking, and FollowAfter cues at 40 Hz. |
| Output plugins | Working for Art-Net, sACN, and OpenHaunt nodes, several at once. Configured from the `outputs` collection and editable while the show is up, with per-output status in the UI. Flags only seed an empty showfile. |
| Stage view | Working. A ground plan is uploaded, calibrated against something of known length, and fixtures are dragged onto it — then the same rig in 3D from front of house, beams and all. |
| Flows | Working. The spec's node graph, evaluated as a graph: sources, conditions, boolean logic, delays and actions, with live state on every node. Replaced `triggers`. |
| Devices / events | Working. OpenHaunt nodes are discovered over mDNS and adopted as fixtures; their inputs land in `live_values`; flows turn those into cues. Tested end to end against `tools/openhaunt-sim`, which is all there is until there is firmware. |
| WASM plugins | Not started. `infra/plugins/mod.rs` is a stub. |
| 3D programmer | Working in outline. A shared programmer buffer beats playback, and pan and tilt are puppeteered by grabbing a ring, an arc, or the beam spot on the floor — in the rig and on the plan. Effects, phasers and geometric selection are still ahead. |
| Selection | Working as a list: ordered, reorderable, its own panel, kept apart from the programmer. Still a list of ids rather than the geometric query the spec asks for. |
| Effects, timecode | Not started. No schema for either yet. |
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

### 8. WASM plugins

Nothing else depends on it, and the plugin API should be designed against a system that already plays back cues and drives output.

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
- `tools/openhaunt-sim`, a simulator implementing the node side of the protocol, so the whole path is covered by tests without hardware on the bench.

All of that is in. `FixtureAddress` and `ParameterDirection`/`ParameterBinding` went in first, with the two migration paths they needed — a hand-written `Deserialize` for the JSON column and `showfile::upgrades` for the real ones. `types::openhaunt` is the only place that knows what a module id means. `DeviceManager` browses, adopts, and drives; `SetLiveValue` merges an input inside the engine actor and replicates it; `model::triggers` evaluates the rules in the engine's own tick beside playback.

What it leaves open:

- ~~The node-graph UI.~~ Task 12.
- Per-pixel WS2812. A strip is one colour and one brightness.
- OSC and MIDI as trigger sources. `TriggerSource` is an enum with one variant so they can be added beside `Parameter` without touching anything else.
- RDM, which the gateway module's `caps` advertises and nothing here uses.
- A running fade and a `SetParameter` trigger writing the same key: last writer wins, and it is the fade, because it writes on every tick. Documented rather than solved — deciding what *should* happen is a product question.
- The broker is started once per process and never stopped. A node adopted by a previous leader is re-configured on promotion, but a follower keeps a broker it started while it was leading.

Assumptions made about the protocol, to be fed back into the OpenHaunt docs since there is no firmware to check them against: `/api/v1/config` takes `{ mqtt: { broker }, dmx?: { protocol: "sacn", universe } }` and persists it; the mains flag is descriptor bit 6, reachable through `GET /api/v1/info` as `module.flags` (a `mains=1` TXT key would let the panel warn without the round trip); input events are `{ state, edge, ts }` and sensor readings `{ value, unit, ts }` on the same `input/<n>` topic; output payloads are `{ state }` for a relay, `{ r, g, b }` and `{ brightness }` on ports 0 and 1 for a strip, and `{ text }` for a display; `POST /api/v1/state` takes `{ outputs: { "<n>": payload } }`; `status` is the literal `online`/`offline`, retained, with `offline` as the will; health is `{ uptime_s, temp_c, poe_class, errors }`; the DMX module lists `sacn` in `caps` and listens on unicast 5568 for its configured universe; and TXT `sn` matches the instance short serial, one module per node.

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

**The simulator has a window.** `tools/openhaunt-sim-gui`, and the thing it fixes
is small and real: `openhaunt-sim` can only be driven by typing at its stdin, and
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

The cost of finding them was five tags. The alternative — a Docker container
reproducing the Linux half — found the worst one in a minute, and is worth
reaching for before pushing the next one.

Left open:

- **Nothing is signed or notarized.** macOS and Windows both warn on first run.
  The workflow is laid out so this is adding secrets and a few steps.
- **No auto-update.** `latest.json` and a Tauri updater keypair are the next step.
- **32-bit Raspberry Pi is not built.** The arm64 runner is aarch64, which is
  64-bit Raspberry Pi OS.
- **The simulator's panel cannot restart the node.** `openhaunt_sim::start` has no
  shutdown, so a module is chosen with a flag and lives for the process. Adding
  cancellation is what settles it, and it would serve the tests too.

## Further out

Everything below is in the spec and has no schema and no code yet. Listed so the near-term work does not paint itself into a corner.

**Selection as a geometric query.** Selections are meant to be generated from the rig by geometric functions and re-evaluated as the rig changes, not stored as fixture lists. That is a query language. It needed positions first, and task 13 is where a rig finally gets them; task 14 built the panel that will show the result, as the list of ids that comes first.

**Effects and phasers.** Derived from the 3D selection with modifiers that can themselves be dynamic. Needs selection.

**3D programmer.** The rig *view* is task 13 and programming in it is task 14 — the camera frames a picked fixture, pan and tilt are grabbed by ring and arc, and the quicksheet opens at the light. What that leaves is the rest of the spec's §Programming: effects and phasers over a selection, and the parts of a fixture that are not intensity, colour and position.

**Waveform timecode and "timecode without timecode".** Beat grids, markers, live audio analysis for band sync. Should subsume the `Timecode` follow mode rather than sit beside it.

**Open control interfaces.** OSC, MIDI, and control surfaces alongside the existing WebSocket API.
