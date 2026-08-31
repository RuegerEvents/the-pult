# A show always has somebody — tasks

Each group is meant to end in its own commit with `cargo test`, `npm test`,
`npm run check` and a zero-warning build passing.

Group 2 is the one that can be got wrong quietly: everything in group 1 is
visible the first time a show is opened, while a seed that clobbers a rename
looks fine on one console and only shows up on two.

## 1. The constant and the seed

- [x] 1.1 Add `User::DEFAULT_ID` (a fixed `const Uuid`) and the default name
      `"Operator"` to `crates/pult-schema/src/types/user.rs`, beside
      `USER_COLOURS`, with the reason for a constant rather than a derived id
      written down. Verify the crate builds and the existing user tests pass.
- [x] 1.2 Seed the default user at the end of `load_from_showfile`
      (`crates/pult-backend/src/engine/mod.rs`): if `self.state` has no user with
      `DEFAULT_ID`, create one named "Operator" with `colour_for(0)`, persisted,
      attributed to nobody. Verify with an engine test that loading an empty
      showfile yields exactly one user and that it is written to SQLite.
- [x] 1.3 Verify with a test that loading a showfile that already has the default
      user writes nothing — no second row, and no oplog entry. `create_entity`
      has no existence check, so this is the guard, not an optimisation.
- [x] 1.4 Verify with a test that the seed is not undoable and not in the history
      of what people did: it carries no `user_id`, so `is_undoable()` is false
      and `recent_by_people` does not return it.
- [x] 1.5 Verify with a test that a showfile written before this change — an
      empty `users` table with oplog rows already in it — gains the default user
      on open, and that its existing rows are untouched and stay non-undoable.

## 2. Two stations, one operator

- [x] 2.1 Verify with a two-station test that both stations loading the same show
      end with exactly one user, with the same id on both.
- [x] 2.2 Verify with a test that a station joining a session and applying a
      snapshot does not seed a second time, and ends holding the leader's user
      rather than one of its own.
- [x] 2.3 Verify with a test that a rename survives: the default user is renamed
      on one station, the other station reloads the show, and the chosen name is
      what both hold. This is the property 1.3 protects, asserted end to end.
- [x] 2.4 Verify with a test that deleting the default user and loading the show
      again recreates it, so a show cannot be left with no user.

## 3. A client is always somebody

- [x] 3.1 Mirror the id into `frontend/src/lib/users.ts` as `DEFAULT_USER_ID`,
      beside `USER_COLOURS`, with the same argument written down — the browser
      needs it before the first write, so a round trip would be a round trip to
      learn a constant. Verify with a Rust test that reads `users.ts` and asserts
      the two spellings of the constant agree.
- [x] 3.2 In `frontend/src/lib/stores/user.ts`, start as the default when
      `localStorage` holds no `pult.user`, rather than as `null`, and identify to
      the socket as it on connect. Verify with a vitest that a client with empty
      storage reports the default id, and that one with a stored id still reports
      the stored id.
- [x] 3.3 Track whether anybody has chosen — a `localStorage` flag beside
      `pult.user`, client state rather than show data. Verify with a vitest that
      it is false for a fresh browser, true after choosing a user, and true after
      deliberately choosing the default.
- [x] 3.4 Make Sign out fall back to the default instead of to nobody: clear the
      stored identity, then adopt the default. Verify with a vitest that the
      client is working as the default afterwards and that the stored identity is
      gone, so the next visit starts as the default rather than as who signed out.
- [x] 3.5 Verify with a backend test that a write from a client that never sent
      an explicit identify is still attributed and still undoable — the property
      the whole change exists for, asserted at the seam rather than in the store.

## 4. Saying so, and taking the apologies out

- [x] 4.1 In `UserBar.svelte`, show the operator's name in place of "Who are
      you?", and keep the `--live` bordered treatment while nobody has chosen —
      with the meaning changed to "this is everyone's undo history until you say
      who you are". Verify by opening a fresh show and reading the chip and its
      tooltip.
- [x] 4.2 Drop the nudge once somebody has chosen, including choosing the default
      on purpose. Verify the chip is ordinary after picking a user and stays that
      way across a reload.
- [x] 4.3 Delete the toast in `frontend/src/lib/stores/undo.ts` ("Say who you are
      first") and the empty state in `HistoryPanel.svelte` ("Say who you are in
      the top bar"), and the `disabled={!$userId}` on the undo and redo chips.
      Verify undo and redo are live on a fresh show and that `npm run check`
      stays at zero warnings.
- [x] 4.4 Verify by hand on a fresh show, end to end: open it, change a fixture's
      name, press Ctrl-Z, and see it come back — with nobody having been asked
      anything. This is the acceptance criterion for the change; the tests above
      are how it stays true.

## 5. Writing it down

- [x] 5.1 Assert in `scripts/demo-seed.mjs`'s output, or in a test beside it,
      that a seeded demo show has an operator — the backend provides it, so this
      is a check that the assumption holds rather than code to add.
- [x] 5.2 Add the roadmap entry: what was decided about a shared default versus
      one per station, why the id is a constant in two languages, and the
      concurrent-seed case that reverts a rename and is allowed to.
- [x] 5.3 Mark `default-user` done in `openspec/BACKLOG.md`, in the form the
      plugin entries use — pointing at the archived change and recording what the
      answers turned out to be.
