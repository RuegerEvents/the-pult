# the-pult

A lighting console that is several consoles.

Every station runs the whole show — engine, playback, output, and its own copy of
the state — and they keep each other in step over the network. There is no server
in the middle to lose. The interface is a web app the station serves itself, so a
console is a window onto a station and so is the tablet at the other end of the
rig.

> Early. The engine, sync, playback, output, devices and the programmer work.
> Effects, phasers, timecode and the plugin runtime do not exist yet. See
> [docs/ROADMAP.md](docs/ROADMAP.md) for what is built and what is honestly
> missing, and [docs/SPEC.md](docs/SPEC.md) for what it is aiming at.

## What is here

| | |
|---|---|
| **`pult-backend`** | A station, as a server. Serves the console on `:7700`. |
| **`pult-gui`** | The same station, as a desktop app. |
| **`openhaunt-sim`** | The node side of the [OpenHaunt](https://github.com/OpenHaunt/node) I/O protocol, in software — there is no firmware yet. |
| **`openhaunt-sim-gui`** | A window onto one of those, with buttons for its inputs. |

Each is one file. The console's frontend is built into the binaries that serve
it, so there is nothing to deploy beside them.

## Installing a build

Nothing here is signed yet, so both desktop platforms will object the first time.

**macOS.** Right-click the app in Finder and choose *Open*, then *Open* again in
the dialog. If macOS instead says the app is **damaged and can't be opened**, that
is what it says about an app it cannot check rather than a broken download:

```
xattr -dr com.apple.quarantine /Applications/the-pult.app
```

**Windows.** SmartScreen offers *More info* → *Run anyway*.

The plain server and simulator binaries are not affected — only the desktop apps
go through Gatekeeper and SmartScreen.

## Running it

Grab a build from [Releases](../../releases), or:

```
cargo run -p pult-codegen -- generate     # TypeScript types from the schema
npm --prefix frontend ci
npm --prefix frontend run build
cargo run -p pult-backend
```

Then open `http://localhost:7700`, or that machine's address from anything else
on the network.

For the desktop app, `cargo run -p pult-gui`. It starts a station and opens a
window onto it — and keeps serving, so a tablet can join the same console.

Or all of it at once, with a seeded show and two simulated nodes:

```
scripts/demo.sh              # a fresh show with something to look at
scripts/demo.sh --two        # a second station, joined to the first's session
scripts/demo.sh --help       # the other options
```

That works in `.demo/`, which is gitignored, so it never touches a real showfile.

## Working on it

```
cargo test                                # the workspace, minus the desktop shells
npm --prefix frontend run dev             # Vite, proxying /ws to :7700
npm --prefix frontend test
npm --prefix frontend run check
```

Both the Rust build and `svelte-check` are kept at zero warnings.

[CLAUDE.md](CLAUDE.md) has the architecture, the lifecycle rules, and the one
thing to remember: **`pult-schema` is the single source of truth**, and adding an
entity type should need no edit anywhere else.

## Licence

MIT. See [LICENSE](LICENSE).
