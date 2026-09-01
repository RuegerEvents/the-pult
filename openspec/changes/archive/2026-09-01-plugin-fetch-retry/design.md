## Context

See proposal.md — Why. This is written after the code, so it records what was
decided rather than proposing it.

The relevant shape: `assets::fetch_from_peers` walks a list of peer addresses and
asks each one over HTTP. `plugins::begin_fetch` calls it on a task of its own and
sends the outcome back to the manager, which turns it into a `PluginStatus`.
Nothing re-drives a fetch — `reconcile` starts one, and only a roster change runs
`reconcile` again.

## Goals / Non-Goals

**Goals**

- A station that could not be reached is not reported as a station without the
  bundle.
- A console that came up half a second before its peer does not end up with a
  plugin that will never run.

**Non-Goals**

- Re-driving a fetch later, from a timer or a panel button. Out of scope, and
  named in the proposal.

## Decisions

### Three outcomes, not two

`fetch_from_peers` answered `Option<Asset>`, which cannot say why the answer was
`None`. It now answers `Fetched::{Got, NobodyHasIt, Unreachable(n)}`, counting the
peers whose request produced no answer at all.

The counting distinction is the request erroring versus a peer answering something
that is not the bundle. A 404 and a body that hashes wrong are both *answers* — the
station said its piece — and they end the search. A refused connection, a timeout
or a connection closed before a response is not an answer.

### Only an unanswered ask is worth repeating

`ASK_PEERS_TIMES` attempts, backing off 250ms, 500ms, 1s. Repeated only for
`Unreachable`: a peer that answered "no" will answer "no" again, and asking it four
times is a console talking to itself.

An `Err` from `fetch_from_peers` is *this* station's disk failing to store what came
back, not anybody's network, so it stops too. That was got wrong first time — the
code retried it while the comment beside it said only unreachable peers were worth
asking twice.

*Alternative considered:* retrying on a longer schedule, or indefinitely with the
plugin left reading "fetching". Rejected: a console that silently keeps trying looks
like one that is working, and an operator at a get-in needs to be told.

### The station does not ask itself

Every station publishes its own row into `stations`, so "the other stations" has to
be said out loud. There were two `peer_addresses`, and only one said it. A station
asking itself already knows the answer — not having the bundle locally is what
started the fetch — so it spent a round trip to be told 404 by its own HTTP server,
and once the fetch retried, it spent four.

There is now one `peer_addresses`, in `assets.rs`, taking the asking station's id.
Both callers use it.

## Risks / Trade-offs

- **Longer before a failure is reported.** Four attempts against a black-holed
  address is four request timeouts plus 1.75s of backoff. → The plugin reads
  *fetching* throughout, which is the honest state; the spec requires that
  distinction and it was already there.
- **A retry that succeeds hides a real network fault** an operator might want to
  know about. → The station log carries every failed ask at debug; the status is
  about whether the plugin runs.
- **`Unreachable` counts peers, not attempts**, so "could not reach 2 stations"
  after four tries does not say it tried four times. → The count is what an
  operator can act on; the attempts are the console's business.

## Migration Plan

None. No stored data and no wire format changes; `Fetched` is internal to the
backend.

## Open Questions

- Whether a failed fetch should be re-drivable without a roster change — a panel
  button, or a station noticing a new peer appear. Named as a non-goal here, and
  the natural next question if this turns out not to be enough.
