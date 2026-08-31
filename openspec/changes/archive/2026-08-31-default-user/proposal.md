# A show always has somebody

## Why

Undo shipped in task 31 and does not work on a new show. `users` is a PERSISTED
collection that nothing ever seeds — a fresh showfile has none, and
`scripts/demo-seed.mjs` makes none either. The frontend's `userId` starts `null`,
"before anybody has said", so the first change an operator makes carries
`Authorship { user_id: None }`, and `Operation::is_undoable()` requires
`user_id.is_some()` (`crates/pult-schema/src/events/operation.rs:196`). That
write can never be taken back — not later, not once the operator says who they
are, not ever.

Both surfaces already apologise for it rather than fix it. The undo store raises
"Say who you are first — undo is per person"
(`frontend/src/lib/stores/undo.ts:39`), the History panel prints "Say who you are
in the top bar to take anything back", and the UserBar's chip is bordered in
`--live` with the tooltip "Nobody is signed in — changes cannot be taken back".
Three pieces of UI exist to describe a hole. The console shipped a feature whose
first-run state is off.

The fix is that there is no no-user. A show has an operator from the moment it
exists, so Ctrl-Z works before anybody has been asked to configure anything.

## What Changes

- **Every show gets a default user, created by the backend on load.** Not by the
  first browser to connect: a station runs headless, and plugins and station RPCs
  write too. Seeding where the show comes up is the only place that covers a
  console with no browser attached.
- **One default per show, not per station.** Its id is a fixed, well-known UUID,
  so two stations opening the same show write the same row rather than two rows
  holding two operators. Deriving it from the station would contradict the reason
  `User` exists at all: `crates/pult-schema/src/types/user.rs` argues identity is
  *chosen* rather than derived from the machine, because one person's desk and
  tablet are both them.
- **It is called "Operator", and it is an ordinary user row.** Renameable,
  recolourable, deletable, indistinguishable from one somebody typed in — a name
  nobody chose beats no attribution, and the moment somebody dislikes it they can
  change it and it stays changed.
- **Seeding is conditional and idempotent.** A station that finds the row already
  present writes nothing. This matters more than it looks: `create_entity`
  (`crates/pult-backend/src/engine/mod.rs:1292`) validates and inserts with no
  existence check, so a station joining a session and seeding unconditionally
  would replicate "Operator" over a rename another station had already made.
- **A browser with nothing in `localStorage` adopts the default and says so.**
  Undo works from the first change. The UserBar shows the operator by name and
  keeps a visible nudge to say who you actually are, because two people at two
  browsers sharing one undo history is a real cost and it belongs on screen
  rather than in a surprise.
- **Sign out falls back to the default instead of to nobody.** The end-of-session
  gesture UserBar.svelte's comment describes still works; what goes away is the
  state it currently lands in. `beUser(null)` stops being reachable from the UI.
- **Existing showfiles get one too**, on open, by the same conditional seed. Their
  already-written oplog rows carry `user_id: None` and stay un-undoable — that is
  history and cannot be rewritten — but everything from the upgrade forward can
  be taken back.
- **`Authorship::user_id` stays `Option<Uuid>`.** The engine's own writes — a fade
  advancing at 40 Hz, a station publishing its memory use — are genuinely
  nobody's, and an unattributed write is precisely how the plugin store's
  non-undoable default works (task 35). This change is about the client path never
  sending `None`, not about the type losing its `None`.

## Capabilities

### New Capabilities

- `users/identity`: that a show always has at least one user, who creates it and
  when, how it converges when two stations open the same show, what a client
  works as before anybody has chosen, and what identity a client can and cannot
  be in.

### Modified Capabilities

None. `plugins/datastores`, `plugins/distribution` and `plugins/configuration`
are untouched: a store write's undoability comes from whether the host attributes
it, and this change does not alter attribution, only guarantee that the client
path has somebody to attribute to.

## Non-goals

- **Not access control.** `user.rs` already says why there is no password:
  everyone on the network can change everything, which is the right default for a
  desk in a room where everyone is trusted. A default user does not weaken a
  boundary that was never there.
- **No rewriting history.** Operations already in an oplog with `user_id: None`
  stay non-undoable. Attributing them retroactively to the default would be
  inventing a claim about who did something.
- **No per-station identity.** Deliberately rejected above; a station that wants
  its own operator makes one, which is the existing "+ Somebody else" flow.
- **No user deletion policy.** Deleting the default is allowed and leaves a show
  where a fresh browser has nobody to adopt again — the seed is on load, so the
  next open restores it. Guarding the row against deletion would make it a
  special thing, and the point is that it is not.
- **No sessions, no login, no per-user preferences.** A user is still a name and
  a bucket to undo from.

## Impact

- `crates/pult-schema/src/types/user.rs`: the well-known default id and its name,
  as constants beside `USER_COLOURS`. No field changes, so no migration and no
  `ShowState` work — the collection already exists and already replicates.
- `crates/pult-backend/src/engine/mod.rs`: a conditional seed after
  `load_from_showfile`, in the same spirit as `seed_outputs_from_flags` in
  `lib.rs` — anything already configured wins.
- `crates/pult-backend/src/lib.rs`: nothing, if the seed lives inside the load
  the engine already performs. Named here because the ordering against
  `EngineCommand::LoadFromShowfile` and a later `ApplyStateSnapshot` is the part
  that has to be right.
- `frontend/src/lib/stores/user.ts`: adopting the default when `localStorage` has
  nothing, once the `users` collection has arrived; `beUser(null)` no longer
  reachable from the UI.
- `frontend/src/lib/components/UserBar.svelte`: the "Who are you?" chip becomes
  the operator's name with a nudge; Sign out becomes a fall-back to the default.
- `frontend/src/lib/stores/undo.ts` and `HistoryPanel.svelte`: the two "say who
  you are" messages describe a state that can no longer happen and go away.
- `scripts/demo-seed.mjs`: nothing to add — the backend seeds, so the demo show
  gets an operator for free. Worth asserting rather than assuming.
