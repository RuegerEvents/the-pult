# plugins/distribution Specification

## Purpose
How a plugin gets into a show and onto every station running it: what the show
records, how bundle bytes reach a station that lacks them, and what a station
runs once it has them.

## Requirements

### Requirement: A show records the plugins it needs

The show SHALL hold a persisted roster of plugin packages. Each entry SHALL
carry the plugin id, its display name, its version, the plugin API version it
was built against, the sha256 of its bundle, whether it is enabled, and its
show-level configuration. The roster SHALL replicate to peer stations and SHALL
survive a reload of the showfile.

A plugin id SHALL appear at most once in the roster.

#### Scenario: A roster entry reaches every station in a session

- **WHEN** a plugin is installed on one station of a joined session
- **THEN** every other station's roster contains that entry
- **AND** the entry is still present after any station reopens the showfile

#### Scenario: Installing a plugin id the roster already holds

- **WHEN** a bundle is installed whose plugin id is already in the roster
- **THEN** the existing entry is replaced rather than duplicated
- **AND** stations running the previous bundle stop it and start the new one

### Requirement: A bundle is content-addressed and fetched on demand

A plugin bundle SHALL be a single archive containing the plugin's manifest, its
component, and any assets it serves. It SHALL be stored addressed by the sha256
of its own bytes.

A station that holds a roster entry whose bundle it does not have SHALL fetch
those bytes from a peer station and SHALL verify the sha256 of what it receives
before storing or running it. Bytes that do not hash to the requested digest
SHALL be discarded.

#### Scenario: A station that joins late acquires the bundle

- **WHEN** a station joins a session whose show carries a plugin it has never had
- **THEN** it fetches the bundle from a peer and runs the plugin
- **AND** no operator action on that station is required

#### Scenario: A peer answers with the wrong bytes

- **WHEN** a peer's response does not hash to the requested digest
- **THEN** the response is discarded and the plugin is not run

#### Scenario: No peer has the bundle

- **WHEN** no reachable station holds the bundle for a roster entry
- **THEN** that plugin reports that its bundle is missing, naming the digest
- **AND** every other plugin in the roster still loads

### Requirement: A station runs what the show carries

A station SHALL reconcile the plugins it runs against the roster whenever the
roster changes, without restarting. Adding an enabled entry SHALL start that
plugin; removing an entry, or disabling it, SHALL stop it; changing an entry's
bundle digest SHALL stop the running plugin and start the new bundle.

A station SHALL run a carried plugin without any per-station approval step.

#### Scenario: A plugin is removed while the show is up

- **WHEN** an operator removes a plugin from the roster during a show
- **THEN** every station stops that plugin
- **AND** saved layouts referencing its panels open with those panels absent

#### Scenario: A plugin is disabled rather than removed

- **WHEN** a roster entry is disabled
- **THEN** every station stops that plugin but the entry and its configuration remain
- **AND** re-enabling it starts the plugin again with that configuration

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

### Requirement: A station refuses a bundle it cannot run, and says why

A station SHALL refuse to run a bundle whose manifest declares a plugin API
version this station does not implement, and SHALL report the version the bundle
asked for and the version the station speaks. Such a refusal SHALL NOT prevent
the showfile from opening, nor prevent other plugins in the roster from running.

A bundle whose archive cannot be read, whose manifest is invalid, or whose
declared component is absent SHALL be refused on the same terms.

#### Scenario: A show authored on a newer console

- **WHEN** a station opens a show carrying a plugin built against a plugin API it does not implement
- **THEN** the show opens, that plugin reports the version mismatch, and the rest of the roster runs

### Requirement: A local plugin directory overrides the show

Where a station is configured with plugin directories, a plugin found on disk
SHALL take precedence over a roster entry with the same plugin id on that
station only. The station SHALL report that the override is in effect and which
version it is running.

A plugin loaded from a directory SHALL NOT be added to the roster, and SHALL
continue to reload when its files change.

#### Scenario: Developing a plugin on a station joined to a session

- **WHEN** a station has a plugin directory holding the same plugin id the show carries
- **THEN** that station runs the directory copy and reports the override
- **AND** every other station in the session runs the show's copy
- **AND** editing the files on disk reloads the plugin on that station alone

### Requirement: Installing a plugin is a single upload

An operator SHALL be able to install a plugin by supplying its bundle to a
station, and SHALL be able to remove one from the show. Both actions SHALL be
available from the console's own interface and SHALL be protected the way other
show-changing controls are.

A bundle that is not a readable archive containing a valid manifest SHALL be
rejected at upload with the reason, and SHALL NOT create a roster entry.

#### Scenario: Installing from one console equips the rig

- **WHEN** an operator uploads a bundle on one console
- **THEN** the plugin is running on every station in the session
- **AND** it is running again after every station restarts

#### Scenario: Uploading something that is not a plugin

- **WHEN** the uploaded file is not a valid bundle
- **THEN** the upload is rejected with the reason and the roster is unchanged

### Requirement: What a show asks for is legible before it runs

A station SHALL make each carried plugin's declared permissions — its data
access, whether it may invoke commands, the hosts it may reach over the network,
and the environment variable names passed through to it — readable by an
operator without opening the bundle.

#### Scenario: Reading what a show's plugins may do

- **WHEN** an operator inspects a plugin in the console's interface
- **THEN** its declared data access, command access, network hosts and environment names are shown

### Requirement: A plugin declares when it is relevant

A manifest MAY declare whether the plugin is relevant while a show is being
built, while it is being run, or both, defaulting to both. A station SHALL record
this and SHALL group plugins by it where it presents them. It SHALL NOT affect
whether a plugin loads.

#### Scenario: A setup-only plugin still loads during a show

- **WHEN** a plugin declares itself relevant only while a show is being built
- **THEN** it is grouped as such in the interface but runs exactly as any other plugin does

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
