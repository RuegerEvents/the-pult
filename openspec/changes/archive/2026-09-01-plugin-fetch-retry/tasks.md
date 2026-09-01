## 1. Already shipped

Written after the code, so these record what to check rather than what to build.
Everything here is in `main` as `7a51b0f` and `c05db14`; the boxes are ticked
against the commits, not against work still to do.

- [x] 1.1 `assets::Fetched` distinguishes `Got`, `NobodyHasIt` and `Unreachable(n)`,
      counting only requests that produced no answer. Verified by
      `a_station_that_could_not_be_reached_is_not_a_station_without_it`, which holds
      a peer answering wrongly apart from a peer that could not be asked.
- [x] 1.2 `begin_fetch` repeats only an unreachable ask, `ASK_PEERS_TIMES` times,
      backing off 250ms/500ms/1s, and does not sleep after the last attempt.
      Verified by `a_peer_that_did_not_answer_is_asked_again`, which was confirmed to
      fail with the constant set to 1 — a test that passes either way would pin
      nothing.
- [x] 1.3 A `NobodyHasIt` outcome and a local storage error both stop at once.
      Verified by `a_bundle_nobody_has_reads_as_fetching_and_then_says_so` and by
      the reason text asserted in `a_peer_answering_with_the_wrong_bytes_gets_nowhere`.
- [x] 1.4 One `peer_addresses`, in `assets.rs`, excluding the asking station and any
      station with no published address; both callers use it. Verified by
      `a_station_does_not_ask_itself` and `a_station_with_no_address_is_not_asked`.
- [x] 1.5 The failure reason names which of the two happened, and the digest.
      Verified by the assertions in the two roster tests above.

## 2. Bringing the spec level

- [x] 2.1 Write the delta against `plugins/distribution`: one added requirement for
      who is asked and when the asking repeats, and the reporting requirement
      modified to require the two reasons be told apart. Verify
      `openspec validate plugin-fetch-retry --strict` passes.
- [x] 2.2 Archive, and verify `openspec validate --specs --strict` still passes with
      the delta folded into `openspec/specs/plugins/distribution/spec.md`.
- [x] 2.3 Verify the full gate is still clean — `cargo test`, `cd plugins && cargo
      test`, `cd frontend && npm test`, `npm run check`, `cargo build` — since
      archiving must not have touched code.
