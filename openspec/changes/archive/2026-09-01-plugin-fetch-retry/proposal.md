## Why

Written after the fact, which is the honest thing to say about it. Roadmap task 40
went in as a flake fix, and one of the four flakes turned out to be the console
giving a wrong answer that only a busy machine made visible. Fixing it changed
behaviour a station's operator can see, and `openspec/specs/plugins/distribution/`
does not describe what the console now does.

Specifically, the spec's *No peer has the bundle* scenario says a station that
cannot get a bundle "reports that its bundle is missing". The code now says one of
two different things depending on **why**, and it asks again before saying either.
A spec that is behind the code is worse than no spec, so this change exists to
bring it level.

## What Changes

Nothing in the code. All of it shipped in `7a51b0f` and `c05db14`; this is the
spec delta those commits should have carried.

- **The two ways of not getting a bundle are told apart.** "No station in this
  session has it" sends an operator to install it somewhere. "I could not reach
  two stations" sends them to the network. Folding both into "missing" sent people
  to the wrong place.
- **A station asks again when somebody could not be reached.** Bounded, and only
  for that case — a peer that answered "no" will go on answering no. Before, one
  refused connection left a plugin permanently failed, because nothing re-drives a
  fetch until the roster changes.
- **A station asks the other stations, not itself.** Every station publishes its
  own row into `stations`, and the fetch was reading that list unfiltered.

## Non-goals

- **No retry forever.** A bounded number of attempts, then a reported failure. A
  console that silently kept trying would look like one that was working.
- **No retry when a peer answered.** Asking a station that said no to say no again
  is a console talking to itself.
- **Nothing about how a failed fetch is re-driven later.** Today the roster
  changing is what starts one; whether an operator should be able to say "try
  again" from the panel is a separate question this does not answer.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `plugins/distribution`: what a station reports when it cannot get a bundle now
  distinguishes "nobody has it" from "nobody could be reached", and a station asks
  again, a bounded number of times, in the second case only.

## Impact

- **Specs only.** `openspec/specs/plugins/distribution/spec.md` gains a
  requirement and replaces one scenario with two.
- The code, tests and roadmap entry are already in `main`:
  `crates/pult-backend/src/infra/assets.rs` (`Fetched`, one `peer_addresses`),
  `crates/pult-backend/src/infra/plugins/mod.rs` (`ASK_PEERS_TIMES`), and the
  tests in `tests/roster.rs` and `src/engine/tests.rs`.
