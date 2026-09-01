## ADDED Requirements

### Requirement: A station asks the other stations, and asks again when one could not be reached

A station fetching a bundle SHALL ask the other stations in the session and SHALL
NOT ask itself.

Where one or more stations could not be reached at all, the station SHALL try
again a bounded number of times before reporting failure. Where every station that
was asked answered, and none of them held the bundle, the station SHALL NOT try
again: an answer is an answer.

A station SHALL report a fetch as still fetching while it is trying, and SHALL
report a failure once it has stopped.

#### Scenario: A peer that did not answer is asked again

- **WHEN** a station asks a peer for a bundle and the request fails without an answer
- **AND** that peer answers normally shortly afterwards
- **THEN** the station gets the bundle and runs the plugin
- **AND** no operator action is required

#### Scenario: A peer that answered is not asked again

- **WHEN** every station that was asked answered, and none held the bundle
- **THEN** the station stops asking and reports the failure

#### Scenario: A station does not ask itself

- **WHEN** a station fetches a bundle it does not have
- **THEN** it does not make a request to its own HTTP API
- **AND** a station that has not published where it serves is not asked either

## MODIFIED Requirements

### Requirement: A plugin's state is visible while it is being acquired

A station SHALL report, for every plugin it knows about, whether it is fetching
its bundle, loading, running, or failed, and SHALL give the reason for a failure.
Fetching a bundle SHALL be reported as its own state, distinct from failure.

Where a fetch failed, the reason SHALL distinguish a bundle that no station holds
from stations that could not be reached. These call for different things of an
operator — one to install the bundle somewhere, the other to look at the network —
and reporting either as the other sends them to the wrong place.

#### Scenario: An operator watches a plugin arrive

- **WHEN** a station is fetching a bundle it lacks
- **THEN** that plugin reads as fetching rather than as failed or missing

#### Scenario: Nobody has it

- **WHEN** every station that was asked answered, and none held the bundle
- **THEN** the reason says no station in the session has it, naming the digest

#### Scenario: Nobody could be reached

- **WHEN** the stations that might hold the bundle could not be reached
- **THEN** the reason says how many could not be reached, naming the digest
- **AND** it does not say that no station has the bundle, which was not established
