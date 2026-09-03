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
- **`crates/pult-gdtf`** — GDTF, read and written. A pure format library: `quick-xml`, `serde`, `zip`, `uuid`, `thiserror`, and *no pult crate* — which is what lets it be tested against other people's files with no station near it. Writing is why it exists rather than a crate off crates.io. The translation into the schema is `crates/pult-backend/src/infra/interop/gdtf/`.
- **`crates/pult-mvr`** — MVR, read and written. The other half of the interop pair and
  a pure format library like `pult-gdtf`, depending on it for the fixture definitions
  inside an archive and for the millimetre Z-up to metre Y-up conversion they share.
  `transform.rs` is where a matrix becomes a position, a rotation and a **signed**
  scale — signed because a fifth of the trusses in a real Vectorworks file are
  mirrored, and no rotation is a reflection.
- **`crates/pult-backend`** — A station, as a library and a binary. Axum WebSocket server, `Name.pult` showfile bundles, peer sync (mDNS + TCP), the WASM plugin runtime (`infra/plugins/`), fixture connectors. `pult_backend::start(Config)` brings a whole station up and is what both the binary and the desktop app call.
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

## A fixture type says what a light can do; a mode says where the bytes go

`FixtureType::parameters` is what an operator can set. `FixtureType::dmx_modes` is how
those parameters reach a DMX line in one particular mode, and which mode a given unit
is in lives on its address — `FixtureAddress::Dmx { mode, breaks }`, a place *per break*
because a fixture with a separate dimmer break sits in two spans that need not be in the
same universe.

**A type with no modes still has one.** Everything the console made for itself — a type
derived from an OpenHaunt node, the demo seed, the hand editor — names no mode at all,
and `FixtureType::mode()` computes an implicit `"Default"`: one byte per output
parameter in the order the type lists them, three for a colour, and nothing at all for
a parameter on a module port. Computed rather than stored, so there is one layout
rather than a written one drifting from the parameters it was written from. The browser
does not work that out for itself with one exception, `patch.ts`'s `implicitChannels`,
which the hand editor needs in order to show an operator where the parameter they just
added has landed — and it says so where it is defined.

**A showfile is not a migration target.** While the console is in development nobody is
carrying a season's work in one, and a migration is a promise about every shape the data
has ever had. So there is none: `infra/showfile/mod.rs` stamps a file with
`SCHEMA_GENERATION` and refuses one from another generation, saying so plainly. Two
things make that necessary rather than merely tidy, and both are the SQLite read path:
`#[derive(PultSchema)]` generates `from_columns`, which reads each column on its own and
unwraps — so a non-`Option` column that is NULL **panics while a show is opening**, and
an `Option` column that fails to parse **becomes `None` with no error at all**. The
stamp catches the second, which nothing else can see. `add_missing_columns` stays, since
adding a field is free; what it cannot do is fill one in, so a check beside the stamp
names the first required column nothing filled and refuses that too. Bump
`SCHEMA_GENERATION` when a stored *shape* changes; adding a field is not that.

**A colour is one parameter and several channels.** Every `ColorAdd_*` and `ColorSub_*`
attribute is the fixture's colour; a reader that made three parameters would give an
operator three faders where every other console gives a picker. So the type carries an
emitter list, each channel of a mode names the one it drives, and `pult_render::color`
gets from a colour to a level per emitter — compiled twice like the rest of the
evaluator and held together by `testdata/color-mix.json`.

**A `.gdtf` is kept whole and the row is a reading of it.** The archive lives in the
asset store and `FixtureTypeSource::Gdtf` points at it by sha256, so exporting hands
back the file byte for byte and a later version of this console reads more out of the
same bytes without asking anybody to download anything again. `FixtureTypeSource` also
decides what may be overwritten: a node's own type is rebuilt whenever the node
describes itself again, and doing that to an imported one would throw the file away.

```
cargo test -p pult-gdtf                              # the format library
scripts/fetch-interop-corpus.sh                      # other people's files, gitignored
cargo test -p pult-gdtf -- --ignored                 # against them
```

The GDTF Share needs a login, and it lives in the station's `preferences.toml` and never
in the show — a showfile travels, and a password in one travels with it. Three things
about that server are load-bearing and written down in `infra/interop/share.rs`: its
login answers **200 with an HTML page** when the credentials are wrong, so success is
decided by the body; its list is **tens of megabytes and unfiltered**, so it is fetched
once, cached, and searched locally; and its session **goes idle after about two hours**,
so an unauthorised answer logs in again and retries exactly once.

Both import paths go through `infra/interop/apply.rs`, which is where the rules about
writing live: a plan is built by a pure function before anything is stored, so a
rejected file leaves neither an asset nor a row behind; every write carries one gesture,
so an import is one Ctrl-Z; and a write that fails takes the rest back.

**An MVR is a whole rig, and `POST /api/import/mvr` is the same shape.** Every uuid the
file uses is the id the row gets — an imported fixture's `id` *is* its MVR uuid — so a
re-import updates the drawing rather than doubling it, with no lookup table to keep. A
fixture *type* is the exception and is keyed by the GDTF's own `FixtureTypeID`, since a
drawing can name one definition twice. The file wins on a re-import, and what an earlier
import left in a layer this one no longer mentions is **listed under `missing` and never
deleted**. A fixture whose GDTF the archive does not carry gets a placeholder type, so
the address, the mode and the place survive until somebody supplies the real file.

**And back out.** `GET /api/export/mvr?layers=…` writes the rig as an archive, with
each fixture type's own file where it arrived as one and a generated GDTF otherwise —
the rule `/api/export/gdtf` already follows. Exporting the whole show means the whole
show, including a symbol nothing instances, a class nothing is tagged with, and the
fixtures no layer claims; a *filtered* export carries what its layers use. The proof
is a round trip: every real file in the corpus, imported, written back out and read
again, gives the same fixtures at the same addresses in the same modes.

One trap it found. **Two fixture types can honestly want the same file name.** One
drawing carries the same Robe head twice — two `FixtureTypeID`s, one product name —
and written under one archive entry they become one type on the way back in, with
half the rig repatching itself. A name already taken now gets a number, in id order,
so two exports of one show write the same names.

```
cargo test -p pult-mvr -- --ignored                  # other people's rigs
cargo test -p pult-backend --test mvr_corpus -- --ignored   # and what they become here
curl -X POST http://localhost:7700/api/import/mvr \
     -H 'content-type: application/vnd.mvr-scene+zip' --data-binary @rig.mvr
curl -o rig.mvr http://localhost:7700/api/export/mvr
```

## The console keeps its own log, and a peer's

**A diagnostic is not the oplog.** The History panel is who changed what — attributed,
undoable, replicated, pruned on its own retention. The **System Log panel** is the
other thing: per station, nobody's to undo, hundreds of lines a second at `debug`, and
the only place a plugin author can read what `logging.log` promised them. `tracing`
still writes to stdout exactly as it did; a capture layer sits beside the `fmt` one and
keeps what it is told.

**It is installed from `main`, never from `start`.** `tracing_subscriber::init` is once
per *process* and a station is a library a process may start more than one of, so
`pult_backend::logging::install` builds the whole subscriber and hands back a
`LogHandle` that both binaries put in `Config` as a `#[serde(skip)]` field. A station
given none simply has no log, which is what every test wants. `logging::detached` is the
same handle with nothing feeding it, for a process that already has a subscriber — and
it takes the levels it is given, because **preferences are read by `install` and not by
`start`**: doing it per station would overwrite what the caller asked for.

**Appends ride the existing `Update` message.** A LOCAL ring in `ShowState` would
rewrite and rebroadcast the whole buffer per line, so lines go straight onto
`UpdateBroadcast` on the `logs` path, coalesced on a 100 ms tick — no new protocol
shape, no `ShowState` entry, and **no hop through the engine actor**, because queueing
diagnostics behind whatever the console is busy with is wrong exactly when somebody is
reading them. The backlog is the `log.tail` RPC. A browser without the panel open
subscribes to nothing and costs nothing.

**Two levels, and a raise that cannot reach past a peer's own.** `log_level` is what a
station keeps; `peer_log_level` (default `warn`) is what it puts on the sync link, so a
peer's warnings always arrive and nobody's `debug` crosses the show's network. A
console watching a peer asks for more with `SyncMessage::LogRaise` — clamped by
`publish_level_for` to what that peer captures, since a station cannot publish what it
never kept, and reaching past it would mean one console changing what another writes to
its own ring and file. **Nothing expires**: the ask is recomputed from who is actually
watching whenever a session comes or goes, and a console that vanishes takes its
connection, and the raise with it.

**A source is a field.** `LogSource` is `Station | Plugin(id) | Browser(session)`, so
the per-plugin filter cannot be defeated by a message containing a bracket;
`host_impls.rs` records `plugin = %id` rather than interpolating a prefix. A browser's
own `window.onerror` reaches the station through `log.report`, deduped and rate-limited,
and crosses to peers like any other line — the tablet at the back of the room is the
console nobody is watching.

**Ordering is honest, not exact.** Each line carries its emitting station's `seq` and
clock: `(node_id, seq)` dedupes the backlog against the live stream and makes a dropped
line *visible* ("1,204 lines did not arrive") rather than a silent hole. Across
stations the merge is by `at_ms`, which is only as good as their skew — see
`station-clock-offset` in the roadmap, which is a live correctness hole in fades and
not only a cosmetic one in logs.

```
cargo test -p pult-backend --lib logging      # the ring, the levels, the file
cargo test -p pult-backend --test logs        # two stations over a real sync link
PULT_LOG_DIR=/somewhere cargo run -p pult-backend   # where this run's file goes
```

## What it costs, and the browser is one of the machines

**Stations is who is here; System is what it costs.** The first panel is the network —
leader, addresses, the link measured from here. The second is processor, memory,
uptime, a line per output connector out of `Station::frame_costs`, and the browsers.
Latency is in both deliberately, being the one figure that answers both questions.

**A browser is not a station and must not appear in `stations`.** That collection is
one row per node, written by the node about itself and replicated; a tab that closes
has to leave nothing behind. `clients` is a LOCAL path instead — a map keyed by the
*short* session id, the same eight characters `LogSource::Browser` carries, so a
warning in the log and a row in the panel are the same tab. `infra/clients.rs` owns it:
the page reports over `client.report`, the socket's own disconnect takes the row away,
and a sweep at ninety seconds takes what is left of a page that stopped talking without
hanging up — ninety rather than sixty because a browser throttles a backgrounded tab's
timers to about one a minute, and pruning at the throttle would flicker the tablet at
the back of the room in and out of the list.

**The figures are LOCAL and the exception replicates.** A fault is occasional and a
frame rate is every second: a row per browser per report crossing the sync link for
ever is a stream nobody reads on the network carrying the show. So the continuous
figures stay with the station serving the page, the way `peers` does, and a window
under 20 fps or with one frame over 100 ms becomes a `warn` through the `log.report`
path task 48 already carries everywhere. `struggling()` in `frontend/src/lib/stats.ts`
is that rule, and the panel *calls* it rather than restating it — a second copy drifted
immediately and had the banner claiming a log line that was never written.

**Measured in the loop that already exists.** `stores/output.ts` evaluates the rig once
per animation frame, so the frame time and the evaluating half are taken there. A
second `requestAnimationFrame` loop would keep a page rendering purely to prove that it
can, which is the wrong thing to do to the tablet being diagnosed. So **a page drawing
nothing measures nothing** and says so — `frames` is `None` and the panel prints
"drawing nothing", for the reason an idle connector carries no `FrameCost` at all. The
figure is the *gap between frames*, not the work inside one: a page served a frame
every 200 ms is stuttering however cheap its own work was.

Two more things worth holding on to. The clock offset is **read** from the estimate
`ws/clock.ts` already maintains, never measured again — a second estimate of one
quantity is a second answer to it. And a page **cannot name its own key**: the station
fills in `session` and stamps `at_ms`, so `client.report` answers the key it landed
under, which is the only way a browser learns its own session id.

Sparklines are the reader's memory. Nothing on the wire carries a series — every report
is one closed window — so `frontend/src/lib/trace.ts` keeps the last sixty readings the
tile witnessed and the panel says so rather than implying a record. A trace dedupes by
the window's *stamp*, because a station that has gone quiet is still being rendered with
its last figure and would otherwise draw a flat line that reads as steady work.

**A station row says what the console costs *and* what the machine costs.**
`cpu_percent` and `mem_used` are this process, deliberately — a console sharing a box
should report its own share. `MachineStats` beside them is the box: global CPU, memory
and swap, load average, the machine's uptime as against the backend's, free space on
**the volume the showfile is on**, and the warmest sensor there is. Never sum the two;
read them as a pair, because a station at 4% on a machine at 96% is about to be starved
by something nobody is watching. `sysinfo`'s `network`, `disk` and `component` features
supply it — no second crate, since it was already here.

**A process CPU percentage is of one core; the machine's is of all of them.** So the
panel labels both ("15.2% of a core" against "6.4% of 18 cores") and states the
comparison outright. The pair only earns its place if it can be compared, and unlabelled
it reads backwards.

One trap, and it is not a corner case: **a relative showfile path matches no mount
point**, so the disk reads a plausible zero. `demo.sh` passes `.demo/demo.db`. The path
is absolutised in `StationReporter::new` by resolving the *directory* and re-joining the
file name — canonicalizing the file fails when the show is about to be created.

**And the probing is a thread, never the runtime.** Every `sysinfo` call blocks, and two
of them block for longer than a console can stand still: the thermal sensors take about
a second on a Mac, and the first enumeration of the volumes takes as long as the
operating system needs to read the directory the executable sits in — it does that once
per process, as a bundle lookup, and against a `target/debug/deps` of six hundred
thousand files it took six seconds. On the runtime thread that was six seconds in which
the station accepted no connection and ran no timer; in a test binary, where every task
of a station shares one thread, it was every test that started a station on a loaded
machine failing with "never accepted a connection". So `StationReporter` owns none of
the handles. A `station-probe` thread does, and hands each reading over a `watch` the
way `links` and `frames` already arrive. The first row waits for the first reading, and
every row after it carries the latest there is — a slow sensor changes what a row says
and never when it is said.

**Network throughput is four figures, not one**, and the panel keeps them apart. Three
are what the console is responsible for — what each connector put on the wire (counted
in `Frame`, *after* the dedup, so a settled rig honestly costs less than a moving one);
what crossed each peer link (`protocol::Counted` wraps the `TcpStream` before it is
split, so the handshake and the heartbeats are in the figure and no call site had to
remember); and what the station sent each browser (counted in the socket's send task,
because a page cannot see its own socket). The fourth is what the machine's interfaces
carried, which includes everything else the box is doing and must never be read as the
console's own. `sysinfo` supplies that with its `network` feature — no second crate,
since it was already here for CPU and memory.

Two traps. **Loopback is excluded** or a demo talking to itself counts every byte
twice. And **`PeerLatency` writes only its own half of a `PeerLink`**: it fires per
heartbeat, more often than the byte window closes, so replacing the row whole wipes the
counters and throughput reads zero almost always.

```
cargo test -p pult-backend --lib clients   # the map, the sweep, who may write a row
cd frontend && npm test                    # the meter, the traces, what counts as struggling
```

## The disk is off the actor, and every source has its own queue

**A group commit, with no constant in it.** `persist`, `oplog::append` and
`order::save` were awaited inside the engine actor against a pool of one connection, so
one operator's edit waited behind another's fsync. `engine/writer.rs` is a single
writer task with an ordered queue, and it commits a *group*: while a commit is in
flight everything that arrives queues up, and when it lands they all go into the next
one. That is the whole rule — no window in milliseconds and no batch size, because a
constant would have to be right for somebody else's disk, and on a fast one with a
single operator the batch degenerates to one write per commit.

A command still replies only when its write is **durable** — the actor hands its
receipts to a task that answers the caller when they land, rather than sitting on the
fsync itself. Nothing about what an acknowledgement means has changed; what changed is
that the *next* command is no longer behind this one's disk, which is the only way the
writer can ever hold a group to commit.

**The oplog is awaited, and it is the one exception.** Entity state is read from
memory — `Get` resolves against `ShowState` and never against SQLite — so a create that
has not reached the disk is still fully visible to the next read. The oplog is not like
that: undo is a *query over it*, the History panel reads it back, and a peer catching up
is served `oplog::since` from the file. Deferring it makes a user's own Ctrl-Z race their
own write, which is exactly what seven tests said the moment it was tried. It costs
little, being one INSERT.

**And one write on the create path used to be quadratic.** `order::save` rewrites a whole
collection, and the engine asked for one after *every* create — about 12.5 million
inserts to patch 5000 fixtures, which is why seeding a rig that size took over two
minutes. `order::append` is the O(1) case a create actually needs, and `save` is kept for
a reorder or a delete, neither of which can be one row. The comment that used to say
"creates are human-paced" was true of an operator and false of an MVR import, which is
the case that matters.

**A create broadcasts its collection, and that is bounded in time.** A subscriber
watching `fixtures` is watching the collection, and a pattern matched against
`fixtures/__create` reaches nobody — so a create has to send the whole thing. Sent per
row, that deep-cloned every fixture in the show once per created fixture, which cost 89
seconds to patch five thousand against three for the entire persistence path. So
`broadcast_after_set` marks the collection and `flush_collections` sends it at most every
`COLLECTION_FLUSH_EVERY`. Note what did *not* work: flushing whenever the command queue is
empty, because a client with sixty-four writes in flight empties it between almost every
one. A ceiling in time holds however the queue behaves, and it is a ceiling on a *burst*
rather than a delay on a write — an idle console has not flushed for far longer, so one
create still goes out at once.

**And an owed broadcast has to be able to wake the loop**, or the ceiling becomes a hole.
The last write of a burst marks the collection, the flush is not yet due, and the actor
then blocks on whatever the show wants next — which on an idle station is a long time and
on a settled one is never. So `next_wake` is shortened to whatever is left of the
interval, and the wake branch flushes. Getting this wrong hangs a delete rather than
slowing it, which is how a test found it.

**A disk that refuses is now reported after the fact, not instead of the write.**
`persist` used to gate the in-memory insert on the disk succeeding. It no longer can, so
a failure reaches the caller as an error while the value is already in the show.
`persist_order` has always behaved that way, on the grounds that losing a list's order is
not a reason to reject the fixture that was just patched.

Two things it needs. A **second pool** to the same file, since the showfile is WAL and a
peer's catch-up read must not queue behind a commit or land inside one; a show in memory
shares the one pool instead, because every `sqlite::memory:` connection is a different
database. And `order::save` stays **outside** the batch — it opens its own transaction
and SQLite has no nested `BEGIN` — which costs nothing, since an order changes when
something is created or moved and never when a value does.

**Admission is in front of the engine, not inside it.** `engine/admission.rs` holds a
bounded queue per source class — Operator, Station, Peer, Plugin — and a router forwards
into the engine's one channel in weighted turns. So a plugin in a write loop fills its
own queue and nothing else, and the engine still reads one channel and still knows
nothing about where a command came from. The weights are **turns, not priorities**:
strict priority starves, and a peer replaying twenty minutes of oplog would never finish
while anybody was programming. A full queue makes its own senders wait rather than
dropping, unlike `OutputHandle::push` — a skipped frame is redrawn a fortieth of a
second later, and a skipped write is gone.

```
cargo test -p pult-backend --lib engine      # the writer, the router, the show
```

## A show is a folder, and Save is a version

A showfile is `Name.pult/` — `bundle.toml`, `show.db`, `assets/<sha256>` and
`versions/<id>.db`. The assets are **files** because a version is a `VACUUM INTO` copy
of `show.db`, and a copy carrying a 256 MB fixture archive would cost that per save; as
files, fifty versions hold one copy of each mesh. `.pultz` is the folder zipped, which
is the form that travels: a folder does not go in an email and on some platforms is not
one thing at all.

**The identity is the machine's, not the show's.** `Config::identity`, then
`PULT_IDENTITY`, then the config directory. It was always meant not to travel with a
show, and a folder is far easier to copy than a file was — two stations sharing an id
would both claim the same outputs and break the vector clock's tie-break.

**No show open is a real state**, and the one a console started with no arguments comes
up in. The engine, the sync layer and the HTTP server all run against a database that
is never written anywhere; the asset store is the one part with nowhere to put anything,
so it is the one part that says no. The browser draws the welcome screen over the same
socket the show would use.

**Opening a show is this station stopping and another one starting in its place.** A
station is built around one showfile from `start` down, so `Console` is the process
around it: it keeps the configuration, pins the port the OS gave out so a `--port 0`
console does not move, records `recent.toml`, and starts the next station. `show.new`,
`show.open`, `show.close`, `show.saveAs`, `show.restore` and `show.list` are **RPCs**,
because which showfile a console has open is nobody's to undo and must not be told to a
peer; each answers `{ok: true}` and *then* the station stops, which a client sees as
the disconnect it already handles. A page compares the show `/api/config` names with
the one it loaded under and reloads when they differ — every store in it is holding the
previous show's rig — and the tablet on another station's socket does the same.

**Save is a point to come back to, not a flush.** Every PERSISTED write is already on
the disk when it is acknowledged. `["versions", "__checkpoint"]` is the verb, beside
`__by` and `__home`: the engine builds the row from its own clock and the caller's
authorship and turns it into an ordinary `__create`, so history, the showfile and every
peer see a create, and Ctrl-Z after an accidental Save deletes the row and takes the
file with it.

**The row replicates and the snapshot does not.** A snapshot is a copy of *this*
station's `show.db`, and a station that joined afterwards never held that state — so
each station copies its own when a `versions` row lands (its operator's, a peer's, or
an undo's) and publishes the LOCAL `versions_here`, which is the only way a panel can
honestly say "not on this station". Restore is refused while a peer is connected, since
that peer would replay its newer operations straight back over it.

Three orderings are load-bearing, and each was a bug first. The copy waits on a
`WriteJob::Barrier` rather than the row's own receipt, so **the snapshot contains the
version it is a snapshot of**. Shutdown waits for the checkpointer *after* the engine,
which holds the only handle. And shutdown **awaits what it aborted** — `JoinHandle::abort`
lands at the next suspension point, so a listener is still bound when its replacement
tries to bind it. `axum::serve` needs more than that: it hands each connection to a task
that is not a child of the one that accepted it, so a station tells its sockets it is
going (`AppState::stopping`) or every open WebSocket goes on talking to an engine that
has stopped, with the page still saying "Connected".

**A restore always leaves an orphan, by construction.** The "Before restoring…" version
is taken after the database being put back was written, so its row is not in it.
`versions::reconcile` reads the row back out of the snapshot's own `versions` table.

Autosave is the leader's, on `autosave_minutes`, only when the oplog has moved, trimming
its own window to `autosave_keep`. `backup_dir` mirrors each snapshot and the assets it
points at somewhere else, and failing to is a warning rather than a failed Save.

**Four demo shows, in Rust.** Haunt, Theatre, Club and Festival, seeded through
`EngineHandle` like anything else — so validation, the oplog and the seeded operator are
what they are for a person; what they skip is the network, not the model. `--demo <id>`
on both binaries and a card on the welcome screen. They are seeded **on a task**, not
inside `start`: the listener is bound first, so awaiting two hundred writes there left
the port accepting and answering nothing. `scripts/demo-seed.mjs` keeps the sized rigs,
deliberately over the public API, because that one is the measurement instrument.

**A demo never writes a rotation.** A fixture's own axis is −Y, so zero rotation *is*
hanging, and `{90, 0, 0}` meaning "hanging" is a quarter turn away from it — which
aimed three of the four rigs at the back wall. `Transform::facing(position, direction)`
does the decomposition properly, and `demo/kit.rs` says which way a light points as a
direction.

**And the console can draw a room it was never given a mesh for.** A `SceneObject` with
no geometry is an empty group, so a truss a console made for itself was invisible.
`pult_schema::types::catalogue` names the pieces — F34 in three lengths and a corner,
decks, wall panels, flats — with their dimensions; `pult-codegen` emits the table to
TypeScript so there is one of it; `frontend/src/lib/stock.ts` draws them procedurally,
one merged geometry per id however many are in the rig. An imported mesh always wins,
and the MVR importer never guesses one: a drawing says what it is with its mesh, and
picking an `f34-2m` because the name said "truss" would put a measurement into
somebody's rig that nobody measured.

**A showfile is still not a migration target.** `SCHEMA_GENERATION` is 3 and refuses a
file from another generation by name. Opening also vacuums when more than a quarter of
the file is free, which is the one moment nothing else is using it.

```
cargo test -p pult-backend --test shows    # opening, saving, restoring, travelling
cargo test -p pult-backend --lib demo      # every demo hangs together, and points down
cargo run -p pult-backend                  # → the welcome screen
cargo run -p pult-backend -- --show Rig.pult --demo festival
curl -o show.pultz http://localhost:7700/api/shows/export
```

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
cargo run -p pult-backend                 # then http://localhost:7700 — the welcome screen
cargo run -p pult-backend -- --show Rig.pult   # or straight into a show
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

## The rig is a drawing, and a place is a transform

`Fixture::position` is an `Option<Transform>` — a position in metres, a rotation as
XYZ Euler degrees, and a **signed** scale — and it is *relative to whatever the
fixture hangs off*. Alongside it are `scene_objects` (trusses, rostra, screens, focus
points, and `Group` for the handle that moves a truss and its lights together),
`layers`, `symbols`, `classes` and `named_assets`, all PERSISTED and all keyed by the
uuid the file they came from used, so a re-import matches rather than duplicates.

**Scale is signed because a drawing mirrors things.** Twenty-one of the forty-three
trusses in the first real MVR this console was pointed at have a basis whose
determinant is −1. No rotation is a reflection, so an unsigned decomposition brings a
mirrored truss back as some rotation that puts it nearly right with its bolt holes on
the wrong side. The reflection is pulled onto X as a negative scale, and anything
drawing one needs a two-sided material.

**Composing a chain is worked out twice** — `crates/pult-schema/src/types/scene.rs`
and `frontend/src/lib/scene.ts` — for the reason `SelectionQuery` is evaluated twice:
dragging a truss re-composes every child per frame and cannot be a round trip. The two
are held together by `testdata/transforms.json`, whose `chains` half both suites read.
Its `matrices` half starts from a matrix as an MVR file writes one and is read by
`pult-backend`, which is where `pult-mvr` and `pult-schema` meet.

Consequence worth holding on to: **a geometric selection term reads a world position**,
so `evaluate` takes the scene objects as well as the fixtures. A light on a truss is
where the truss put it.

**The rig view is plain three.js, and the beam is not geometry.** No Threlte: the
declarative layer was where two defects lived — a `ConeGeometry` with reactive `args`
rebuilt per fixture per frame, and a `SpotLight` mounted inside an `{#if}` that changed
the scene's light count mid-fade and recompiled every material — and removing it
removed both by construction. `Rig3D.svelte` owns its renderer, scene, camera and
`camera-controls` **per panel**, because two `rig` tiles can be open at once.

`frontend/src/lib/beam.ts` is the beam: one instanced open-ended cylinder for the whole
rig, and the cone is vertex displacement, so a zoom costs one float in a buffer. What
makes it read as light rather than as a tube is **the tube's own surface normal against
the view**: the middle faces the camera and is bright, the edge is perpendicular and
goes to nothing, and the power on that term falls with how end-on the beam is seen, so
looking down the barrel lights the whole disc — the flare — with no second term. The
normal is worked out in the vertex shader for the cone it just made and interpolated,
never from screen-space derivatives, which are flat per triangle and draw the strips.
Attenuation is along the throw in metres and steeper for a wider beam. The fragment's
alpha is **one**: additive blending scales the colour by it, and writing the strength
there as well squares every beam into a ghost. The first version of this shader got
every one of those wrong at once — a silhouette term taken from the beam's *axis*,
which is the same for every pixel across the beam — and drew flat, hard-edged,
faceted cones. Colour dims in **HSV, value only** — a scaled RGB drags a saturated
colour towards grey on the way down, which no dimmer does. Haze is turbulence (the
absolute value of signed noise, four octaves) in world space with **time as the third
axis**, floored at the beam's own intensity so it adds folds rather than taking light
away; its two knobs are `Show::haze_density` and `haze_turbulence`: show data, because
how hazy the room is is a fact about the room, seeded from a station preference the
way `home_fade_ms` is. It reaches no lamp.

**The floor cuts the beam, not the cone's own end.** A cone cut square to its axis at
the axis's floor hit is level with the floor only when the beam is vertical; aimed at an
angle, the uphill half of the end ring stands in the air above the spot it is lighting.
`drawnLength` in `stage.ts` runs the cone on until its whole end ring is under the deck,
and the fade over the floor does the cutting. The pool spotlight takes the beam's own
half-angle, so the spot on the floor is exactly as wide as the beam that makes it.

**A strobe is drawn and never evaluated.** A strobe channel carries a *rate*: the
console sends the byte and the fixture does the flashing. So `pult-render` has nothing
to work out and needs no corpus case, and `strobeGate` lives in `beam.ts` because the
square wave is a fact about the picture rather than about the rig.

**And a piece with no mesh is drawn from the catalogue.** `frontend/src/lib/stock.ts`
turns `SceneObject::catalogue` into geometry — see *A show is a folder* — which is what
lets a console that has never imported an MVR hang a rig on something.

**The browser draws the drawing.** `frontend/src/lib/geometry.ts` loads a mesh once
per sha and clones it per object, because a rig with ninety-five truss sections
instances five symbols. Three rules live there: a `.3ds` is Z-up and is turned in that
one place; a `.3ds` asks for its texture by the bare name the archive carried, which
`named_assets` and three.js's own URL modifier resolve to a content-addressed asset;
and a file the loader refuses becomes a placeholder box, because a rig view that goes
blank over one bad mesh is worse than one with a box in it. A mirrored instance gets
its own material — negative scale reverses winding, and back-face culling turns it
inside out.

The **Layers panel** is where a drawing's layers are shown and hidden. Visibility is
per browser and hiding a layer takes its objects out of the plan and the rig **and
nowhere else**: a hidden fixture still takes a cue, still answers a group, and is still
in the patch. And the rig view reports what a frame of *itself* costs, which is not the
station's output frame cost in the `stations` row — nothing about drawing a rig reaches
a lamp, and `demo.sh --measure` deliberately starts no browser.

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

**A show can be a size instead of a scene.** `--size small` is the console's own Haunt
demo, seeded in Rust at open time (`--demo haunt`), and the default; `--demo` takes any
of the four; `big` and `huge` add a generated rig on top — 500 or 2000 fixtures
across as many universes as they need, a cue stack over several sequences each
capturing a slice of the rig, and effects left running so the station has something
moving in it. They exist to be measured rather than looked at, and `--measure` is how:
it seeds, drives every sequence to a cue with an effect on it, seeds an Art-Net output
at loopback so there is a frame to measure at all, and prints what one cost — then
stops, with no sims and no dev server, because both would be taking the CPU being
measured. `--release` with it, or the figures mean nothing next to anybody else's.

`--size` also takes a plain count, because the shape of the curve is the answer and not
one point on it, and `--cues` and `--slice` are separate axes so one thing moves at a
time. Left alone, `--size <n>` holds the *captures per cue* at `huge`'s count rather
than its fraction — a fraction held constant grows the cue stack with the rig, and then
two runs differ in two ways at once.

**The instrument says how much it disagrees with itself.** It takes several reporting
windows, discards the first, and prints the median with the full spread beside it. It
also waits for the cue-taking it does to get the show moving to go **quiet** before it
starts: those writes are three hundred per-fixture broadcasts on a 505-fixture rig, and
counting them made the frame spread 92% and made a running show look as though it were
pushing values. Quiet rather than a slept constant, which is only ever right for the rig
it was measured on.

**A browser is measured separately, and the figures must not be read side by side.**
`--measure-browser` opens a headless page (Playwright, an optional devDependency) and
reads what it reports through `client.report`. Its own mode because a page drawing the
rig competes for exactly the CPU `--measure` is holding still.

```
scripts/demo.sh --size huge                        # 2000 fixtures, 300 cues, three plans
scripts/demo.sh --size 5000 --cues 60 --slice 0.02 # one axis at a time
scripts/demo.sh --measure --release --size 5000    # seed it, read it, print it, stop
scripts/demo.sh --measure-browser --release --size 5000
```

**A station knows what its own output frames cost** and publishes them in the
`stations` row beside `cpu_percent`, so the figures `--measure` prints are the ones the
Stations panel shows and the ones a peer sees. **One entry per connector**, because
their rates and their costs are their own: Art-Net drawing at 40 Hz beside an OpenHaunt
node that was told about a fade once are not two samples of one number.

**Three figures per connector rather than one**, because a frame has parts that scale
differently: evaluating, assembling universes, and the socket write. Evaluating is
linear-ish in the rig; the other two are per universe, and a rig of 5000 six-channel
heads is about 59 universes against 24. Splitting them is what answered the question
this was built for, and the answer was the opposite of the prediction: at 5000 fixtures
**evaluating is 94% of the frame and assembly plus socket is 6%**, where the worry had
been that the per-universe half was the one that would not shrink.

What it is *not*: what the process costs. That is `cpu_percent`, in the same row, which
is why anything printing one prints the other. And a connector that emitted nothing in a
window reports **nothing rather than zero**, since zero would read as "instant" when the
truth is that nothing happened.

## What is on the wire, and a connector says what its own traffic looks like

**Stations is who is here, System is what it costs, and *On the wire* is what left.**
The `wire` panel shows the bytes themselves: the sheet a DMX universe went out as, the
messages a node was sent. Which is the third of the three panels task 48 opened, and it
inherits both answers rather than re-deciding them.

**A view is asked for, never published.** A universe image is 512 bytes forty times a
second; a station that broadcast that to its browsers — or, worse, across the link
carrying the show — would be paying continuously for a picture nobody is reading. So
opening the panel is `output.watch`, closing it is `output.unwatch`,
`infra/connectors/viewers.rs` holds who is looking, and a connector nobody is watching
is **never asked**: the manager's view arm sleeps for an hour rather than waking ten
times a second to find out that nobody is there. **Nothing expires** — the ask is
recomputed from who is actually here, so a tab that vanishes stops the drawing as surely
as one that closes politely. And a drawn view that has not changed is not sent again,
which is what "diff at panel rate" comes to: a settled rig with the panel open costs
nothing.

**A peer's output is asked for down the link**, because only the station holding a
socket can say what went through it. `SyncMessage::OutputWatch` carries the whole ask
and empty is the withdrawal; `OutputTraffic` carries the answer back. Exactly the shape
`LogRaise`/`LogLines` already had, at protocol version 6 — and a peer's ask lands in the
same `Viewers` table a browser's does, with the peer standing in for a session, so a
connector cannot tell a booth across the room from a tab on this machine.

**A connector describes its own traffic, in shapes rather than in protocols.**
`OutputPlugin::observe(focus)` answers `Vec<OutputSection>`, each carrying a
`SectionBody` — `Universes` or `Messages` today — and
`frontend/src/lib/components/wire/views.ts` is the one place a shape becomes a
component. That is the whole of what makes a new output cheap. One whose traffic
carries universes gets the DMX sheet for **nothing**. One that looks like neither adds
a variant in `pult-schema`, a component beside the others, and one line in that table —
no panel changes and nothing enumerates outputs anywhere. A shape this build has never
heard of draws as itself rather than vanishing, the rule the layout tree already follows
for a panel id it does not know. And the default answer is `None`, so a connector that
does not describe itself says so and the panel prints that rather than an empty sheet.

**`focus` is opaque all the way through**, and named in the connector's own terms — a
universe number, a node's serial. A field per protocol is exactly what a seam meant to
carry a protocol nobody has written yet cannot have. The one place a universe is spelled
as a focus string is `universeFocus` in `frontend/src/lib/wire.ts`, beside the sheet
that asks for one.

**The DMX family pays nothing for being watched.** `UniverseCache::observe` reads the
images the dedup was already keeping, so Art-Net, sACN and the sACN a gateway is fed all
answer the same way and a sheet reads the same whichever carried the universe. It
reports **when a universe last changed as well as when it was last sent**, because a
keep-alive is not movement: a sheet that read the send as movement would report every
idle universe as busy. What is not free is a ring of discrete messages, which is why
`OutputPlugin::watched` tells a connector whether anybody is reading at all — OpenHaunt
keeps its port commands only while somebody is, and throws away what it held when the
last viewer goes.

**Two rings, and neither loses a message silently.** The connector's is bounded by what
it can afford between two looks and hands over what it has *drained*; `wire.ts`'s is
bounded by what a person can read, and is what turns a sequence of batches back into a
log. Both count what they dropped and the panel prints the total, for the reason the
system log makes a gap in `seq` visible rather than leaving a hole nobody can see.

```
cargo test -p pult-backend --lib connectors   # the registry, the cache, and what is drawn for whom
cargo test -p pult-backend --test wire        # two stations, and a console watching the other's wire
cd frontend && npm test                       # what a browser makes of the batches
```

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

`pult-gdtf` has an `#[ignore]`d half that reads other people's files.
`scripts/fetch-interop-corpus.sh` downloads them into gitignored `testdata/corpus/`;
what is checked in beside it is `testdata/gdtf/`, three small fixtures written here.
CI runs both halves.

`pult-render-wasm` *is* a default member, despite being the browser's half: its tests
are the corpus that holds the two compilations of the evaluator to each other, and a
guard outside the default suite is a guard nobody runs. Its vitest half needs
`scripts/build-evaluator.sh` to have been run, and says so loudly rather than passing
quietly when it has not.

Both the Rust build and `svelte-check` are kept at zero warnings, so a new one is
visible rather than buried.

**Counts are gates; milliseconds are not.** `cargo test -p pult-backend --test counts`
asserts three machine-independent figures: a running show pushes **zero** fixture
updates at a browser, a drag of sixty frames is **one** row in the history, and a
settled rig reports **no** universe as changed. A timing threshold on a shared runner
flaps, a flapping gate gets disabled, and a disabled gate is worse than none — so what
`--measure` prints is read by a person before a release and asserted on by nobody.
