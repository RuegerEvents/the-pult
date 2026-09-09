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
- **`crates/pult-audio`** — Sound, as numbers: waveform peaks and their codec, an LTC
  encoder and decoder, the Beat This! detector through `rten`, and the chase
  discipline. A pure crate in the shape of `pult-gdtf` — `serde`, `thiserror`, `rten`
  and *no pult crate, no OS, no socket, no clock* — so every one of them is testable
  with no sound card in the machine. The station's half, which is all the OS there is,
  is `crates/pult-backend/src/infra/audio/`.
- **`crates/pult-schema`** — Data model + path accessor infrastructure. All entity types live here. Source of truth for the WebSocket protocol and sync protocol.
- **`crates/pult-gdtf`** — GDTF, read and written. A pure format library: `quick-xml`, `serde`, `zip`, `uuid`, `thiserror`, and *no pult crate* — which is what lets it be tested against other people's files with no station near it. Writing is why it exists rather than a crate off crates.io. The translation into the schema is `crates/pult-backend/src/infra/interop/gdtf/`.
- **`crates/pult-mvr`** — MVR, read and written. The other half of the interop pair and
  a pure format library like `pult-gdtf`, depending on it for the fixture definitions
  inside an archive and for the millimetre Z-up to metre Y-up conversion they share.
  `transform.rs` is where a matrix becomes a position, a rotation and a **signed**
  scale — signed because a fifth of the trusses in a real Vectorworks file are
  mirrored, and no rotation is a reflection.
- **`crates/pult-mvr-xchange`** — the MVR-xchange protocol, on paper: the messages and
  both of the specification's framings. `serde`, `uuid`, `thiserror` and nothing else —
  no socket, no runtime, and *not `pult-mvr` either*, because not one field of a message
  carries MVR content and an archive crosses as an opaque buffer. The station's half is
  `crates/pult-backend/src/infra/interop/xchange/`.
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
`programmer_values` entry over them, a running timeline's recorded track between the
two, the home value beneath — and every consumer works out a number for the moment and
at the rate it needs one. That is the whole of the model, and it is why the engine has
no tick.

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

**And "the show" is a version per collection, not one over everything.** `push_output`
hands the connectors `OUTPUT_COLLECTIONS` — fixtures, fixture types, the programmer —
and asks whether *those* have moved; `playback_pass` names its own list the same way.
One counter over the whole show was what this used to be, and the difference is not
academic: a station writes its own `stations` row every two seconds and its output
status every second, so an idle console rebuilt and re-pushed an identical patch one to
two times a second. At 5000 fixtures that costs **116 ms** inside the output loop, and
it is where two thirds of the Art-Net connector's missing 10 Hz went. The list lives
beside the read it belongs to, and the fallback is *everything*: a write nobody can
attribute to a collection counts as all of them. Narrowing this until it says nothing
is a rig that silently stops updating, which is why the gate is a pair.

**A frame's deadline is measured from the last deadline, never from the wake.**
Nothing wakes on time — 2.4 ms of timer granularity and scheduler latency is ordinary —
and `Running::schedule` measuring from `Instant::now()` baked that into the period, so
every frame was late by the sum of every lateness before it and a 25 ms period was
really 27.4. Chained from the deadline, the same 2.4 ms is jitter about a fixed rate.
The chain is *clamped* to `[now, now + period]` rather than merely floored, because a
settled DMX line waits 800 ms between keep-alives and chaining off one would make the
first frame of a cue arrive after the light had got where it was going.

Worth holding on to: **the frame cost could not have found either of those.**
`began.elapsed()` wraps `plugin.send`, which is the right thing for it to measure —
both defects lived in the gaps *between* frames, and nothing measuring the work inside
a frame can see a frame that was never asked for.

**The browser has to know the station's clock.** The objects are anchored in console
time, so a page evaluating against an unadjusted `Date.now()` runs every fade out by
however wrong its own clock is, silently. `frontend/src/lib/ws/clock.ts` estimates the
offset the way a round-trip time is estimated, maintains it rather than taking it once,
and — this is the rule that matters — **says nothing until it has one**: `consoleNow()`
answers `null` and panels show a gap rather than a plausible wrong number.

**And so does a station, about the leader's.** Same failure between two consoles, and it
reaches lamps rather than pixels: `live_fades`, `live_effects` and a cue's `went_at` are
absolute milliseconds, so a station evaluating a peer's fade against its own clock runs
it out by their skew. `pult_schema::clock` is the answer — **the leader's clock is the
show clock**, a follower estimates the offset over `ClockPing`/`ClockPong` on the sync
link, and `now_ms()` applies it, which corrects playback, the connectors, a Go's `at`, a
log line and the answer a browser syncs against all at once. The estimator is
`ws/clock.ts`'s arithmetic, held to it by `testdata/clock-offset.json`.

Four rules there, each of which is a way the rig would otherwise jump. A correction under
20 ms applies, over 1 s steps, and in between is **walked at 5% of real time** — because
`now_ms` is monotone-from-a-base precisely so a stepping system clock cannot jump every
running fade, and correcting it puts that back unless it is disciplined. It is **never
stepped backwards** at any size: that falls before a landed fade's `t1` and starts a
parameter that had arrived moving again. A **promoted leader keeps the offset it had**,
as a standing bias, so the show clock is continuous across a failover — after one, show
time is no machine's wall clock but a timeline the session carries. And a station with no
estimate yet **applies zero and says so**, in a `warn` and in its row: `consoleNow()`'s
rule is right for a page, which can show a gap, and wrong for a lamp.

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
stations the merge is by `at_ms`, which is only as good as their skew — and since task
64 that skew is corrected, so the interleave got better without this code changing.

```
cargo test -p pult-backend --lib logging      # the ring, the levels, the file
cargo test -p pult-backend --test logs        # two stations over a real sync link
PULT_LOG_DIR=/somewhere cargo run -p pult-backend   # where this run's file goes
```

## What it costs, and the browser is one of the machines

**Stations is who is here; System is what it costs.** The first panel is the network —
leader, addresses, the link measured from here, and what each station is doing about the
show clock. The second is processor, memory, uptime, a line per output connector out of
`Station::frame_costs`, and the browsers. Latency is in both deliberately, being the one
figure that answers both questions.

**The clock column is three states, because two of them are zero.** `ClockSync` on the
station row is `Reference | Corrected | Uncorrected`: a station that *is* the clock and a
station that could not measure one are both adding nothing, and one figure meaning both
would be exactly the plausible-wrong-number the mechanism exists to remove. Beside it and
LOCAL, `PeerLink::offset_ms` is how far that peer's *show* clock is from this one's —
about zero on a converged link however far either is correcting itself, which is what
makes it answer "do these two consoles agree" rather than "how odd is that machine".

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

**And the page shows one screen for it, not three.** The menu and the welcome screen
call `beginSwitch("opening Festival")` *before* they ask, so the cover is up before
the socket goes; the switch lives in `sessionStorage` so the reloaded page comes up
already saying it; and it ends when `watchStation` hears a fresh `/api/config` and the
tab is staying put. The tablet is *told*: the stop signal carries why
(`ShowSwitch::describe`) and the socket's send task — the one that holds the sink —
writes it into a close frame with code **4001**, which `frontend/src/lib/switching.ts`
turns into the same screen. Any other close code is a lost console and draws as one.

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

**A boom is a run of truss stood on its end, and a lantern on it is written in the
boom's frame.** A fixture's position is relative to what it hangs off, rotation
included, so `kit::boom` turns the run's handle a quarter about Z and `kit::on(parent,
world_offset, world_direction)` puts a world-terms offset and aim into that turned
frame — once, so no show file hand-inverts a rotation. The first Theatre drew its
booms as two more horizontal bars. And everything hangs `HUNG_BELOW` (350 mm) under
its bar's centre line: the chord square plus a clamp, and the one figure every demo
uses, because the first Club hung its washes 600 mm *beside* the truss and what that
drew was lights floating next to it. The demo tests check both, composed through
`world_transform`, which is the only direction that says where a light on a boom
looks.

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

In dev, Vite proxies `/ws`, `/assets`, `/stock` and `/api` through to `PULT_BACKEND`
(default `http://localhost:7700`), so dev is same-origin too. **A prefix the station
serves has to be listed there**, and the failure is quiet rather than loud: an unproxied
prefix returns the SPA's own HTML rather than a 404, so a loader gets a page where its
file should have been and falls back — which, when `/stock` was missing, drew every truss
in the rig as `geometry.ts`'s placeholder cube in dev and nowhere else.

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

**And the SDK has a typed half over the same generic wire.** `pult-codegen`
writes `plugins/sdk/src/generated/` from the same inventories the frontend proxy
comes from, so `data::cues().nth(3).fade_in_ms().set(4000)` is
`host::set(&["cues", "3", "fade_in_ms"], …)` with the compiler on it — the split
`frontend/src/lib/ws/data.ts` has had all along. **The WIT stays untyped and that
is the point**: a component's imports carry the package version and a record's
fields are part of every signature using it, so a `Cue` gaining a field would be
a breaking ABI change, and a show now carries its plugins between machines. As a
*source* convenience it costs nothing: a bundle built against schema-of-Tuesday
still loads on schema-of-Wednesday, and the one call naming a path that station
has not got fails there, with the path and the wanted type in the message.

`schema.rs` is the entity types, mirrored out of `pult-schema`'s own source with
`syn` — the plugins workspace cannot depend on that crate, whose sqlx, tokio and
inventory are the wall `pult-render` was split out over. `pult-render` itself is
a *path dependency* rather than a mirror, because it was built to be compiled
twice and its own doc says "and its plugins": a `ParameterValue` in a guest is
the console's type. What the mirror cannot carry is code, so a type whose
`Default` is written by hand — `Transform` rests at scale 1, `FixtureAddress` at
universe 1 — loses the derive, and so does anything holding one directly. Losing
it is a compile error at the plugin author's desk; keeping it would have been a
fixture patched at scale zero.

Introspection is not replaced and must not be. Typed accessors are what was known
at *build* time; `host::entities()` is what *this* station has now, including the
collections an SDK never heard of — which is what `command-line` builds its whole
grammar out of.

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
cd plugins && cargo test                   # the SDK's paths, and the CLI grammar
cargo test -p pult-codegen                 # the checked-in SDK is what codegen writes
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

The **`xchange` panel** is MVR-xchange: the group, who is in it, what they have
committed, and the two acts. See *A rig can leave while it is still being drawn*.

The **`outputs` panel is the I/O panel** — outputs and inputs, which are the same row
read in two directions — and the **`timeline` panel** is a position with a waveform, a
beat grid, events, markers and takes written against it. See *A recording is a function
of time* and *A song the console can hear*.

The **`sheet` panel** is the rig as a table and the **`cues` panel** the editor of one
sequence; **`pools`** is the direct selects — saved groups and presets, not sequences.
See *The loop, and a colour that says where a value came from*.

The frontend opens onto a **tiled workspace** rather than a sidebar and tabs. Panels
live in a tree of splits and tab groups: drag a tab to a tile's edge to divide it or
to its middle to stack it, drag the gutters to resize, and pick a layout from the menu
in the top bar. Presets are built in; *Save as…* writes an arrangement into the show
as a `layouts` row. Which layout this browser is looking at is kept in `localStorage`,
not in the show.

**Setup is a mode, not nine panels.** `PanelMeta.home` in `layout/panels.ts` is
`workspace | setup | both`: patching, fixture types, devices, I/O, network, session,
plugins, MVR-xchange, the show and the settings are *errands*, and an errand that costs
a tile costs the picture somebody is programming against. `components/setup/Setup.svelte`
is a full-screen dialog whose every section is the panel component **unchanged**, which
is the whole trick and why it was cheap. What stayed a panel stayed for a reason written
beside it in `panels.ts` — a sparkline is only a record because the panel witnessed it,
a log subscribes while it is mounted, a wire view *is* an `output.watch`. And the three
modals that had each grown their own opinion about Escape and the backdrop are one
`Dialog.svelte`.

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

**A cue is the stack up to it.** Taking a cue applies the latest capture of every key
over the cues up to and including it, worked out from the show and not remembered:
`Playback::take_cue`. Forward one step that changes nothing — what an earlier cue set
is still what is driving the parameter, and is left alone. Jumping back releases every
key only later cues capture, over the cue's down time; jumping forward applies what
the cues in between set. Tracking playback, deliberately, and the reason a blackout cue
still has to zero everything by name.

**A cue fades two ways.** `fade_in_ms` is what a parameter takes going up and
`fade_out_ms` what it takes coming down, on the cue and per capture, the capture
winning. Zero out means "this cue does not split its fade" rather than "snap", so a
show that never sets one runs exactly as it did. Only values with an order to be on
can be going down — a colour has three and a relay none, and those take the in time
rather than have the console guess a ranking.

**And it fades in a shape, asked for in the same three steps.** The capture's `easing`,
then the cue's, then the show's `fade_curves` — `FadeCurves::resolve` in `pult-schema`,
one implementation, because playback and the cue editor deciding it separately would
disagree only on the cues nobody tested. Both are `Option<Easing>` so that "said
nothing" is a different value from "said linear": `Easing::default()` is `Linear`, and
without the option a default above could never reach a capture. A show's own answer is
per **group** — `FadeGroup` is intensity, position, colour, beam and everything else,
keyed off the parameter *key* so that a release, which holds only a key, asks the same
question a capture does. Position rests at ease-in-out and the rest at linear, which is
the whole point: a dimmer has run linear since dimmers had handles, and a head that
runs linear into a mark and stops dead reads as a fault. **A release takes the show's
curve too** — letting go of a mark is a move, and nothing above it can say otherwise.
Seeded from a station preference the way `home_fade_ms` is.

## The loop, and a colour that says where a value came from

Select, set, store, play, update — the loop every other desk has. Four panels and three
verbs, and the interesting part is how little of it is stored.

**The `sheet` panel colours every cell by which layer is driving it**, and this console
does not have to keep a flag to do that. The model already keeps *what is driving* each
parameter, so `frontend/src/lib/sheet.ts`'s `source(drivenBy, track, shownCue)` reads
the answer off `driving.ts`'s four layers and the panel *calls* that rule rather than
restating it — the discipline `stats.ts`'s `struggling()` already follows. Programmer
amber, recording green, effect magenta, this cue white, tracked cyan, home grey;
MA-near deliberately, because an operator who has stood behind a grandMA reads amber as
the programmer without being told. The one layer a page cannot read off a fixture row
is the recording — a take is bytes that never leave the wasm — so `Evaluator::recording`
answers which watched keys one is asserting, as a read over `TrackAt::value_at`, and
only while a timeline is running.

**Looking at a cue is not taking it.** A click on a `cues` row sets `cueInView` and the
sheet draws the stack *up to* that cue, hard against tracked, reaching no output at all;
a double-click or the Go column takes. Which means "a cue is the stack up to it" is now
evaluated twice: `trackedThrough` in `cues.ts` mirrors `cue::tracked_through`, because a
cue clicked in a list has to colour the sheet in the same frame, and `testdata/tracking.json`
holds the two together the way `selection-queries.json` holds `evaluate`.

**Update needs no target.** A parameter being driven by a cue says which cue —
`live_fades[key].cue_id`, `live_effects[key].source` — so `updateDriven` writes each held
value into the cue driving it *now*, one gesture over however many cues it touches. Keys
nothing is driving are handed back rather than guessed at, and the Store dialog opens
with exactly those ticked. **Cue only** is the other half: `cueOnlyCompensation` writes
what the *next* cue was tracking into it, for every key the store changes that the next
cue does not capture itself, so a change stops at this cue's edge. The compensating
capture carries the value and none of the timing — it is going into a different cue and
should move the way that cue moves. Both are one gesture, so each is one Ctrl-Z.

**Space is Go on the cue sheet touched last**, and with none focused there is no Go and
a toast. A console with three of them open has to answer which, and answering it by
picking one is a look on stage nobody asked for.

## A preset is a reference, and the literal sits beside it

`presets` is a PERSISTED collection of `Preset { id, name, values }`, one flat pool
whose values may be **any mix** — a preset *is* a look, and a look is usually a position
and a colour together. The I/P/C/B/O tags an operator filters by are **derived from the
keys and never stored**, so a preset that grows a colour is a colour preset from that
moment and nobody has to reclassify it.

**Reference first, literal beside it.** `ParameterCapture.preset` and
`ProgrammerValue.preset` are `Option<Uuid>`; `value` stays the copy taken when the
capture was stored and is **never** rewritten by a preset edit — it is a record of what
the cue was stored as, not a cache of what the preset now says. So deleting a preset
**cascades nothing**: every cue that used it goes on running exactly as it did, the UI
says "preset missing", and Ctrl-Z of the delete restores every link because no link was
broken. `ParameterCapture::value_in` is the one resolution, called by `start_capture`,
by the playback pass and by `paperwork.cueValues`.

**And editing one reaches the cues that are standing**, which is the whole reason a
palette is worth having. `"presets"` is in `PLAYBACK_COLLECTIONS`, and
`Playback::repoint_presets` restarts every standing fade whose capture names a preset
that now resolves elsewhere — from `value_at(now)` over `home_fade_ms`, the pattern
`release_key` follows and for the same reason: swapping a running fade's `to` in place
would jump, because `from` is where it started. Gated on its own version counter, so an
ordinary Go never walks the fades looking for one.

**Moving a value breaks the link, in one write.** A fader, a typed number, an `at +10`
each write the whole programmer row with `preset: null` rather than a value and a
clearing write — two writes leave a moment in which the show says the parameter is still
that palette's while holding a number the palette does not say.

Two smaller things worth holding on to. **`values` is a SQL keyword**, which
`Preset::values` found: identifiers are quoted in the macro's `column_defs`, in `db.rs`
and in both migration passes, which unquote before comparing against `PRAGMA table_info`.
And **a guest can now spell a parameter key**: `pult-codegen` mirrors `parameter_key`
into the SDK by an explicit list — the mirror carries shapes and not code, and that is
the stated exception — because a fourth hand-written spelling would be a plugin reading
a key nothing writes.

```
cd frontend && npm test                              # the sheet's colours, tracking, cue only, presets
cargo test -p pult-schema --test tracking_corpus     # the other half of that corpus
cargo test -p pult-backend --lib playback::tests::presets   # a standing cue follows a palette
cargo test -p pult-backend --test counts             # a store across three cues is one undo
```

## A piece says where it connects, and a light says what it is clamped to

Until a person could put a truss in a room, a stock piece was a shape the *browser*
drew: `stock.ts` built cylinders from `catalogue.rs`'s dimensions, and that was the
whole of what an `f34-2m` looked like. Which meant an exported MVR carried an **empty
group** where the truss was — MVR has no primitive, its `GeometryNode` is a file or a
symbol instance — so a rig built here and opened in Vectorworks was a room full of
nothing. A from-scratch rig is *all* stock pieces, so that was the whole rig.

**So the geometry is one implementation, on the station.** `pult_schema::stock::stock_glb(id,
&properties)` is a pure function over the same table; `GET /stock/{id}.glb` serves it with a
strong ETag over the bytes; `frontend/src/lib/stock.ts` loads it through `geometry.ts`'s
cache like any other mesh, so a hundred truss sections are one download. The bytes drawn
are the bytes exported. It is deliberately **not** an asset: the store refuses a write
with no show open and the welcome screen still draws a rig, a generated mesh that
outlived the code that made it would be a stale asset nobody could explain, and there is
nothing here a peer holds that this station cannot make in a hundred microseconds.
Determinism is a gate rather than a nicety — the ETag, the archive entry's name and the
symdef's uuid all rest on it — so no map is iterated, every position is rounded to a
micrometre, and the tests generate every piece twice.

**A `.glb` goes out as a symdef, and comes back as the piece.** `stock_symdef_name` carries
the piece id and its canonical properties, and `stock_symdef_uuid` is a v5 of that name;
`parse_stock_symdef` checks the uuid against the name **before trusting it**, which is
what stops a drawing that happens to name a symbol `pult-stock:f34-2m:{}` from having its
own mesh thrown away. This console restores `catalogue` and stores no mesh; anybody else
opens an ordinary symbol with a truss in it.

**A connector is a point, an outward facing and a kind, and mating is the whole snapping
rule.** Two joints mate when their points meet and their facings end up opposite — which
is what a bolt does, and the reason the *rotation* falls out of the snap rather than
being one more thing to get right. Like mates like: a deck edge never catches a truss end
however close it is dragged. A corner is a **six-way box** — six `TrussEnd`s, one per
face — which makes a spigot kind unnecessary, since a base plate is one truss end
pointing up and a top plate one pointing down. **Free is worked out from the geometry,
not from a field**: two pieces are joined when their joints are already mated, so a run
of four sections offers a `+` at each end and nowhere in the middle, and goes on being
right when somebody deletes a section out of the middle.

**A light on a bar is clamped, and a clamp has two degrees.** `Fixture::mount` is
`{chord, along, roll}` — which chord of the piece, how far along it, how far round it —
and `crates/pult-schema/src/types/mount.rs` resolves one. Told only a `position`, a gizmo
would have to offer three axes and a free rotation, and every one of the six would take
the light off the truss.

**The mount does not replace the position: the browser writes both, together.** Resolving
a mount on a truss that came out of a drawing means knowing where its chords are, and
those come off the mesh's own bounds — which only `geometry.ts` ever measures, because
the station never loads a mesh. So the browser is the writer for *every* parent,
catalogue piece or drawing alike, and `testdata/mounts.json` is what keeps its arithmetic
equal to `Mount::transform`'s. `HUNG_BELOW` is 205 mm measured from the **chord**, which
on an F34 is the same 350 mm below the bar's centre line every demo used before there
were chords to hang off. A catalogue piece declares its four; anything else gets **one**,
along the bottom of its bounds, which is the smallest guess available on purpose.

**A re-import reads the clamp back off the geometry.** MVR has nowhere to say a light is
clamped, only where it is — so where the parent is a catalogue piece and the light is
sitting within a millimetre of where one of that piece's clamps would put it, that is
what it is on. A millimetre because the number came out of this console's own arithmetic
on the way out; anything looser would be inventing a clamp for a light somebody placed by
hand.

**Composing a chain now runs both ways.** `local_of(world, parent_world)` is the inverse
of `world_transform`, in `scene.rs` and `scene.ts`, held together by the `inverses` half
of `testdata/transforms.json` — and it is not `−position`: the inverse of "moved and then
turned" is "turned back and then moved in the turned frame". Every drag in the editor
crosses that seam, because a gizmo hands back a world placement and a
`SceneObject::transform` is relative to its parent.

**And the editor writes nothing the engine had to learn.** Place, duplicate,
delete-with-children, the default `Stage` layer and every mount write are sequences of
ordinary path writes inside one gesture (`stores/editor.ts` over `stores/gesture.ts`), so
each is one Ctrl-Z. Two selections, deliberately separate: a `SelectionQuery` is a
question about the *rig* and `at 50` means the fixtures it answers, so `selectedObjects`
is its own store and a truss is never in that scope.

`SCHEMA_GENERATION` is 5.

```
cargo test -p pult-schema                          # the corpora, the glb, the joints
cargo test -p pult-backend --test mvr_corpus       # a from-scratch rig, out and back
cargo test -p pult-backend --test counts           # a dragged truss is one history row
curl -o truss.glb 'http://localhost:7700/stock/f34-3m.glb'
```

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
way `home_fade_ms` is. It reaches no lamp. **Density is how much of a beam shows at
all** — the haze is the only reason light can be seen in the air — so 0 is a clear
room with no beam drawn and the light still on the floor, 1 is every beam with its
folds, and 1 is the default.

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

**The view draws when there is something new to see, at most sixty times a second.**
The camera moved, the rig changed — worked out by comparing this frame's attributes
against the last frame's, through `Math.fround`, because the output store ticks every
frame whether or not a value moved — or the picture animates on its own clock: a lit
beam with haze in it, a strobe. Only lit beams get an instance, packed from the front,
since an unlit cone whose fragments all discard is still a cone that is rasterised. A
settled dark rig costs nothing and the panel prints *idle*; beside the page's own work
per drawn frame it prints what the GPU took, read through
`EXT_disjoint_timer_query_webgl2`, which is the figure a laggy view is felt by — a rAF
gap stays at nine milliseconds however far behind the GPU is. What the GPU spends on
the beams is **per blended layer stacked on a tile**, not per pixel, triangle or
instruction: measured, and recorded as `beam-overdraw` in the roadmap. A **View** sheet
on the rig panel holds this screen's work light, resolution and **render mode**, in
`localStorage` beside the layout (`stores/view.ts`) — the haze is the show's, a work
light is not. The four modes are four questions: *Wireframe* (where is everything:
wire materials and an aim line per fixture), *Cones* (where is it pointing: the same
cone under a flat alpha-blended shader, never white), *Real* (the beam shader) and
*Photoreal* (the scene into a half-float target, bloom above white, ACES over the
sum — the only thing that keeps crossing beams a colour). `dress` in `Rig3D.svelte`
switches by swapping materials and visibility on what the scene already has; nothing
is rebuilt. Two traps there: the scene object is `$state.raw`, because a deep proxy
keeps a plain field written through it and the render loop holds the object itself;
and the photoreal frame is linear light, so the clear colour, the grid's grey and the
beams' gain are said in linear terms on that path only. Measure any of this in a
*headed* browser only: a hidden tab has no animation frames.

**And the rig view is an editor, but the editing is in panels beside it.** **Pieces** is
the catalogue to drag from and the work plane a drop lands on; **Rig tools** is import
and export, the gizmo's three modes, duplicate, delete and line-up; **Objects** is the
drawing as a tree by parent — the only way to reach a `Group`, which has no geometry and
so cannot be clicked; **Object** is what one piece is, in numbers. The *Scene* layout
preset puts them round the rig. They are panels rather than sheets on the rig's own
toolbar because a strip that opens above a canvas pushes the picture down, and the
picture is what somebody is aiming a pointer at. What stayed on that toolbar is only what
is about looking — and even the View settings hang *over* the canvas rather than above
it. Two consequences: the gizmo's mode is a store, since the buttons and the viewer are
now different panels; and the delete prompt is mounted at the root of the app, because it
can be reached from three places and a modal owned by one of them is missing from the
other two.

**And the rig view is an editor.** A 2D view is not a second panel: it is this one with
an **orthographic camera**, so there is one editor, one gizmo and one hit test — and
ortho with the three-quarter preset is an axonometric nobody had to build. Plan and
section default to it and the toggle is beside the presets. The gizmo is three's
`TransformControls` attached to an operator-placed **pivot** rather than to an object,
because four selected trusses have no single transform to drive and rotating them about
the corner they meet at is almost always the move; each frame applies the pivot's delta,
measured against where the drag *started*, since a per-frame delta accumulates rounding
across a two-second drag. Its writes are coalesced to one per animation frame —
`objectChange` fires per pointer event — and the free-joint list is a `$derived` rather
than per-frame work, since finding one composes every visible piece's placement. A locked
piece keeps its gizmo away and stays **pickable**: a piece you cannot select is a piece
whose name and layer you cannot read.

**Five places to look from, none of them stored.** Front, plan, section, three-quarter
and focus-on-selection, worked out in `frontend/src/lib/camera.ts` from a box over the
placed fixtures and the scene objects and a distance fitted to *both* lens angles — the
horizontal one is `atan(tan(fov/2) · aspect)`, so the same button frames a five-fixture
demo and a festival, on a tablet and on a monitor. Three decisions live there: plan
stands a couple of degrees off vertical, because a camera looking straight down has
nothing to resolve its own roll; section looks from stage left, so the stage is on the
left of the frame the way a section is drawn; and the box reaches the deck for the rig
and **not** for a selection, since one head at six metres framed with the floor in shot
is six metres of air. Focus changes only the distance, never the angle. The front
preset *is* where the view opens, so those are one place rather than two.

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

**Read the Hz column against the frame cost, because they answer different questions.**
A connector at 4.5 ms of a 25 ms budget is not short of frame; a connector at 29 Hz when
`Frames::DMX` asks for 40 is sampling every fade eleven times a second fewer than it
should, and no amount of headroom inside the frame says anything about that. Both defects
that figure eventually named were outside the frame entirely — see *Connectors own their
rate*.

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

**An output carries the universes it says it carries, and it filters where the
evaluating is.** `OutputConfig::universes` is a routing rather than a label: empty is
every universe in the patch, and a list means a two-interface split — this Art-Net node
carries 1–4, that one 5–8 — that every connector obeys, the OpenHaunt one included,
since the sACN it feeds its gateways is a universe on a wire like any other. The filter
lives in `render_carried`, *before* the rig is evaluated rather than at the socket,
because evaluating is 94% of an output frame at five thousand fixtures: gated at the
send, both halves of a split cost what the undivided rig cost. Filtering before
`UniverseCache::needs_send` is also what keeps the cache honest — a universe recorded as
sent that never left would make the wire viewer describe traffic that does not exist.
`carries` is one predicate in `pult-schema`, called by the connectors and by
`OutputCoverage::of`, because the panel's coverage warnings and the socket disagreeing
about what an output carries is exactly the defect this field lived with for as long as
nobody read it.

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

## Which cable it goes out on

A console at a venue has a house LAN, an isolated lighting network with no route to
anything, and often a wifi the tablet is on. Until task 65 nothing here said which one
anything used: both mDNS daemons bound every interface they could find, sACN's multicast
left by whatever the route table said — `IP_MULTICAST_IF` was never set — and every
output socket bound `0.0.0.0:0`.

**And one default was wrong rather than merely unexpressive.** `infra::local_ipv4` finds
this machine's address by connecting a UDP socket to `8.8.8.8` and reading the local end
back, which is *the interface with the default route*. It was what a station advertised
over `_pult._tcp`, so the address a peer was told to dial for the show was decided by
which cable reaches the internet. It is now the fallback: a service advertises what it
actually bound.

**Six keys and a per-row map.** `[network]` in `preferences.toml` names `http`,
`session`, `mvr_xchange` and `openhaunt`, which bind on their own, plus `artnet` and
`sacn`, which bind nothing and are the fallback an output row takes. A station
preference and never show data — which cable is in which socket is a fact about the
machine. An **output** names its own per station in `OutputConfig::interfaces`, a
`BTreeMap<NodeId, String>`, because the row replicates and `en5` means a different cable
on every machine.

**A name or an address**, parsed as an `IpAddr` first and read as a name otherwise. Both,
because a name survives a DHCP lease and an address is the only way to say *which* alias
on a NIC carrying both 2.0.0.x and 10.0.0.x. IPv4 only, and a v6 literal is refused by
name rather than bound to nothing.

**Two absences, two answers**, and this is why no existing show or preferences file had
to change. Told nothing falls through to the preference and then to every interface,
exactly as before. Told an interface the machine has not got **refuses**, visibly —
`pult_schema::types::network::resolve` is the whole rule, and `infra/net.rs` is what
records the outcome so a console in the booth can read the stage rack's fault.

**An inventory is not a reading, and that is a collection of its own.** What a machine
has and what each service made of it live in `station_networks`, keyed by the station's
`NodeId`, written *only when they change* — not on the `Station` row beside CPU and
frame costs. That row is one measurement at one moment, replaced whole every two
seconds; an interface list changes when somebody plugs a cable in. Carried there it was
**57% of the row** — 2517 bytes against 1081, on a laptop with twenty-four interfaces of
which three have an address — rewritten every two seconds and logged to the oplog, for
names nobody had changed.

**The one exception is a listener, and it is deliberate.** The HTTP server falls back to
every interface with the fault recorded rather than refusing, because the page is how the
setting gets fixed: a console that will not serve its own UI over a mistyped cable cannot
be put right from the desk. A listener bound too widely *offers* something; a sender on
the wrong cable *does* something. `Network::bind` refuses and `Network::bind_listener`
falls back, so a service chooses by name.

**Re-resolved on the probe tick, both ways**, so a refused service starts by itself when
its cable appears and a running one whose cable drops reports the fault without being
torn down. Plugging the show network in never needs a restart. The output manager
reconciles on its own one-second timer for the same reason: a cable coming or going, and
a failover, change what should be running without anything writing to the show.

**Restricting an mDNS daemon restricts discovery too, and that is the feature.** A daemon
both advertises and browses, so a console kept on the lighting network will not *find* a
peer on the house LAN either — and adopting an OpenHaunt node it cannot reach is how a
fixture ends up on a cable that cannot carry it.

**Which station sends an output is part of the same question.** Art-Net and OpenHaunt
have no way to arbitrate between two senders, so `OutputConfig::node_id` being `None`
resolves to **the leader** for those kinds — resolved rather than refused, because `None`
is what every existing show has and is harmless on one console: a lone station is the
leader, and a second one joining stops the double-send instead of starting a fight. sACN
can share, having a priority byte, so an unowned sACN output runs everywhere and
`SacnPriority` makes that defined: `Auto` is the leader at 100 and each follower in a slot
below it, `Manual` is a map by station, and a station the map does not name takes its Auto
number. Slots are **self-claimed from the other stations' own rows** — each takes the
lowest one no *lower node id* holds, which converges in a pass with no leader arbitrating
and nothing new on the wire — and **sticky**, remembered in `preferences.toml`, because an
sACN receiver changing which source it follows is a visible jump on stage.

**Two stations on one Art-Net universe warn and are never refused.** Once an output names
its cable, stage-left out `en5` and stage-right out `en6` is a legitimate split, and
nothing here can tell it from two consoles shouting at one rack — they differ only in
whether the interfaces reach the same broadcast domain.

**The picker groups and never filters, which was also measured.** Hiding interfaces with
no address looks obviously right and is wrong: on the machine this was built on, the
address-less ones are `en1` to `en6` — the adapter ports, which is exactly where a show
LAN gets plugged in, and exactly what somebody wants to name before it is configured.
The re-resolve rule exists so a cable *can* be named before it is ready, and a picker
that hid it would put that out of reach of the UI. The filter does not sort the noise
correctly the other way either: a VPN's `utun` has an address and is never the answer.
So the ready ones come first and the rest stay reachable under *Not configured yet*.

Worth holding on to: **enumerating interfaces is on the runtime thread and was measured
before it was put there** — 0.5 ms for 24 of them, against the six seconds the first disk
enumeration cost, which is why the disks are on `station-probe` and this is not.

```
cargo test -p pult-schema network             # resolution, the ladder, the slots
cargo test -p pult-backend --lib infra::net   # the two absences, and a restricted daemon
cd frontend && npx vitest run src/lib/network.test.ts
```

## A recording is a function of time, and the wire can be read

The console can now read DMX as well as write it, and hold a position against which
things happen. Both halves exist for one reason, and it is worth stating before
anything else: **a recording is decoded values against time, never bytes.**

**A capture is `(fixture, parameter, value)` wherever it comes from.** So anything that
arrives on a wire and is going to land in the programmer or in a track goes through the
patch on the way in — byte to value through the mode the fixture is patched in, which is
the only thing that knows what the byte meant. That is what lets a take made off a guest
console play back onto a rig that has since been repatched, and it is why `input.grab`
writes `0.749` rather than `191`.

**`inputs` mirrors `outputs`, with two differences and both follow from a socket being
one machine's.** There is **no leader fallback**: an output with no `node_id` resolves
to the leader because Art-Net cannot arbitrate between two senders and *something* has
to send, where nothing has to listen — so a row naming no station means nobody is
listening and the panel says so. And `universes` is an explicit `BTreeMap<u16, u16>`
from **wire** universe to **patch** universe, empty by default: an input that guessed
the identity mapping would decode somebody else's universe 1 straight over the house
rig's. Everything else — the per-station `interfaces` map, the enable, the status row —
is the same field for the same reason, which is why they are one I/O panel.

**Received universes stay in the connector.** A universe is 512 bytes forty times a
second *per source*; putting that in `ShowState` would replicate somebody else's opinion
of the rig across the link carrying this one's. So the merged images are LOCAL to
`infra/connectors/input.rs` and drawn on demand through the **same** `Viewers` table an
output's wire view uses, with the input's row id standing in for an output's — which is
what makes a peer's input watchable with nothing added to the RPC or to the sync
protocol. Nothing crosses until somebody grabs or records.

**The merge is priority, then HTP, then a timeout.** Per patch universe, a table keyed
by the sACN CID or — for Art-Net, which carries no source identity at all — the
datagram's source address. Highest priority present wins outright; equals take the
highest of each slot; a source silent for 2.5 s is gone. That last rule is what makes a
console being *unplugged* different from a console *sending zeros*: the first drops out
and lets whatever else is there through, and the second wins on HTP against nothing.

**A colour decodes exactly on any fixture, and that is `unmix`.** `r`, `g` and `b` come
from the emitters that are primaries by `primary_channel`'s own rule — the same rule the
mix passes them through going out — and then every emitter whose received level differs
from what that colour would derive becomes an **override**. A CMY head has no primaries
and comes back as three overrides over black; an RGBW head whose white is somewhere the
mix would never put it comes back with the white pinned; a plain RGB par comes back with
nothing pinned, which is a colour an operator can then edit. Half a byte of tolerance,
because the levels came off a wire as bytes: tighter and every emitter of every fixture
would be pinned, and the next colour command would do nothing.

**`pult_render::track` is the codec, and there is one of it.** Hand-written
little-endian — magic `PLTK`, a `u16` version, per `(fixture, key)` a list of change
points — read and written by the crate that is compiled twice, so the browser hands the
bytes to the wasm through `load_track` and the station reads the same bytes for a wire.
A recording is the largest thing that has ever driven a parameter and two readers of it
would drift exactly where it is longest. Replay is **stepwise**: the recorder wrote a
point when the value *changed*, so holding the latest one reproduces what came down the
wire, and interpolating would invent motion nobody sent. A track says **nothing at all**
before its first point, so what is under it shows through.

**Stack order is programmer > track > effect > fade > home.** A recording is somebody
else's console asserting a value, so it beats the playback it was recorded over and
loses to the operator standing at this desk. It is also the only layer that can be
present and say nothing, and the only one that makes `settles_at` answer `None` without
being an effect — a connector must not drop to its keep-alive while a take is rolling.

**A timeline says *when*; a sequence stays the stack.** An event is a position and a Go
into any sequence, so one song can drive three of them. Running state is a SYNCED
anchor, the shape `Sequence::went_at` has: `running`, `anchor_ms`,
`position_at_anchor_ms`, `rate`, and the position everywhere is arithmetic over it. **No
tick anywhere** — the panel's readout is a `requestAnimationFrame` loop over that
arithmetic, and it shows a gap rather than a figure until `consoleNow()` has an offset.
`play`, `stop`, `locate` and `record` carry `at` the way a Go does — and where a
timeline has sound, the station playing it rewrites that anchor from what the
loudspeakers are actually doing. See *A song the console can hear*.

**Only the leader fires, and the pass runs everywhere.** A crossed event becomes an
ordinary `run_synced_command` on the sequence, replicated like any Go, with `at` set to
the wall millisecond the *event* fell at rather than the moment the pass ran. The record
of what has been fired is kept on every station, though, or a promoted follower would
re-fire the first half of the song. A locate — or a play from a position — takes the
**tracked state** instead: the latest event per sequence at or before the new position,
applied once, with nothing crossed backwards fired.

Two rules there were each a defect first. **The deadline says when to wake, not what to
do**: gated on its own `next_deadline` having arrived, the pass woke *past* the event,
found no next one, and returned early having skipped what it was woken for. And **a jump
is told from progress exactly, never by a tolerance**: ask the new transport where the
playhead was at the previous pass's moment, and if that is not where the previous pass
saw it, somebody moved it. Guessing how far it should have got needs a tolerance, and a
tolerance on a busy station is a Go that silently did not happen.

**Record is armed on the timeline and covers all the input carries.** `recording` is a
SYNCED field naming an input — SYNCED because the station that *arms* it is usually not
the one holding the socket. The `InputManager` reconciles against it the way it
reconciles sockets: running and armed at an input this station holds means recording.
It reads the patch **once**, restricted to the fixtures the input's universes reach,
then appends a change point per key whose value moved, at `timeline.position_at(now)`.
On stop it encodes, `AssetStore::put`s under `application/vnd.pult.track`, appends a
`TimelineTrack` and clears `recording`. Selection-scoped recording was declined: a take
made with the wrong selection is missing fixtures nobody notices until playback.

**Stopping a timeline lets its keys go.** The engine samples each enabled track at the
stop position — through `infra/tracks.rs`, the *same* cache the output manager reads, so
the two cannot disagree about what a stopping track was showing — and hands them to
`Playback::release_tracks`. They fade home over `home_fade_ms` into the stack beneath,
evaluated **at the moment the release lands**, which is the honest approximation: the
model has one fade per key and a fade still in flight underneath cannot be joined. A key
the programmer holds is untouched, and a key with an effect under it is left alone, since
an effect beats a fade and a release fade under one would be invisible.

**An input binds a fixed port, which is the whole difference from an output.** Every
output socket leaves by an ephemeral port; an input sits on 5568 or 6454 beside whatever
else on the machine is listening to the same show LAN, so `SO_REUSEADDR` (and
`SO_REUSEPORT`) is required. Bound to `0.0.0.0` even when an interface is named: a
multicast group is joined *on* an interface as a separate act, and binding a receiver to
a unicast address is what stops broadcast Art-Net arriving at all.

`SCHEMA_GENERATION` is 5. `FollowMode::Timecode` is gone — a position written on a cue
is a clock the cue cannot see, and the same fact as a `timelines` event is a list an
operator can read and drag.

```
cargo test -p pult-render track                      # the codec, and what it refuses
cargo test -p pult-render color                      # mix and unmix, both ways
cargo test -p pult-backend --lib connectors::dmx     # render, decode, render again
cargo test -p pult-backend --lib connectors::input   # the merge, and what times out
cargo test -p pult-backend --lib model::timelines    # crossing, jumping, stopping
cargo test -p pult-backend --test timecode           # a wire read back, and a cue that went
cd frontend && npx vitest run src/lib/evaluator.test.ts
```

## A song the console can hear

Task 66 gave a timeline a position; this is the sound that position is usually about.
Playing a file, drawing it, finding the beats in it, and chasing somebody else's clock.

**One station plays, and the audio callback's sample clock is the reference.**
`Timeline::node_id` says which — or the leader, the rule `OutputConfig::runs_on` follows
for Art-Net — and it measures the playhead off the frames its own callback delivered,
not off a wall clock. A sound card's crystal and a computer's are two different
crystals, and the one the audience can hear is the one that has to be right. **A tablet
plays nothing**: Web Audio was declined outright, because the reference clock must not
be a tab a browser can throttle to one frame a minute.

The playhead is that cursor **minus the device's own reported latency**, and the
subtraction is not a detail: a callback that has just filled a buffer has read ahead of
what anybody can hear, and a console anchoring the show to the read position runs every
cue a buffer early — 12 ms at 512 frames and 44.1 kHz, five times that on a machine set
up for safety.

**Twenty milliseconds is the whole rule.** Under it, nothing happens. Over it, the
station rewrites the timeline's SYNCED anchor — as the timeline's own `locate`, so it
replicates as one operation the way a Go does — and every other station and every
browser follows the loudspeakers. The threshold is `pult_schema::clock`'s own, taken
deliberately: the anchor is the *show's*, and a station rewriting it every buffer would
put jitter on the link carrying the show for a correction nobody can perceive, with
every browser's playhead visibly stuttering. **Audio never gates the transport** — a
timeline with no audio, or on a station with no device, runs exactly as task 66 left it,
and reports *nothing* rather than an empty status row.

**Peaks are an asset, computed once by the station that plays the file.** Min and max
per bin at a hundred bins a second, as `i16`, under `application/vnd.pult.peaks`;
`Timeline::peaks` is the sha. Min and max because a waveform is read as an envelope and
a snare that peaks for two milliseconds is invisible in its bin's mean; `i16` because
the sixteenth bit of a column's height is not a thing anybody can see and it halves what
the tablet downloads. Computed by the *playing* station, so two of them do not race to
write the row. It is the one format in this console with two readers —
`pult_audio::peaks` and `frontend/src/lib/waveform.ts` — and it earns that by being
eighteen bytes of header and a pair of `i16`s, with no arithmetic in it to drift.

**The detector is Beat This! (CPJKU, MIT for code and weights) through `rten`.** Every
conventional one is copyleft or Python: aubio GPL-3, BTrack GPL-3, Essentia AGPL-3,
MiniBPM GPL-2, madmom and librosa Python. Beat This! also gives **downbeats**, which is
what a speed master wants. Its inference path is **vendored** into `beats.rs` with
attribution — the `beat-this` crate's `clap`, `glob` and `hound` are not optional — and
the models are `.onnx` **checked in byte for byte as upstream published them**, because
`rten` 0.26 loads ONNX directly and there is therefore no Python in this build and a
sha256 that means something. The small model and the mel front end are **embedded** (a
venue has no internet); the 83 MB full model is fetched into the config directory on
request and verified before a byte is written. **The detector proposes and the operator
confirms**: it writes `Timeline::detected`, drawn over the waveform, and one button
turns it into `grid`.

**LTC is declared, never sniffed.** What the wire carries is a frame *count*; whether
thirty of them is a second is not in the signal at all, and 29.97 non-drop against 30 is
one frame in a thousand — right all afternoon and wrong at the performance. The chase is
`pult_audio::lock`: `Hold` under 20 ms, `Varispeed` within ±2% up to a second, `Locate`
beyond it. ±2% is a third of a semitone, past which a resampled stem is audibly wrong,
and it bounds the correction in time — twenty milliseconds closes in one second. **The
anchor is written from the timecode's position and never from our playhead**, because
the generator is the authority. **Lock loss stops nothing**, and **chasing does not press
Play**: whether a timeline runs stays the operator's.

**A timeline drives a speed master as a bounded step.** Crossing a grid segment writes
that master's `bpm` and `t0` — the wall millisecond the segment's downbeat fell at,
worked out backwards from the anchor, so a late pass still puts the "one" where it
belonged. One write per segment and none in between, which is `types::speedmaster`'s
discipline: the rate and the anchor it is measured from arrive together. A locate
restates it; stopping does not, so the chases run at the song's tempo through the
applause.

**Which device is a station preference, and a wrong one refuses visibly.** `[audio]` in
`preferences.toml` names `output` and `input`, matched by the device's own name or by
the platform's id — the rule `types::network::resolve` follows for a cable, and for the
same reason. Told nothing is the system default; told a device this machine has not got
is a fault on the `audio_status` row rather than a silent fall-back, because from the
desk "playing out of the wrong output" and "playing out of nothing" look identical.

**And every blocking call is on a thread.** `cpal::Stream` is not `Send`, so a stream
lives on a thread that answers over a `oneshot`; enumeration and decoding go through
`spawn_blocking`, for the reason `infra/stations` puts the disks on a probe thread.
**No test opens a device**: `AudioHandle::feed_timecode` pushes samples into exactly the
queue a `cpal` input thread pushes them into, so a generated LTC stream exercises the
real decoder, the real chase, the real anchor write and the real firing.

```
cargo test -p pult-audio                             # peaks, LTC, the detector, the chase
cargo test -p pult-backend --lib infra::audio        # who plays, and the two conversions
cargo test -p pult-backend --lib model::timelines    # the speed-master step
cargo test -p pult-backend --test audio              # timecode in, an anchor and a Go out
cd frontend && npx vitest run src/lib/waveform.test.ts
```


## Paperwork is a drawing, and it is drawn twice

The rig has to leave the console as paper: A3 sheets with a title block, viewports at a
stated scale, labelled plan heads, and tables of weight, power and counts. The roadmap
called this "a read-only plugin over introspection" and that was half right, which is
the interesting half.

**The tables are the station's and the drawing is the browser's.** A WASM guest has no
meshes, no HTTP route and no file output, and only `frontend/src/lib/geometry.ts` ever
measures a mesh — so a plugin cannot draw a truss without shipping a second copy of the
rig renderer. But the *arithmetic* is rows and sums over what the station already holds,
so it lives in `pult_schema::types::paperwork` behind the **`paperwork.tables`** RPC,
and a plugin, the command line and curl can all ask what a truss weighs. Written in the
browser beside the renderer, that number would have been reachable by nothing.

**One drawing model, rendered twice.** `frontend/src/lib/paperwork/drawing.ts` is lines,
polylines, text and images in **paper millimetres from the top left**; `svg.ts` renders
it for the preview and `pdf.ts` for the file. There is no SVG-to-PDF converter anywhere,
deliberately — a converter is a third implementation of the page with its own opinions
about dashes, joins and text placement, and it is what would make the file differ from
the preview somebody signed off. The PDF's whole conversion is one flip, `y →
(height − y)·mm`, handed to `drawSvgPath`'s own transform rather than applied twice.

Two things are one implementation on purpose, and both were briefly two. **Clipping** is
`drawing.ts`'s, not each renderer's: Liang–Barsky for lines, Sutherland–Hodgman for
fills, so a truss half out of a viewport is cut the same way in both and the corpus can
assert it. And **`textOrigin`** in `font.ts` resolves anchor and baseline for both, so
`text-anchor` is always `start` in the SVG — SVG could do it and PDF cannot, and letting
each do it its own way is the same trap one level down.

**One font, shipped, measured through fontkit.** Open Sans under the OFL in
`frontend/static/fonts/`, embedded in the PDF by `pdf-lib` and asked for by the SVG
through the same URL. Layout depends on its metrics — de-collision decides where a
label goes by *how wide it is*, and a column is as wide as its widest cell — so there is
**no metric fallback**: a sheet cannot be drawn until the font has loaded. A second
answer to that question is a page laid out differently on a Linux tablet than on the
desk it was approved from.

**A viewport is either drafting or picture, and only one of them has a scale.**
Drafting is vector: each object's projected convex outline, filled white, painted far to
near, which is hidden-line's look for none of its cost and is exact for the convex
pieces a rig is mostly made of. Picture is the rig renderer through an offscreen
`Rig3D` — `forCapture` turns on `preserveDrawingBuffer`, and `captureFrame` frames,
dresses and renders **synchronously**, because the caller reads the buffer as soon as it
returns and one animation frame later the pixels may be gone. It also must not call
`frame()`, which writes the projection into `stores/view.ts`: a document re-projecting
somebody's open rig panel is a document editing a workspace.

`Fit` **snaps to a standard ratio** rather than printing what a free fit came to — the
Vectorworks sheet this was designed against prints 1:35, which no rule measures. And a
picture viewport prints **NTS**, orthographic or not: the rig renderer fits its own
camera and this code did not choose that fit, so a ratio here would be a number nothing
worked out. It printed `1:0` before that was written down.

**Nothing prints a bare total, anywhere.** `Totals` carries what it could *not* account
for beside what it summed — how many items had no figure, and whether any of it came
from the catalogue's nominal weights — and `Totals::weight_label` is the one place that
becomes text, so the PDF, the CSV and a plugin's answer cannot disagree about whether a
number is complete. `≥ 412 kg nominal` is a different claim from `412 kg`, and `—` is
what a total with nothing in it says, because zero reads as a rig that weighs nothing.
A filtered table captions what it left out; a plan that could not place twelve fixtures
says so in its margin. **A loading table is a number somebody hangs a truss on**, and
this is the whole reason the module is shaped the way it is.

**Catalogue weights are nominal and say so.** `StockPiece::weight_kg` is a round figure
for the *class* of part — 290 mm four-chord box truss, 48.3 × 4 mm tube, an
aluminium-and-ply deck — taken at the heavy end, because the direction to be wrong about
a load is upwards. `SceneObject::weight_kg` overrides one, and a total resting on
entered weights is printed differently from one resting on these.

**Waiting is part of correctness here.** `geometry.ts` hands back a placeholder box for
a mesh it cannot load, which is right for a rig view and wrong for a rigging plan — a
box is a drawing of something that is not there. So an export polls `pendingGeometry()`
until every piece has its mesh and then **refuses**, naming what did not arrive, rather
than printing what it has. The first exported PDF had no trusses on it at all and said
nothing, because two animation frames is not a download.

Three smaller rules with reasons. **A captured line mode clears to white** and dresses
its models in flat grey — a rig view is a dark studio, and a shaded axonometric on A3
came out as a near-black rectangle. *Real* and *photoreal* keep the dark ground, which
is not an inconsistency: a beam is additive light and there is no version of one on
white. **Fixture bodies keep their own material** even on paper, because the per-frame
update writes `emissive` on it and a `MeshBasicMaterial` has none. And **label
de-collision only ever pushes upwards** — splitting the push sends the lower label onto
its own head, `clearOfHead` lifts it back, and the two rules chase each other until the
passes run out.

**The sheets are the show's, seeded on `show.new` only.** A PERSISTED `sheets`
collection with six defaults mirroring the reference set. Two consequences, recorded
because they will be asked about: a showfile made before this feature opens with no
paperwork and there is no action that adds it, and a show made today never picks up a
later version's better default sheets. `layouts` does it the other way; this was the
user's call.

**A rendered viewport says which cues are running, and when.** A beauty shot of a rig
with nothing on is a photograph of a dark room, so `ViewStyle::Picture` carries a list of
`CueShot` — a cue and how long it has been running. The time is the point: a fade sampled
at zero is the state *before* the cue, and an effect at zero has every head at the same
phase, so both are the least useful frame of the look. **Nothing is taken**: the
`paperwork.cueValues` RPC tracks the stack with `cue::tracked_through` — the same
function a Go uses — turns each capture into the fade or effect it would have started,
anchored so it has run exactly that long, and evaluates it. A document that put the rig
into the state it was drawing would be changing the show it documents, visibly, in the
room. Fades run from the parameter's **home** value, not from what the console happens
to be doing, or the same sheet would export differently every time.

**Previews render in the background, debounced, and only for the sheet being looked at.**
At screen dpi rather than the sheet's own, because a preview is looked at on a screen.
The signature that decides whether to redraw is everything that changes the pixels and
nothing that does not — the block's rectangle is deliberately *not* in it, so dragging a
viewport across the page does not re-render it every frame. And every wait in that path
races `requestAnimationFrame` against a timer, because **a backgrounded tab is served no
animation frames at all** and an export that stalled on one would report a failure thirty
seconds later that had not happened.

**A 2D viewport can dimension each bar**, which is the only question anybody up a ladder
is asking. `Dimensions` carries the datum — left, right or centre, because every crew has
a convention and none of them is right — and draws a gap chain plus a running chain, in
one unit per bar decided by its longest figure. Three things there were wrong first and
are worth keeping: the chain is measured **along the bar in its own frame**, not across
the page, so a truss at forty degrees reads the same as one square on; it is grouped by
**what the lights actually hang off**, which is the `Group` handle rather than the truss
sections under it, and a group's length is the span of its children; and the running
figures are a *chain* with the figure at each tick, not twelve dimensions from the datum
piled on one line. Off by default on every sheet but Fixtures — on a 200-fixture festival
plan at 1:200 a chain per bar buries the drawing.

**Labels are a drafting thing and there is no picture version of them.** A rendered view
is a photograph; one with channel numbers written over it is neither one thing nor the
other. The composer draws none on a picture and the editor does not offer the controls.

**And a sheet is laid out on the sheet.** Blocks are dragged and resized on the preview
itself through an overlay of handles — *outside* the SVG, because the drawing is a
rendering of the model and the thing being dragged has to be the block. Two rules make
that safe, and both are the same rule: a rectangle dragged through itself stops rather
than turning inside out, and a block dragged off the paper keeps a corner on it, because
in either case what you get back is a block that draws as nothing and can never be
grabbed again. `resizedRect` is pure and in `stores/paperwork.ts` so both are tested.

A drag is **one gesture and one write per animation frame**, which is `stores/editor.ts`'s
rule for dragging a truss applied to dragging a viewport — and `Sheet::blocks` is one
column, so each of those frames rewrites the whole array. `a_dragged_sheet_block_is_one_row`
in the counts test is the gate: without the gesture, putting a viewport back is a key
somebody holds down. The title block's own text is `Show::production`, edited in the Show
panel, because it is a fact about the production rather than about a sheet.

```
cargo test -p pult-schema paperwork        # the tables, and what they refuse to say
cargo test -p pult-backend --test counts   # a dragged block is one Ctrl-Z
cd frontend && npx vitest run src/lib/paperwork   # both renderings of one page
```

## A rig can leave while it is still being drawn

MVR as a file is `/api/import/mvr` and `/api/export/mvr`. **MVR-xchange is the same
scene over the network**, between this console and a previz or a CAD seat, as it
changes. `crates/pult-mvr-xchange` is the protocol on paper — messages, and the framing
around them — with no socket, no runtime and no MVR content in it, so its corpus is
testable with no station near it the way `pult-gdtf`'s is.
`crates/pult-backend/src/infra/interop/xchange/` is the console's half.

**Two modes, and they do not combine.** DIN SPEC 15801 defines *TCP mode* — mDNS
discovery under `<group>._mvrxchange._tcp.local.`, plus a framing of the protocol's own
(`778682`, a version, a package number and count, a payload kind, a 64-bit length, all
big-endian) — and *WebSocket mode*, ordinary DNS to a URL somebody hosts, JSON in text
frames and files in binary ones. Both are built, and this console can be a WebSocket
host as well as a client: hosting is the `/mvrxchange` route on the port already serving
the page, so the address to hand somebody is the one they already use. The show names
one group at a time.

**Only the leader is on the wire, and the identity it presents is the show's.** A pult
session is several stations replicating one show; if all of them advertised, one
`MVR_COMMIT` from a previz would arrive at each and every one would import the same file
into the same replicated show — three plans, three gestures, racing. So there is one
exchange client per show, hosted by whichever station is leading, `station_uuid` is a v5
over the **show id** and `StationName` is the show's name. A failover then reads to a
peer as one console that moved rather than one that vanished and another that appeared.

**Which is why the state is LOCAL on every station and pushed by the one running it.**
`SyncMessage::XchangeState` carries it out and `SyncMessage::XchangeAsk` carries an
operator's act back to the leader with their user id in it — the shapes `LogLines` and
`LogRaise` already have, at protocol version 7. **Not a SYNCED entity**, deliberately:
the engine logs every non-LOCAL write to the oplog, so a discovered laptop appearing on
the LAN would be a row in the History panel and something in an undo stack.

**The group is show data; the veto is the station's.** `Show::mvr_xchange` holds the
group, the mode and the URL, and is off until switched on — because the leader moves,
and a group kept per station would change group on a failover. What is a fact about the
machine is `mvr_xchange` in `preferences.toml`: whether this console may take part at
all.

**A commit is deliberate, and applying is one act.** A button and a required comment
(an empty one is a commit nobody can tell from the last), the whole rig, never targeted.
Applying sends the request, waits for the archive and runs the *same* import the REST
route runs — one gesture, one Ctrl-Z, attributed to whoever clicked from wherever they
clicked. `interop/mvr::read_rig`/`write_rig` are that shared implementation, moved out of
the two handlers so a socket and an HTTP route cannot disagree about what an export
contains. A live sequence is a **warning and not a refusal**: `xchange.apply` answers
`{needsConfirm, running}` and the panel asks, because a console cannot know whether the
house is in and a rule that refused while anything was live would refuse all afternoon.

**Commits live beside `preferences.toml`, capped by a count.** An announcement is a claim
that bytes exist and the bytes move later — possibly to a station that joins tomorrow and
reads the history out of `MVR_JOIN` — so the archive has to still exist byte for byte.
Not the asset store, which would put every commit ever made in the `.pultz` and on the
link carrying the show; not regenerated, which would answer a request for last Tuesday
with today's rig. And this station announces **what it holds, not what it once made**:
`ours` and `here` are separate fields, so after a failover the history is shorter and
every entry in it is real.

Three traps, each of which was a defect first.

**A TCP station's address comes from mDNS and from nowhere else.** A message arriving
over TCP comes from the *ephemeral* port the sender dialled out of, and every interaction
in that mode is a short connection — so remembering a station from an inbound message
overwrote its discovered listening address with one that was dead on close, and commits
were announced to a closed port. `Known::dialable` is what says which addresses can be
opened.

**A station that comes up already settled must still say so.** The ordinary case — the
exchange off — changes nothing on the manager's first look and took the "nothing moved"
early return, leaving every panel showing a default state with no reason in it.

**And `MVR_NEW_SESSION_HOST` is answered `OK: false`.** In that protocol `true` means "I
have moved". An unauthenticated station on the LAN naming a host to connect to is a
redirect, so it becomes a prompt in the panel with the sender and the URL in it, and
following it writes the *show's* settings — every station of a session has to agree which
group it is in.

The specification disagrees with itself in four places, each somewhere a strict parser
would refuse a message the document itself prints; all four are read both ways, written
the table's way, and pinned by a corpus case. The subtlest is the package field order,
listed count-before-number in Table 66 and number-before-count in the byte layout under
it — invisible in an unchunked message, where number 0 of count 1 is the same bytes
either way.

**None of it has been pointed at grandMA3 or Vectorworks.** The TCP half can be checked
at a venue in an hour; a group this console hosts has no second implementation anywhere.

```
cargo test -p pult-mvr-xchange              # the messages, the framing, the corpus
cargo test -p pult-backend --lib xchange    # the rules, and two consoles over TCP
cargo test -p pult-backend --test xchange   # two stations, one hosting and one joining
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
cd plugins && cargo test       # the plugins workspace's host-buildable crates
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
