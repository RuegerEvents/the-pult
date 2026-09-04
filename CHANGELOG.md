# Changelog

Notable changes, newest first, in the spirit of [Keep a Changelog][kac], with
[semantic][semver] versions. The release workflow extracts the section whose
heading starts with the version being tagged — plainly, `## 0.0.1`, not the
bracketed form — so every release needs one and it has to be spelled that way.

[kac]: https://keepachangelog.com/en/1.1.0/
[semver]: https://semver.org/spec/v2.0.0.html

## Unreleased

### Added

- **A show is a folder, and Save is a version.** A showfile is now `Name.pult/` — the
  database, the assets as files, and a snapshot per saved version — with `.pultz` as
  the single file that travels. ⌘S takes a version; the Show panel lists them with who
  took each one and restores any this station holds. Save as, export, import, and
  autosave every fifteen minutes with a rolling window.
- **A welcome screen.** Started with no arguments the console comes up with no show
  open and offers recent shows, the shows folder, sessions on the network, a `.pultz`
  to import, and four demo shows it makes for itself: Haunt, Theatre, Club and
  Festival. `--show` and `--demo` skip it.
- **A stock catalogue.** The rig view can draw F34 truss, stage decks, wall panels and
  flats without being given a mesh, so a console that has never imported a drawing
  still has a room to hang lights in.
- **Beams that read as light.** The 3D rig view draws a real volumetric beam: bright
  in the middle and nothing at the edge, flaring when you look into the lens, starting
  at the width of the lens rather than at a point, fading out over the deck rather than
  clipping through it, and a dim beam keeps its hue instead of going grey. The beam
  angle is the one the fixture's own GDTF measured. How hazy the room is is a setting
  on the show, seeded from a station preference, so everyone opening the file sees the
  room the designer lit.
- **Strobe and shutter are drawn.** A strobing fixture flashes in the rig view and a
  closed shutter puts its beam out. Both parameters already existed and already came
  in and out of a GDTF; nothing had ever shown them.
- **`--size <n>` measures any rig.** `scripts/demo.sh --size 5000` seeds whatever
  count is asked for, with `--cues` and `--slice` as separate axes so one thing moves
  at a time. `--measure-browser` is a new mode that opens a headless page and prints
  what the *browser* costs, kept apart from `--measure` because a page drawing the rig
  competes for exactly the CPU that run is holding still.
- **A frame is reported in three parts.** Evaluating, assembling universes, and the
  socket write, where it used to be two. At 5000 fixtures this is what shows that
  assembly and the socket are 6% of a frame and evaluating is 94% — the opposite of
  what was expected.
- **Opening a show is one screen.** Opening, closing, restoring or copying a show
  used to show "the console stopped answering" and then a reload; it now says
  "Opening Festival…" from the moment the button is pressed until the page is
  looking at the result — on the console that pressed it and on every other browser
  on that station, which the station tells why it is stopping.
- **A View sheet on the rig panel.** A work light for how brightly this screen draws
  what no fixture is lighting, and the resolution the view renders at. Kept on this
  screen, not in the show; the haze stays the show's.
- **The rig panel says what the GPU took.** Beside the page's own work per frame,
  where the browser will say, and *idle* when nothing was drawn — which is the
  figure that explains a view that reads nine milliseconds and still feels laggy.

### Changed

- **Playback tracks.** A cue is now the stack up to it: jumping back to a cue lets go
  of everything only later cues brought in, and jumping forward applies what the cues
  in between set. Going forward one cue at a time behaves exactly as before.
- **Haze density means how much of a beam shows.** At 0 the air is clear and no beam
  is drawn; at 1 every beam shows with its folds. It used to only mix the folds in,
  so a clear room still drew every beam. New shows start at 1.
- A station's identity moved out of the showfile and onto the machine
  (`--identity`, `PULT_IDENTITY`, or the configuration directory), so copying a show
  no longer clones the console that made it.
- Opening a showfile compacts it when more than a quarter of the file is free.
- `--showfile <file>` is now `--show <bundle>`. Showfiles are generation 3; a file
  from an older build is refused by name, as before.
- **The showfile is written off the engine's own thread, in batches.** One operator's
  edit no longer waits behind another operator's disk. A write is still acknowledged
  only once it is durable; what changed is that a group of them share one commit,
  sized by whatever arrived while the last one was in flight rather than by any
  constant.
- **Plugins, browsers and peers each have their own queue into the engine.** They
  shared one, so a plugin in a write loop or a peer catching up after an absence could
  make an operator's fader stop responding. Each class now gets its own bounded queue
  and a share of the turns.
- **The rig view no longer uses Threlte.** It draws directly, which removed two
  defects rather than fixing them: a geometry was being rebuilt for every fixture on
  every frame of every fade, and every material in the scene was being recompiled
  whenever a fade crossed 1% of full. The grid is now infinite and antialiased instead
  of stopping at 40 metres, gizmo rings can no longer hide inside the fixture they
  belong to, and touching the view cancels a camera move in progress.

- **The rig view draws only when there is something new to see**, and at most sixty
  times a second. A ProMotion display was being drawn a hundred and twenty times a
  second whether or not anything moved, with every unlit fixture still rasterised as
  a cone that discarded every fragment; a dark theatre held a GPU at forty percent
  and the festival pinned one. A settled dark rig now costs nothing.
- **The Festival demo is a rig of five kinds** — profiles, spots, washes, blinders,
  beams and LED strobes on six trusses, a floor package and two towers — with seven
  playbacks of looks that end in *Out*, some running and some waiting for Go.

### Fixed

- **The Theatre demo's booms were horizontal**, and the Club demo's washes hung in the
  air beside the truss. Booms stand up now, with their lanterns on sidearms, and
  everything in every demo is clamped under its bar at the same distance. The cyc
  batten's cells no longer overhang its ends.
- **Moving the rig view's work light took the camera home.** It no longer rebuilds
  the scene, and the slider runs from blackout to house lights up in percent.
- **Patching a large rig was quadratic, twice over.** Creating a fixture rewrote the
  whole collection's stored order *and* re-sent every fixture in the show to every
  connected browser. Both are now done once per burst instead of once per fixture.
  Seeding 2000 fixtures went from 21.9 seconds to 1.0, and 5000 with a full cue stack
  from over two minutes to 6.2. The same path is what an MVR import uses, so importing a
  real drawing gets the same twentyfold.
- **`--measure` disagreed with itself by 50%.** It read a single reporting window at an
  arbitrary moment, which could still be half full of the cues it had just taken to get
  the show moving. It now waits for that to go quiet, takes several windows, discards
  the first, and prints the spread beside the median so a reader can see how much the
  number could have come out different. Two consecutive runs now agree to within 3.4%.
- **A station stood still while it read the machine.** The reporter enumerated the
  volumes and the thermal sensors on the runtime's own thread, and the first of those
  can take seconds on a Mac — as long as the operating system needs to read the
  directory the executable sits in. For that long the station accepted no connection
  and ran no timer. The reading happens on a thread of its own now, so a station answers
  the moment it starts however slow the machine's probes are, and a row carries the
  latest reading rather than waiting for the next one.

### Added

- **A panel that shows what actually leaves the console.** *On the wire* is the sheet
  a DMX universe went out as, and the messages a node was sent — where the Outputs
  panel is where an output is configured and the System panel says how many bytes went,
  this one says *which* bytes. Every universe a connector carries is listed with how
  many of its channels are above zero and whether anything in it has moved recently,
  and clicking one shows all 512. It works on another station's outputs too: only the
  console holding the socket can say what went through it, so the question crosses the
  network and that station answers it.
- **It costs nothing when nobody is looking at it.** A universe is 512 bytes forty
  times a second, so a console that published that continuously would be spending the
  show's own network on a picture nobody is reading. Instead a connector is asked only
  while a panel is open on it, ten times a second rather than forty, and an answer that
  has not changed is not sent again — so a rig sitting still is free to watch. Closing
  the panel stops it, and so does closing the tab, losing the browser, or unplugging
  the station that was asking.
- **A new kind of output brings its own view with it.** A connector describes its own
  traffic in shapes rather than in protocols — whole universes, or discrete messages —
  so anything that carries universes gets the DMX sheet for nothing, and anything that
  does not adds one component and one line in a table with no panel changing. A shape
  a console has never heard of is drawn as itself rather than quietly left out, and a
  connector that does not describe what it sends says so.

- **A System panel, and it can see the browsers too.** What every machine is costing,
  in one place, with a sparkline beside each figure for what the panel has watched
  since it was opened. What each output connector's frames took is finally shown at
  all — the console has measured that since 0.1.0 and nothing displayed it. And the
  number nobody had before it: a console is a browser working out
  what the rig is doing on every frame it draws, so the machine struggling in a room
  where every station is comfortable can be the tablet at the back of it. Each browser
  now reports its own frame rate, the time it spends in that arithmetic, how many
  parameters it is watching, its memory, and how far its clock is from the station's —
  which is the one figure that says whether anything else it is showing can be
  trusted. A page drawing nothing says so rather than reporting a frame rate of zero.
- **What the machine is doing, not only what the console is doing.** Every station
  already said what the pult process was costing; it now says what the box around it
  is costing too — processor across every core, memory and swap, load average, how
  long the machine has been up as against how long the console has, free space on the
  drive the show is actually saved to, and how warm it is where the machine will say.
  The two are shown side by side and never added together, because the useful reading
  is the comparison: a console using 4% of a machine that is at 96% is not comfortable,
  it is about to be starved by something nobody is watching. A card says so in words
  when the machine is short of processor, memory or disk.
- **What is on the wire, in four figures rather than one.** What each output
  connector is actually sending — after the skip that leaves an unchanged universe
  alone, so a settled rig honestly reads lighter than a moving one; what the link to
  each peer is carrying, both ways; what the station is sending each browser, which is
  the cost of somebody watching a busy panel; and what the machine's own network
  interfaces are carrying, which includes everything else on the box. Kept apart on
  purpose: a console whose own traffic is a fraction of its machine's has a network
  problem that is not the console's, and one where the two agree has found its own.
- **A browser that cannot keep up says so where every console can read it.** A page
  under 20 frames a second, or one that stalls for more than a tenth of a second in a
  single frame, writes a warning into the station's log, which reaches the other
  consoles like any other line. Its continuous figures stay with the station serving
  it — a frame rate every second is not worth putting on the show's network, and the
  moment it goes wrong is.
- **The console can show its own log.** A System Log panel, because until now the log
  went to a stdout that does not exist under the desktop app, under a packaged `.app`,
  or in a browser — and any browser on the network is a whole console here. A plugin
  saying something about itself went to the same nowhere, which was the strongest
  reason to build this first: it was the audience with no way round it. The panel
  filters by level, by plugin, and by text, follows the tail, and says out loud when
  lines were dropped rather than quietly skipping them. Each run is also written to a
  file beside the station's preferences, so the crash somebody went looking for is
  still readable next time the console comes up.
- **A peer's log, from the booth.** Every station puts its warnings and errors on the
  session by default, so a console in the roof reports its trouble to the desk without
  anybody climbing to it — and nobody's `debug` crosses the network that is carrying
  the show. Clicking a peer's chip asks that station for more while somebody is
  watching, and letting go puts it back; a console asks only as far as what that
  station is keeping for itself, and no console can change what another one writes
  down. Two settings, `log_level` and `peer_log_level`, in the station's preferences.
- **A browser says when it breaks.** An exception inside a panel now reaches the
  station's log instead of dying in a tab nobody has open — folded into one line and a
  count when it happens every frame — so a tablet at the back of the room that has
  stopped working says so where somebody will see it.
- **A whole rig, from the drawing.** MVR — My Virtual Rig — read and written. Import
  one and the fixtures arrive patched, at the addresses and in the modes the drawing
  gives them, where the drawing puts them; so do the trusses and the objects around
  them, with their own meshes, and the layers they are drawn on. Export writes it back
  out, with each fixture type's own file where it arrived as one. Every uuid is the
  one the file uses, so re-importing an updated drawing updates the rig rather than
  doubling it — and what an earlier import left that the new one no longer mentions is
  reported, never deleted, because somebody may have taken that light out on purpose.
  A fixture whose definition the archive does not carry keeps its address and its
  place under a placeholder type until the real file turns up.
- **The stage views draw the drawing.** Trusses and objects in the plan and in three
  dimensions, from the meshes the archive carried. A Layers panel shows and hides
  parts of the rig — per browser, so two people can work on different parts of it at
  once — and locking a layer is the show's. A mesh the browser cannot read becomes a
  box rather than an empty view.
- **Fixture definitions from a file.** GDTF in and out, with modes, breaks, wheels,
  emitters, physical data and the geometry tree, so a fixture brings its real beam
  angle and its real pan and tilt travel instead of the console guessing. The archive
  is kept whole and exports byte for byte. The GDTF Share is searchable and importable
  from the Fixture Types panel, behind a login kept in the station's preferences and
  never in the show — a showfile travels, and a password in one travels with it.
- **A cue can fade two ways.** The *out* time beside the *in* time, on the cue and on
  each captured value: a parameter takes the out time when the cue asks it to come
  down and the in time when it asks it to go up, which is the split fade every
  theatre desk has. A cue with no out time set fades one way in both directions, so
  nothing an existing show does has changed. Only levels and whole numbers can be
  said to be going down — a colour has no agreed ranking and a relay has no in
  between, so those take the in time rather than have the console guess at one.
- **Keep a light where you have aimed it.** A *take* button in the patch panel's Home
  column stores what a fixture is putting out right now as where it rests. Which is
  how a house light's resting place actually gets set: aim it, look at it, keep it,
  rather than working out the number and typing it in about a light that is already
  right in front of you. The station reads its own output, so anything that can ask —
  the command line, a plugin — can do this without being able to read the rig.
- **Somewhere for a parameter to rest.** Every fixture parameter now has a *home
  value*: what it goes to when nothing is driving it. Its type says where that is —
  derived from what the device said about its own ports — and a fixture can override
  it, which is the only way a house light can say that it comes up rather than going
  dark. A new **Home** column in the patch panel sets and clears the override.
- **A sequence can be taken off.** The act the console did not have. OFF beside GO,
  and `sequence 1 off` on the command line: everything that sequence was driving and
  nothing else is still driving goes back to where it rests. A parameter another live
  sequence could drive, or one the operator is holding, is left alone.
- **Home, as a command.** `home` beside `full` and `out`, and a Home button in the
  programmer: the selection's parameters at their home values. The station works out
  what each one is, so a client that can set a level can ask for home without being
  able to read the rig — the same trick `at +10` plays.
- **How long letting go takes.** A show carries it, zero by default, so nothing
  changes until somebody asks for a fade. Show data rather than a console setting,
  because two stations fading one rig home over different times is not a preference
  but a disagreement the audience can watch; the console's own number decides what a
  *new* show starts with.

- **Settings, in the two flavours a console needs.** A new Settings panel, and the
  first thing in it: how many changes a show keeps for undo and the history panel.
  The show's own number travels in the showfile, so two consoles working one show
  agree about how far back Ctrl-Z goes. The console's number lives on the machine and
  decides what a *new* show starts with, which is what keeps them from disagreeing.
  Changes rather than presses — an undo is a change too and shares the room with the
  ones it takes back, so the panel says roughly how many presses that is.
- **One drag is one Ctrl-Z.** A fader dragged across its travel is a few hundred
  writes and, across a selection of twenty, a few thousand. It is one act, and undo
  now treats it as one: the client marks everything written between a pointer going
  down and coming up as a single gesture, and taking it back restores the value from
  before the drag started rather than one frame into it. A held arrow key counts as a
  drag too. Reversing a gesture writes one row per thing it touched rather than one
  per write, so taking back four hundred writes does not put four hundred rows in the
  log.
- **A drag costs the log one row per fixture, not one per frame.** A write inside a
  gesture replaces that gesture's earlier write to the same path instead of landing
  beside it, keeping the value it started from and taking the one it ended on — which
  is what both readers of the log want, since a peer catching up on a path needs only
  where it ended and undo needs the pair. Two seconds of dragging across a selection
  of twenty went from 2,400 rows to 20.
- **Undo and redo, per person.** Ctrl-Z takes back the last thing *you* changed,
  wherever you are: sign in at the desk and on the tablet as the same person and
  either takes back what the other did. There is no undo stack — an operation now
  carries who asked for it, what was there before and which operation it reverses, so
  undo is a query over the oplog. An undo therefore replicates to peers like any
  other write, and redo is undoing an undo. Everything editable can be taken back;
  a Go cannot, because an operator reaching for Ctrl-Z does not mean "move the
  lights". A new History panel shows what everyone has changed, colour-coded and
  named, with undos shown as themselves rather than tidied away.
- **Selection is a question about the rig, not a list of fixtures.** Select by
  type, by name, within a radius, inside a region, or inside a cone from a point —
  the spec's radial selection — and build it up by adding, narrowing and removing.
  The result is re-evaluated against the rig, so patching a sixth mover adds it to
  "every mover" without anyone touching the selection, which is what makes a
  selection survive a festival rig being rebuilt. Order it along an axis or
  outwards from a point, which is what an effect then spreads along. Clicking
  still works and combines with the rest; *Freeze* turns a question back into a
  plain list.
- **Effects.** A shape or a list of keyframes, applied across a selection and
  spread as a chase, from the centre out, in wings, in groups or at random. A new
  Effects panel builds one against a live waveform with a dot per fixture; the
  Programmer shows an amber chip for a parameter under an effect rather than a
  number, because the value beneath is only where it falls back to. Effects are
  held in the programmer or stored into a cue, and every station renders the same
  one from replicated state, so two consoles chase in step.
- **Speed masters.** A tempo several effects follow, tapped along with the band.
  Halve or double it, run or stop it, watch a beat dot. A tap writes the tempo and
  its anchor together, which is what makes a tempo change a step every station
  lands on rather than a drift each one accumulates.
- **A node is told the shape, not the samples.** An OpenHaunt port that says in
  `/info` which shapes it can trace is handed one descriptor and then left alone.
  On real firmware a half-hertz sine is one MQTT message and then twelve seconds
  of silence, where forty a second used to go out; a three second fade is one
  timed `set` rather than a hundred and twenty samples. The console publishes a
  retained `openhaunt/clock` so every node times its cycles against the same
  answer. A port that advertises nothing is driven exactly as before.
- **Cue timing has somewhere to be typed.** Fade in and out, follow mode, and
  per-capture fade, delay and curve — all honoured by the playback engine since it
  landed, and none of them reachable. A running cue now shows a strip of the fades
  and effects it is actually producing, which during a three second fade is not
  what the cue list says. Cues can be inserted between two others and dragged into
  a different order.
- **Panels that can change the show open read-only.** An Edit toggle in the tile
  chrome unlocks one and closing it locks it again, because a console is a tablet
  on a truss as often as it is a desk. Locked controls are absent rather than
  greyed out. Controls across the programmer are sized for a finger.
- **Fixture types can be edited properly.** Rename one, set each parameter's
  default value with the right control for its kind, and pick `Raw` or `Named`, so
  a light nobody has written a profile for can be patched without editing JSON.
- **Device detail, several stage plans, and positions by typing.** A device row
  opens to show its address, firmware, module and a port table saying what each
  port can trace for itself. A show can hold more than one plan and switch between
  them, with the 3D rig following, and a plan can be turned to match the room.
  Positions can be typed as x, trim and z with a resting direction. Flows can be
  renamed.
- **A Go says when it happened.** `goNext` and `goToCue` carry the time, so every
  station anchors a cue's fades and effects at the same millisecond instead of at
  whenever each of them processed the command.
- **The console says when a fixture has no way out.** A new LOCAL
  `output_coverage` path lists the fixtures no enabled output reaches — a DMX
  fixture on a universe nothing carries, or an adopted node with no OpenHaunt
  output — and the Outputs and Devices panels show each gap with a button that
  adds exactly the output it names. Deleting the OpenHaunt output no longer
  leaves adopted nodes silently undriven.
- **Selecting without the plan.** The Patch panel has a selector on every row
  and the Devices panel a *Select* button on every adopted node — click for
  one, shift-click to add — so a fixture can be programmed before it has been
  placed anywhere. Chips in the plan's *Not placed* tray can be dragged onto
  the plan to place them where they land.
- **A plugin can be told when what it remembered changed.** `store.subscribe` for
  plugin authors: a show-scoped store is show data, so an operator's undo can take a
  write back and the same plugin on the console next door can write the same key.
  Neither reaches a value a plugin is holding in memory, and one that never reads the
  key again never found out. Changes now arrive on the existing update callback as
  the store, the key and the new value. The plugin contract moves to 1.1 for it;
  plugins built against 1.0 are unaffected and keep running, which is what the
  version floor exists to guarantee.

### Changed

- **The Stations panel is now about who is here rather than what it costs.** Which
  machines are in the session, which is leading, where each is reached and what the
  link to it measures. Processor, memory and uptime moved to the new System panel,
  beside the frame costs and the browsers they are read against. Latency is in both.
  The built-in Setup layout is a grid rather than three columns, to hold the new tile.

- **BREAKING: where a fixture is, is now a transform.** A position was a point, or a
  point and a direction; it is now a position, a rotation and a scale, relative to
  whatever the fixture hangs off — which is what it takes to draw a rig where trusses
  turn and things hang off them. The scale is signed because a drawing mirrors things,
  and no rotation is a reflection.
- **BREAKING: a showfile from an older build is refused rather than migrated.** While
  the console is in development a showfile is not something anybody is carrying a
  season's work in, and a migration is a promise about every shape the data has ever
  had. A file from another build now says so plainly and names what it is — the
  generation it was written by, or the field every row needs a value for — instead of
  being read hopefully and losing something quietly. Start a fresh show.
- **BREAKING: a hand-made fixture type's channels follow its parameter list.** Where a
  parameter sits belongs to a *mode*, so the Fixture Types panel reads the channel
  number back rather than letting one be typed; reordering the parameters is how it
  changes. Types imported from a file are unaffected — their modes say where
  everything goes.
- **BREAKING: Go at the last cue stays on the last cue.** It used to leave the
  sequence with no active cue at all, which meant "the operator ran out of cues" and
  "the operator turned it off" were the same state — and the console could not tell
  them apart well enough to know what to put back. Running out of cues now holds what
  is showing, and OFF is what takes a sequence off. A show that relied on Go emptying
  a sequence needs an Off where that Go was.
- **Clearing the programmer lands on where a parameter rests**, not on a hardcoded
  zero. On a dimmer that is the same thing; on a house light that comes up, or a
  mover whose tilt sits centred, it is not.

### Removed

- **Three fields nothing ever read.** `Show.is_running`, `Show.active_sequence`
  and `Fixture.active_preset` are gone, along with the Show panel's
  Running/Stopped button, which toggled a flag no code consulted. All three were
  SYNCED, so they had no SQL column and their removal needs no migration; a
  showfile or a peer that still names one loads fine.

### Fixed

- **A colour wheel's black slot no longer loses a fixture type.** A real fixture file
  writes `nan` where that slot's colour goes; the console read it as a number, stored
  it, and could never read the row back. The type was there and gone at the same time.
- **An optional number in the show no longer comes back empty.** A field like a
  fixture's unit number was written to the showfile and read back as nothing, because
  the column's type and the way the value was stored disagreed.
- **A light hanging straight down no longer aims 45° off.** Asking which way a fixture
  faced when it faced straight down gave an answer 180° out, and everything computed
  from it — every beam in the rig view and on the plan — was wrong by an amount that
  looked plausible.
- **A console that gave up fetching a plugin asks again when one arrives.** A station
  that could find nobody with a plugin's bundle stayed that way until somebody edited
  the show — so the console holding it walking in five minutes late changed nothing,
  and the operator's only move was to poke the roster until the console tried again.
  A station joining the session now re-drives exactly the fetches that failed. Only
  somewhere genuinely new to ask counts: a console leaving, or one already known
  saying hello, asks for nothing again.
- **An undo reaching a peer arrived as a fresh change.** A station sent the author of
  a write and the value it replaced, but not which operation it reversed, so an undo
  landed in the other station's log looking like an edit — and the next Ctrl-Z there
  took back the wrong thing. Everything undo needs now travels together.
- **A tick was quadratic in the size of the rig.** `ShowView` scanned the fixture
  list for every lookup rather than indexing it, so the cost of a tick grew with
  the square of the rig. It went unnoticed while a settled show stopped ticking
  altogether; an effect never lets it settle. A thousand fixtures under one effect
  went from 29% of the tick budget to 16%.

- **Unpatching the last fixture reaches the output plugins.** The engine sent
  them nothing for an empty show, so whatever they remembered about the last
  fixture — including that nothing reached it — outlived it. One empty patch now
  follows the last fixture out.
- **Adopted OpenHaunt nodes are actually driven.** The plugin that sends a
  node's ports only runs where an `outputs` row of kind OpenHaunt says so, and
  nothing created one: values moved in the console and never left it. Starting
  to drive nodes now adds that output for the station, once, unless one already
  covers it.
- **A node that reboots gets its values again.** The OpenHaunt output remembered
  what it last sent and would not repeat it, while the node had come back at its
  defaults. A node seen going offline and back is sent every port afresh.

## 0.1.0

### Changed

- **OpenHaunt nodes describe their own ports.** `GET /api/v1/info` now carries a
  `ports` list in E1.73 UDR's vocabulary, and the console builds a fixture type
  from it at adoption. The module table is gone: a node newer than the console, or
  anybody else's module, adopts on its own say-so, and a node that describes
  nothing is refused rather than guessed at. A port whose `class` this console has
  no word for becomes a named parameter.
- **Output payloads follow the port's data type**, not the module. A number port
  takes `{ "value": … }`; `{ "brightness": … }` is retired.
- **`openhaunt-sim` is now `openhaunt-node-sim`**, and `openhaunt-sim-gui` is
  `openhaunt-node-sim-gui`.

### Added

- **A simulated node is a config file.** `openhaunt-node-sim` takes `--config`, and
  its window edits the running node: identity, module descriptor, and every port —
  access, data type, unit, range and class. Applying stops the node and starts a
  new one in its place without the window closing, so a module nobody has built is
  something to try rather than something to write. Presets for the catalogue,
  worked examples in `tools/openhaunt-node-sim/configs/`, and `--write-config` to
  get a file to start editing.

## 0.0.1

The first release, and a deliberately small number: this one is here to prove the
build and the release path rather than to be depended on.

A distributed lighting console — a show engine that several stations share, a
tiled web workspace to run it from, and the output and device layers that reach a
rig.

### Added

- **The show engine.** Cues, sequences, playback with fades and follow-ons, and a
  programmer buffer that outranks playback until it is cleared or stored.
- **Peer sync.** Stations find each other over mDNS, converge from an oplog,
  elect a leader, and survive losing one.
- **Output.** Art-Net, sACN and OpenHaunt nodes, several at once, configured from
  the show rather than from the command line.
- **Devices and flows.** OpenHaunt I/O nodes are discovered, adopted as fixtures,
  and wired to cues through a node graph that shows its own state.
- **The stage.** A calibrated ground plan and the same rig in 3D, with pan and
  tilt puppeteered by grabbing a ring, an arc, or the beam spot on the floor.
- **A tiled workspace.** Panels in a tree of splits and tab groups, with layouts
  saved into the show.
- **Desktop apps.** `pult-gui` runs a console and its server in one window;
  `openhaunt-node-sim-gui` is a window onto a simulated node.
- **One artifact per product.** The frontend is built into the server binary, so
  a station serves its own console — and any tablet on the network gets the same
  one.
