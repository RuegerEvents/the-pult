#!/usr/bin/env bash
#
# Start everything at once: a backend, two simulated OpenHaunt nodes, and the
# frontend dev server. Ctrl-C stops all of it. With --two there is a second
# backend as well, a station of its own, following the first one's show.
#
# The show lives in .demo/ and is rebuilt from nothing on every run, so this never
# touches a real showfile. Pass --keep to carry the last run's show over.
#
#   scripts/demo.sh              a fresh show, seeded with something to look at
#   scripts/demo.sh --keep       carry on from where the last run left off
#   scripts/demo.sh --no-seed    a fresh show, empty
#   scripts/demo.sh --no-sims    no simulated devices
#   scripts/demo.sh --two        a second station, joined to the first's session
#
# Ports can be overridden: PORT, SYNC_PORT, BROKER_PORT — and PORT_2, SYNC_PORT_2,
# BROKER_PORT_2 for the second station.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)

PORT=${PORT:-7700}
SYNC_PORT=${SYNC_PORT:-7701}
# Not 1883: a demo should not fight a real broker that happens to be installed.
BROKER_PORT=${BROKER_PORT:-11883}
# The second station is a whole console, so it needs its own three ports and its
# own showfile — a follower keeps its own copy of the show.
PORT_2=${PORT_2:-7710}
SYNC_PORT_2=${SYNC_PORT_2:-7711}
BROKER_PORT_2=${BROKER_PORT_2:-11884}
DEMO_DIR="$ROOT/.demo"
SHOWFILE="$DEMO_DIR/demo.db"
SHOWFILE_2="$DEMO_DIR/demo-2.db"

KEEP=0
SEED=1
SIMS=1
TWO=0
for arg in "$@"; do
  case "$arg" in
    --keep) KEEP=1 ;;
    --no-seed) SEED=0 ;;
    --no-sims) SIMS=0 ;;
    --two) TWO=1 ;;
    # The comment block at the top of this file is the help text, so the two
    # cannot drift apart. Printed up to the first line that is not a comment.
    -h|--help) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done

# ── Stopping cleanly ──────────────────────────────────────────────────────────
#
# Only what this script started. `npm run dev` spawns vite as a child, so killing
# the tracked pid alone would leave a dev server holding its port; walking the
# tree is what makes Ctrl-C actually free the ports. Matching on process names
# would be shorter and would also kill somebody else's vite.

PIDS=()

kill_tree() {
  local pid=$1 child
  for child in $(pgrep -P "$pid" 2>/dev/null || true); do
    kill_tree "$child"
  done
  kill "$pid" 2>/dev/null || true
}

cleanup() {
  trap - EXIT INT TERM
  # Nothing started means nothing to stop — saying otherwise makes a failed
  # port check look like something crashed.
  if [ ${#PIDS[@]} -eq 0 ]; then
    return
  fi
  echo ""
  echo "stopping"
  local pid
  for pid in "${PIDS[@]}"; do
    kill_tree "$pid"
  done
  wait 2>/dev/null || true
  echo "stopped."
}
trap cleanup EXIT INT TERM

# ── Checks ────────────────────────────────────────────────────────────────────

port_free() {
  ! nc -z 127.0.0.1 "$1" >/dev/null 2>&1
}

require_port() {
  if ! port_free "$1"; then
    echo "port $1 is already in use ($2)." >&2
    echo "Set $3 to something else, or stop whatever is on it." >&2
    exit 1
  fi
}

require_port "$PORT" "the backend's WebSocket API" PORT
require_port "$SYNC_PORT" "peer sync" SYNC_PORT
require_port "$BROKER_PORT" "the MQTT broker" BROKER_PORT

if [ "$TWO" = 1 ]; then
  require_port "$PORT_2" "the second station's WebSocket API" PORT_2
  require_port "$SYNC_PORT_2" "the second station's peer sync" SYNC_PORT_2
  require_port "$BROKER_PORT_2" "the second station's MQTT broker" BROKER_PORT_2
fi

# ── The show ──────────────────────────────────────────────────────────────────

if [ "$KEEP" = 0 ]; then
  rm -rf "$DEMO_DIR"
fi

# ── Build ─────────────────────────────────────────────────────────────────────

mkdir -p "$DEMO_DIR"

echo "building"
if ! cargo build --workspace --quiet > "$DEMO_DIR/build.log" 2>&1; then
  echo "the build failed:" >&2
  cat "$DEMO_DIR/build.log" >&2
  exit 1
fi

if [ ! -d "$ROOT/frontend/node_modules" ]; then
  echo "installing frontend dependencies"
  if ! npm --prefix "$ROOT/frontend" install --silent > "$DEMO_DIR/npm-install.log" 2>&1; then
    echo "npm install failed:" >&2
    cat "$DEMO_DIR/npm-install.log" >&2
    exit 1
  fi
fi

# ── Backend ───────────────────────────────────────────────────────────────────
#
# Started before the nodes, so its mDNS browser is listening when they announce
# themselves. The other order works too, but only once a node re-announces.

# start_station <showfile> <port> <sync-port> <broker-port> <log>
start_station() {
  local showfile=$1 port=$2 sync_port=$3 broker_port=$4 log=$5
  "$ROOT/target/debug/pult-backend" \
    --showfile "$showfile" \
    --port "$port" \
    --sync-port "$sync_port" \
    --openhaunt-broker-port "$broker_port" \
    > "$log" 2>&1 &
  PIDS+=($!)

  for _ in $(seq 1 40); do
    port_free "$port" || break
    sleep 0.25
  done
  if port_free "$port"; then
    echo "the backend on port $port did not come up. Last words:" >&2
    tail -20 "$log" >&2
    exit 1
  fi
}

echo "starting the backend on port ${PORT}"
start_station "$SHOWFILE" "$PORT" "$SYNC_PORT" "$BROKER_PORT" "$DEMO_DIR/backend.log"

# ── Simulated devices ─────────────────────────────────────────────────────────

if [ "$SIMS" = 1 ]; then
  echo "starting two simulated OpenHaunt nodes"
  # --auto presses contact 0 every couple of seconds, so a flow wired to it
  # visibly fires without anyone having to hold a button.
  "$ROOT/target/debug/openhaunt-sim" --module input --serial 1a2b3c --port 8801 --auto 2500 \
    > "$DEMO_DIR/sim-input.log" 2>&1 &
  PIDS+=($!)
  "$ROOT/target/debug/openhaunt-sim" --module relay --serial 4d5e6f --port 8802 \
    > "$DEMO_DIR/sim-relay.log" 2>&1 &
  PIDS+=($!)
fi

# ── Something to look at ──────────────────────────────────────────────────────

if [ "$SEED" = 1 ] && [ "$KEEP" = 0 ]; then
  echo "seeding a show"
  if ! node "$ROOT/scripts/demo-seed.mjs" "$PORT"; then
    echo "  (seeding failed — the demo still runs, just empty)" >&2
  fi
fi

# ── The second station ────────────────────────────────────────────────────────
#
# Started after the show exists, so it has something to be handed when it joins.
# It is an ordinary console — its own showfile, its own identity, no idea it is
# sharing a machine — which is what makes the sync visible: edit on one, watch it
# land on the other.

if [ "$TWO" = 1 ]; then
  echo "starting a second station on port ${PORT_2}"
  start_station "$SHOWFILE_2" "$PORT_2" "$SYNC_PORT_2" "$BROKER_PORT_2" "$DEMO_DIR/backend-2.log"

  echo "pairing the two stations"
  if ! node "$ROOT/scripts/demo-session.mjs" "$PORT" "$PORT_2"; then
    echo "  (both stations still run — pair them from the Sessions panel)" >&2
  fi
fi

# ── Frontend ──────────────────────────────────────────────────────────────────

echo "starting the frontend"
npm --prefix "$ROOT/frontend" run dev > "$DEMO_DIR/frontend.log" 2>&1 &
PIDS+=($!)

# Vite picks another port when its usual one is taken, so the URL is read back
# out of its own output rather than assumed.
URL=""
for _ in $(seq 1 60); do
  URL=$(grep -oE 'http://localhost:[0-9]+' "$DEMO_DIR/frontend.log" 2>/dev/null | head -1 || true)
  [ -n "$URL" ] && break
  sleep 0.5
done

if [ -z "$URL" ]; then
  echo "the frontend did not report a URL. Last words:" >&2
  tail -20 "$DEMO_DIR/frontend.log" >&2
  exit 1
fi

# ── Ready ─────────────────────────────────────────────────────────────────────

cat <<EOF

  ──────────────────────────────────────────────────────────────
   $URL
  ──────────────────────────────────────────────────────────────

  backend    :$PORT (ws), :$SYNC_PORT (sync), :$BROKER_PORT (mqtt)
  showfile   .demo/demo.db
  logs       .demo/*.log
EOF

if [ "$TWO" = 1 ]; then
  # The frontend takes ?port=, so the same dev server is a window onto either
  # station. Two browser tabs, one console each.
  cat <<EOF

  second station
    $URL/?port=$PORT_2
    :$PORT_2 (ws), :$SYNC_PORT_2 (sync), :$BROKER_PORT_2 (mqtt)
    showfile .demo/demo-2.db

  Open both tabs side by side: an edit on one appears on the other, each shows up
  in the other's Stations panel, and a value moved in the programmer on one is
  held on both.
EOF
fi

if [ "$SIMS" = 1 ]; then
  cat <<'EOF'

  Panels are tiled: drag a tab to a tile's edge to split it, to its middle to
  stack it, and pick a layout from the menu beside the name in the top bar.

  Try, in order:

    1. Programming, the layout it opens on — click a moving head in the rig. The
       camera frames it, a pan ring and a tilt arc appear on the yoke and a disc
       where its beam lands. Drag the disc; pull Intensity up in the Programmer
       panel. Store, and the look is a cue. Clear, and playback has it back.
    2. Playback — Edit on that cue puts it back in the programmer with the cue
       taken. Change something, Update, and the cue changes.
    3. Devices — Adopt the Digital Inputs node. That is the moment it learns
       where the broker is; before that it publishes nothing.
    4. Flows — the seeded graphs are there. Press the button in Panic button and
       watch the Wait node count down before the cue moves. Then add a Watch
       node on the adopted node's Contact:0 and wire it up: the simulator
       presses that contact every 2.5s, so the cue starts stepping.
    5. Devices — Adopt the Mains Relay too, then set Switch:0 from Patch and
       watch .demo/sim-relay.log record the output.
    6. Plan — upload a ground plan (a PDF works; page one is used), click two
       points whose real distance apart you know to set the scale, then drag the
       fixtures onto it in Move mode.
    7. Outputs — add an Art-Net output and watch its frame rate appear.
    8. Stations — this console, its cpu and memory, and what it is sending.
EOF
fi

echo ""
echo "  Ctrl-C to stop everything."
echo ""

wait
