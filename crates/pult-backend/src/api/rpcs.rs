//! The station RPCs: the calls that are not entity commands.
//!
//! An entity command deserializes a row, mutates it, and hands the engine
//! something to apply — it *changes* the show, and it writes an operation
//! saying so. What is here is the other kind: calls that go to the manager
//! owning some LOCAL state, and calls that answer a question about the show
//! without changing it. Both would be lies as commands, the first because
//! there is no entity, the second because a read is not an edit and has no
//! business in anyone's undo stack.
//!
//! One table and one dispatcher, so the WebSocket handler and the plugin
//! runtime answer the same calls the same way, and introspection can list them
//! without a second place going stale. A test below holds the table and the
//! match to each other.

use pult_schema::{
    path::PathSegment,
    types::{
        evaluate,
        fixture::{
            driving, parameter_key, FixtureType, HeldByProgrammer,
            ParameterKind,
        },
        scene::SceneObject,
        Fixture, Group,
    },
};
use serde_json::Value;

use crate::{
    engine::EngineHandle,
    // `short_id` is the client registry's, so the eight characters on a browser's log
    // line and the eight on its row in the System panel are the same eight rather
    // than two implementations that agree today.
    infra::{clients::short_id, devices::DeviceHandle, session::SessionHandle},
};

/// What one RPC looks like from outside: enough for a command line to offer
/// it, complete it, and explain it.
pub struct LocalRpcMeta {
    pub method: &'static str,
    /// JSON array of `{ "name", "type", "optional" }`, mirroring the shape
    /// `CommandRegistration::args_schema` uses. `[]` for no arguments.
    pub args_schema: &'static str,
    pub doc: &'static str,
}

pub const LOCAL_RPCS: &[LocalRpcMeta] = &[
    LocalRpcMeta {
        method: "device.adopt",
        args_schema: r#"[{"name":"serial","type":"string","optional":false}]"#,
        doc: "Adopt a discovered OpenHaunt device; answers with the new fixture's id.",
    },
    LocalRpcMeta {
        method: "device.identify",
        args_schema: r#"[{"name":"serial","type":"string","optional":false}]"#,
        doc: "Ask a discovered device to identify itself, e.g. by blinking.",
    },
    LocalRpcMeta {
        method: "device.forget",
        args_schema: r#"[{"name":"serial","type":"string","optional":false}]"#,
        doc: "Forget an adopted device and release its fixture.",
    },
    LocalRpcMeta {
        method: "device.strip",
        args_schema: r#"[{"name":"serial","type":"string","optional":false},{"name":"head","type":"number","optional":false},{"name":"mode","type":"'effect' | 'sacn'","optional":true},{"name":"count","type":"number","optional":true},{"name":"order","type":"string","optional":true},{"name":"universe","type":"number","optional":true},{"name":"address","type":"number","optional":true},{"name":"holdMs","type":"number","optional":true},{"name":"pitchM","type":"number","optional":true}]"#,
        doc: "Patch a node's pixel strip in a mode: `effect`, where the node draws it, or `sacn`, a fixture per pixel the console draws. Anything left out stays. One Ctrl-Z; answers the strip's fixture id.",
    },
    LocalRpcMeta {
        method: "session.join",
        args_schema: r#"[{"name":"sessionId","type":"string","optional":false}]"#,
        doc: "Join a discovered session as a follower.",
    },
    LocalRpcMeta {
        method: "session.joinAt",
        args_schema: r#"[{"name":"address","type":"string","optional":false}]"#,
        doc: "Join the station at a sync address (`host:port`) as a follower, without having discovered it — for a network where mDNS does not reach.",
    },
    LocalRpcMeta {
        method: "session.invite",
        args_schema: r#"[{"name":"nodeId","type":"string","optional":true},{"name":"address","type":"string","optional":true}]"#,
        doc: "Ask a station to join this one's session, starting a session first if this station has none: one on the network by `nodeId`, or any by its HTTP `address` (`host:port`). The station invited takes this show: its own open show is replaced.",
    },
    LocalRpcMeta {
        method: "session.watchNearby",
        args_schema: r#"[{"name":"on","type":"boolean","optional":false}]"#,
        doc: "This page is showing the stations on the network, or has stopped: they are looked for only while some page is. The list is `session.nearby`.",
    },
    LocalRpcMeta {
        method: "session.leave",
        args_schema: "[]",
        doc: "Leave the session this station is following.",
    },
    LocalRpcMeta {
        method: "session.create",
        args_schema: r#"[{"name":"showName","type":"string","optional":false},{"name":"showId","type":"string","optional":false}]"#,
        doc: "Start a session around this station's show; answers with the session id.",
    },
    // Named for what it answers about rather than for `groups`, deliberately: an
    // RPC's prefix is a word the command line's grammar can no longer use for a
    // collection, and `group 3` has to go on meaning the fixtures in group 3.
    LocalRpcMeta {
        method: "selection.resolve",
        args_schema: r#"[{"name":"groupId","type":"string","optional":false}]"#,
        doc: "The fixtures a saved group picks out of the rig right now, in its order.",
    },
    // Where a value set on a selection lands, cut to cells or not: the schema's rule,
    // asked rather than copied, so the command line puts `at 50` on exactly the keys
    // the Programmer would. A read like the one above.
    LocalRpcMeta {
        method: "selection.targets",
        args_schema: r#"[{"name":"fixtureIds","type":"array","optional":false},{"name":"cells","type":"array","optional":true},{"name":"parameterKind","type":"object","optional":false}]"#,
        doc: "The parameters a value of one kind lands on, over these fixtures and these cells of them.",
    },
    // The paperwork's arithmetic. A read like the two above, and on the station rather
    // than in the browser for one reason: the browser is the only thing that can *draw*
    // a sheet, and if the tables were drawn there too then what the rig weighs would be
    // a number no plugin and no command line could ever ask for.
    LocalRpcMeta {
        method: "paperwork.tables",
        args_schema: r#"[{"name":"kind","type":"string","optional":false},{"name":"grouping","type":"string","optional":true},{"name":"rows","type":"object","optional":true},{"name":"layers","type":"array","optional":true}]"#,
        doc: "A paperwork table over the rig: Patch, Loading, Power or Counts, grouped and subtotaled.",
    },
    // What a rig would look like in a given state, for a rendered viewport on a sheet.
    // A read like the one above, and it must stay one: rendering a beauty shot must not
    // *take* the cues it draws, or a document would change the show it documents — with
    // the lamps on, visibly, in the room.
    LocalRpcMeta {
        method: "paperwork.cueValues",
        args_schema: r#"[{"name":"cues","type":"array","optional":false}]"#,
        doc: "What every parameter would be doing with these cues running, each a given time in.",
    },
    // A read, so deliberately not a command: asking what a light is doing must not
    // write history. It exists because nothing stores the answer any more — the
    // console keeps what is *driving* each parameter and evaluates on demand — and a
    // caller that cannot hold the whole stack still has to be able to ask.
    // A read: the types a console knows without a file, and the color mixes the type
    // editor offers. Nothing is written until one is patched.
    LocalRpcMeta {
        method: "fixtureTypes.library",
        args_schema: r#"[]"#,
        doc: "The generic fixture types and the color emitter presets this console ships with.",
    },
    LocalRpcMeta {
        method: "parameter.value",
        args_schema: r#"[{"name":"fixtureId","type":"string","optional":false},{"name":"parameterKind","type":"object","optional":true}]"#,
        doc: "What a fixture's parameters are putting out right now; one when named, all of them when not.",
    },
    // MVR-xchange. RPCs rather than entity commands for the reason `show.open` is one:
    // which group a console is in and what it hands another program are nobody's to
    // undo, and there is no row to attach them to. What an *applied* commit writes is
    // undoable, and is the ordinary import doing it — attributed to whoever asked.
    LocalRpcMeta {
        method: "xchange.commit",
        args_schema: r#"[{"name":"comment","type":"string","optional":false}]"#,
        doc: "Write the rig as an MVR, keep it, and tell the exchange group about it.",
    },
    LocalRpcMeta {
        method: "xchange.apply",
        args_schema: r#"[{"name":"fileUuid","type":"string","optional":false},{"name":"confirm","type":"boolean","optional":true}]"#,
        doc: "Fetch a commit and import it. Answers what is running instead, unless confirmed.",
    },
    LocalRpcMeta {
        method: "xchange.followHost",
        args_schema: r#"[{"name":"follow","type":"boolean","optional":false}]"#,
        doc: "Answer a station that asked this exchange group to move to another host.",
    },
    // The console's own log. Reads and diagnostics, so RPCs rather than commands
    // for the reason `parameter.value` is one: none of this is anybody's to undo,
    // and a log that wrote history every time somebody looked at it would be a
    // strange thing indeed.
    // The sound. Three calls: what this machine has, what is in a song, and whether
    // this station has the big model. All three reach a device, a decoder or the
    // internet, and none of them changes the show — which is exactly what an RPC is
    // for and why none of them is an entity command.
    LocalRpcMeta {
        method: "audio.devices",
        args_schema: "[]",
        doc: "What this station can play a timeline through and listen for timecode on.",
    },
    // A show variable's value. RPCs rather than a field write or an entity command,
    // because both of those carry the caller's user id, and a phone resetting a maze
    // area would then be a row in the History panel and something for Ctrl-Z. A flow
    // writes the same way, so who changed a count is never the question undo answers.
    LocalRpcMeta {
        method: "variable.set",
        args_schema: r#"[{"name":"variableId","type":"string","optional":false},{"name":"value","type":"number","optional":false}]"#,
        doc: "Set a show variable, clamped to its bounds. Not undoable and not in History.",
    },
    LocalRpcMeta {
        method: "variable.add",
        args_schema: r#"[{"name":"variableId","type":"string","optional":false},{"name":"by","type":"number","optional":false}]"#,
        doc: "Add to a show variable (negative to subtract), clamped to its bounds.",
    },
    LocalRpcMeta {
        method: "variable.toggle",
        args_schema: r#"[{"name":"variableId","type":"string","optional":false}]"#,
        doc: "Zero becomes one and anything else zero.",
    },
    LocalRpcMeta {
        method: "variable.reset",
        args_schema: r#"[{"name":"variableId","type":"string","optional":false}]"#,
        doc: "Put a show variable back at its initial value.",
    },
    LocalRpcMeta {
        method: "timeline.detect",
        args_schema: r#"[{"name":"timelineId","type":"string","optional":false},{"name":"clipId","type":"string","optional":true}]"#,
        doc: "Find the beats and downbeats in one of a timeline's clips (the first unless `clipId` names another), in timeline positions; answers them and writes them to `detected`, which an operator then confirms into `grid`.",
    },
    LocalRpcMeta {
        method: "timeline.model",
        args_schema: r#"[{"name":"full","type":"boolean","optional":true}]"#,
        doc: "Whether this station has the full-accuracy beat model, and with `full` fetch it into the config directory.",
    },
    LocalRpcMeta {
        method: "log.tail",
        args_schema: r#"[{"name":"limit","type":"number","optional":true},{"name":"level","type":"string","optional":true}]"#,
        doc: "The recent log, oldest first, with this station's levels and where the file is.",
    },
    LocalRpcMeta {
        method: "log.setLevel",
        args_schema: r#"[{"name":"level","type":"string","optional":true},{"name":"peerLevel","type":"string","optional":true}]"#,
        doc: "Set what this station keeps in its log, and what it publishes to peers.",
    },
    LocalRpcMeta {
        method: "log.watch",
        args_schema: r#"[{"name":"nodeId","type":"string","optional":false},{"name":"level","type":"string","optional":false}]"#,
        doc: "Ask a peer to publish its log at this level while this client is watching.",
    },
    LocalRpcMeta {
        method: "log.unwatch",
        args_schema: r#"[{"name":"nodeId","type":"string","optional":false}]"#,
        doc: "Stop watching a peer's log; the peer drops back when nobody else is.",
    },
    LocalRpcMeta {
        method: "log.report",
        args_schema: r#"[{"name":"level","type":"string","optional":false},{"name":"message","type":"string","optional":false},{"name":"count","type":"number","optional":true}]"#,
        doc: "Report something that went wrong in a browser into the station's log.",
    },
    // The other half of what a browser can say about itself: numbers rather than
    // sentences. A page is a console evaluating a rig at frame rate against a clock
    // it had to estimate, and nothing else on this station can measure any of that.
    //
    // One object rather than a dozen named arguments, because nobody types this — it
    // is a page describing itself every couple of seconds, not a verb for the command
    // line — and a shape that grows a field should not grow a signature.
    // What is actually on the wire. Asked for rather than published, for the reason
    // `log.watch` is: a universe image is 512 bytes forty times a second, and a
    // station broadcasting that to browsers nobody has looked at — or across the
    // link carrying the show — would be paying for a picture nobody is reading.
    // `nodeId` may be a peer's: the ask crosses the sync link and that station's
    // connector answers it, since only the station holding a socket can say what
    // went through it.
    LocalRpcMeta {
        method: "output.watch",
        args_schema: r#"[{"name":"outputId","type":"string","optional":false},{"name":"nodeId","type":"string","optional":true},{"name":"focus","type":"string","optional":true}]"#,
        doc: "Watch what an output is putting on the wire while this client is looking.",
    },
    LocalRpcMeta {
        method: "output.unwatch",
        args_schema: r#"[{"name":"outputId","type":"string","optional":false},{"name":"nodeId","type":"string","optional":true}]"#,
        doc: "Stop watching an output; the connector stops drawing when nobody is.",
    },
    // Forcing channels is the one thing here that *drives* the rig from a diagnostic,
    // so it holds only as long as the client that asked for it: closing the page, or
    // the socket dropping, lets go. `nodeId` may be a peer's, for the reason a watch's
    // may — only the station holding the socket can put bytes on it.
    LocalRpcMeta {
        method: "output.test",
        args_schema: r#"[{"name":"outputId","type":"string","optional":false},{"name":"nodeId","type":"string","optional":true},{"name":"test","type":"object","optional":true}]"#,
        doc: "Force channels to a level on an output's wire, for as long as this client holds them. Omit `test` to let go.",
    },
    // Opening a show is not a change to the show, which is what makes these RPCs
    // rather than commands: nothing about which showfile this console has open is in
    // anybody's undo stack, and a peer must not be told to change shows because
    // somebody here did. They answer `{ok: true}` and the socket then closes, because
    // the act is this station stopping and another one starting in its place.
    LocalRpcMeta {
        method: "show.new",
        args_schema: r#"[{"name":"name","type":"string","optional":false},{"name":"demo","type":"string","optional":true},{"name":"thenJoin","type":"string","optional":true}]"#,
        doc: "Make a new show in the shows directory and open it; `demo` puts one of the demo rigs in it.",
    },
    LocalRpcMeta {
        method: "show.open",
        args_schema: r#"[{"name":"path","type":"string","optional":false}]"#,
        doc: "Open the show bundle at this path.",
    },
    LocalRpcMeta {
        method: "show.close",
        args_schema: "[]",
        doc: "Close the open show; the console lands on the welcome screen.",
    },
    LocalRpcMeta {
        method: "show.saveAs",
        args_schema: r#"[{"name":"name","type":"string","optional":false}]"#,
        doc: "Copy this show under a new name and open the copy. The copy is a new show to the network.",
    },
    LocalRpcMeta {
        method: "show.restore",
        args_schema: r#"[{"name":"versionId","type":"string","optional":false}]"#,
        doc: "Put a saved version back as the show, as one undoable edit that every station in the session follows.",
    },
    LocalRpcMeta {
        method: "show.list",
        args_schema: "[]",
        doc: "The shows this console can offer: where they live, which were opened recently, what is in the shows directory, and the demos it can make.",
    },
    // Reading a wire, as an act. On the *listening* station, deliberately: a socket is
    // one machine's, and a cross-station grab would mean shipping a universe across
    // the link carrying the show for a button press. Refused by name instead, so the
    // page can say which console to open.
    // A recording that stopped waits as the timeline's `pending` take until somebody
    // answers. Either answer is the caller's, in one gesture, so one Ctrl-Z takes a
    // Merge back and puts the take up for review again.
    LocalRpcMeta {
        method: "take.merge",
        args_schema: r#"[{"name":"timelineId","type":"string","optional":false},{"name":"takes","type":"array","optional":true},{"name":"flashes","type":"array","optional":true},{"name":"events","type":"array","optional":true}]"#,
        doc: "Merge a timeline's pending take: the take of each [fixtureId, key] in takes, the flashes of each in flashes, and the cues taken whose ids are in events, or everything where a list is left out.",
    },
    LocalRpcMeta {
        method: "take.discard",
        args_schema: r#"[{"name":"timelineId","type":"string","optional":false}]"#,
        doc: "Throw a timeline's pending take away without merging any of it.",
    },
    LocalRpcMeta {
        method: "input.grab",
        args_schema: r#"[{"name":"fixtureIds","type":"array","optional":false},{"name":"inputId","type":"string","optional":true}]"#,
        doc: "Decode what is arriving on an input for these fixtures and put it in the programmer.",
    },
    LocalRpcMeta {
        method: "wing.claim",
        args_schema: "[]",
        doc: "This page shows the command wing from now on: it shares its selection with it, and answers what needs a screen. The station runs the wing either way.",
    },
    LocalRpcMeta {
        method: "wing.release",
        args_schema: "[]",
        doc: "Stop showing the command wing. It goes on working.",
    },
    LocalRpcMeta {
        method: "wing.context",
        args_schema: r#"[{"name":"fixtures","type":"array","optional":false},{"name":"cells","type":"array","optional":true},{"name":"query","type":"object","optional":true},{"name":"seenRev","type":"number","optional":false},{"name":"focusedSequence","type":"string","optional":true},{"name":"dialog","type":"boolean","optional":false},{"name":"userId","type":"string","optional":true}]"#,
        doc: "What the page showing the wing has selected, which cue sheet it touched last, whether a dialog is open on it, and who is at it.",
    },
    LocalRpcMeta {
        method: "wing.learn",
        args_schema: r#"[{"name":"on","type":"boolean","optional":false}]"#,
        doc: "Start or stop learning: the next control touched on the wing is handed to this page to bind.",
    },
    LocalRpcMeta {
        method: "wing.calculator",
        args_schema: r#"[{"name":"slot","type":"number","optional":true},{"name":"text","type":"string","optional":true},{"name":"set","type":"boolean","optional":true}]"#,
        doc: "Open an encoder's calculator (no slot closes it), with what is typed, and set it on the wing's selection when `set`.",
    },
    LocalRpcMeta {
        method: "wing.map",
        args_schema: "[]",
        doc: "MA's names for every key, fader, encoder and LED of the command wing, where this machine has a grandMA3 install to read them from.",
    },
    LocalRpcMeta {
        method: "client.report",
        args_schema: r#"[{"name":"stats","type":"object","optional":false}]"#,
        doc: "What a browser is costing itself: frame rate, evaluator time, memory, clock offset. Answers the key it landed under.",
    },
];

/// What dispatching needs to reach. Cheap to clone: a few channel handles.
#[derive(Clone)]
pub struct LocalRpcDeps {
    pub session: SessionHandle,
    pub devices: DeviceHandle,
    /// The console's own log, where this process installed one.
    pub log: Option<crate::logging::LogHandle>,
    /// Who is watching which peer's log. Kept beside the log rather than in it,
    /// because it is about this station's *clients* and not about its lines.
    pub log_watchers: crate::logging::Watchers,
    /// The link to the peers, for the one thing here that reaches one: telling a
    /// peer what to publish.
    pub sync: Option<crate::infra::sync::SyncHandle>,
    /// The WebSocket session that asked, where one did.
    ///
    /// `None` for a plugin, which has no browser behind it — so `log.watch` and
    /// `log.report`, the two calls whose whole meaning is "while *this* client is
    /// here", say so rather than pretending. Every other call ignores it.
    pub caller: Option<uuid::Uuid>,
    /// MVR-xchange, where this station is running one. `None` in a test that built
    /// its own dependencies, which is what makes the three calls say so rather than
    /// panic.
    pub xchange: Option<crate::infra::interop::xchange::XchangeHandle>,
    /// The command wing, where this station has one. `None` in a test, which is what
    /// makes the wing's calls say so rather than panic.
    pub wing: Option<crate::infra::wing::WingStation>,
    /// For the calls that answer a question about the show. Reads only — anything
    /// here that wanted to write would be an entity command instead.
    pub engine: EngineHandle,
    /// The sockets this station is serving, for the one thing a page cannot measure
    /// about itself: how much has been sent to it. `None` where there is no HTTP
    /// server behind these deps, which is what a test constructs.
    pub ws_registry: Option<crate::api::ws::SubscriptionRegistry>,
    /// Which station this is, so an ask about an output can be told apart from an
    /// ask about a peer's — the first goes to a connector here, the second down a
    /// link.
    pub node_id: pult_schema::events::operation::NodeId,
    /// Who is watching what an output is putting on the wire.
    pub viewers: crate::infra::connectors::Viewers,
    /// The listening side, for the one call that reads a wire. `None` where there is
    /// no input manager behind these deps, which is what a test constructs — and what
    /// makes `input.grab` say so rather than panic.
    pub input: Option<crate::infra::connectors::input::InputHandle>,
    /// The sound, for the three calls that reach a device or a model. `None` where
    /// there is no audio manager behind these deps, which is what a test constructs —
    /// and what makes those three say so rather than panic.
    pub audio: Option<crate::infra::audio::AudioHandle>,
    /// Where a show act goes, and what this station has open.
    pub shows: crate::ShowsHandle,
    /// What the browsers on this station are saying about themselves.
    ///
    /// `None` where there is no HTTP server behind these deps at all, which is what
    /// a test constructs. A plugin's deps carry one and still cannot report: the call
    /// needs a `caller`, and a plugin has no browser to be.
    pub clients: Option<crate::infra::clients::ClientRegistry>,
}

fn no_audio() -> String {
    "this station has no audio manager".to_string()
}

fn no_log() -> String {
    "this station has no log; it was started without one".to_string()
}


/// Read a level argument, which may be absent but must not be nonsense.
fn level_arg(args: &Value, name: &str) -> Result<Option<pult_schema::ws::LogLevel>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let text = v.as_str().ok_or_else(|| format!("{name} should be a level, as a string"))?;
            pult_schema::ws::LogLevel::parse(text)
                .map(Some)
                .ok_or_else(|| format!("no such level: {text}"))
        }
    }
}

/// Tell a peer what to publish now that who is watching it has changed.
async fn raise_peer(deps: &LocalRpcDeps, node_id: uuid::Uuid, level: Option<pult_schema::ws::LogLevel>) {
    if let Some(sync) = &deps.sync {
        sync.raise_peer_log(pult_schema::events::operation::NodeId(node_id), level).await;
    }
}

/// Which sequences are live, by name.
///
/// Read for one reason: applying somebody else's commit repatches addresses, and a
/// designer's mid-afternoon file landing during a cue is the failure this feature would
/// otherwise be remembered for. The console cannot put a dialog in the room, so it
/// hands the list back and the person who can see the stage decides.
async fn running_sequences(engine: &EngineHandle) -> Vec<String> {
    let Ok(value) = engine.get(vec![PathSegment::Key("sequences".into())]).await else {
        return Vec::new();
    };
    let sequences: Vec<pult_schema::types::Sequence> =
        serde_json::from_value(value).unwrap_or_default();
    sequences
        .into_iter()
        .filter(|sequence| sequence.active_cue_index.is_some())
        .map(|sequence| sequence.name)
        .collect()
}

/// Who is asking, for the writes an RPC can cause.
///
/// Only `xchange.apply` needs one, and it needs it badly: an applied commit is an
/// ordinary import and belongs to the operator who clicked, from whichever station
/// they clicked at. A browser that has not identified itself writes as the default
/// user, which is what every other path already does.
fn asking_user(deps: &LocalRpcDeps) -> uuid::Uuid {
    match (&deps.ws_registry, deps.caller) {
        (Some(registry), Some(session)) => registry.user_for_writes(session),
        _ => pult_schema::types::user::User::DEFAULT_ID,
    }
}

/// Answer one station RPC. `Err` is a message for whoever asked, so it is a
/// plain string the way the WebSocket's `CallResult.error` is.
pub async fn dispatch(method: &str, args: Value, deps: &LocalRpcDeps) -> Result<Value, String> {
    let serial = || {
        args["serial"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "missing serial".to_string())
    };

    // Only the leader drives devices, so on a follower an act on one is the leader's to
    // do: asked over its own page socket, and its answer — a fixture id, or why not — is
    // this one's. Before, a page served by a follower (the Pi, the audio MacBook's own
    // window) showed Adopt greyed out with a "Watching" badge as the only explanation.
    if method.starts_with("device.") {
        if let Some((hostname, http_addr)) = leader_to_relay_to(deps).await {
            return call_station(&hostname, &http_addr, method, args.clone(), std::time::Duration::from_secs(20))
                .await
                .map_err(|why| format!("{hostname} (leading): {why}"));
        }
    }

    match method {
        "device.adopt" => {
            let id = deps.devices.adopt(serial()?).await?;
            serde_json::to_value(id).map_err(|e| e.to_string())
        }
        "device.identify" => {
            deps.devices.identify(serial()?).await?;
            Ok(Value::Null)
        }
        "device.forget" => {
            deps.devices.forget(serial()?).await?;
            Ok(Value::Null)
        }
        "device.strip" => {
            let head = args["head"]
                .as_u64()
                .and_then(|h| u8::try_from(h).ok())
                .ok_or_else(|| "missing head".to_string())?;
            let change: crate::infra::devices::StripChange = serde_json::from_value(args.clone())
                .map_err(|e| format!("invalid strip: {e}"))?;
            let id = deps.devices.patch_strip(serial()?, head, change, asking_user(deps)).await?;
            serde_json::to_value(id).map_err(|e| e.to_string())
        }
        "fixtureTypes.library" => Ok(serde_json::json!({
            "types": pult_schema::types::generic::library(),
            "emitterPresets": pult_schema::types::generic::emitter_presets(),
        })),
        "parameter.value" => {
            let fixture_id: uuid::Uuid = serde_json::from_value(args["fixtureId"].clone())
                .map_err(|e| format!("invalid fixtureId: {e}"))?;
            let kind = match args.get("parameterKind") {
                Some(k) if !k.is_null() => Some(
                    serde_json::from_value::<ParameterKind>(k.clone())
                        .map_err(|e| format!("invalid parameterKind: {e}"))?,
                ),
                _ => None,
            };
            what_is_it_doing(&deps.engine, fixture_id, kind).await
        }
        "take.merge" | "take.discard" => {
            let timeline_id: uuid::Uuid = serde_json::from_value(args["timelineId"].clone())
                .map_err(|e| format!("invalid timelineId: {e}"))?;
            let keys = |name: &str| -> Result<Option<Vec<(uuid::Uuid, String)>>, String> {
                match args.get(name) {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map(Some)
                        .map_err(|e| format!("invalid {name}: {e}")),
                    _ => Ok(None),
                }
            };
            if method == "take.discard" {
                answer_a_take(deps, timeline_id, Some(Vec::new()), Some(Vec::new()), Some(Vec::new())).await
            } else {
                let events: Option<Vec<uuid::Uuid>> = match args.get("events") {
                    Some(v) if !v.is_null() => {
                        Some(serde_json::from_value(v.clone()).map_err(|e| format!("invalid events: {e}"))?)
                    }
                    _ => None,
                };
                answer_a_take(deps, timeline_id, keys("takes")?, keys("flashes")?, events).await
            }
        }
        "input.grab" => {
            let fixture_ids: Vec<uuid::Uuid> =
                serde_json::from_value(args.get("fixtureIds").cloned().unwrap_or_default())
                    .map_err(|e| format!("invalid fixtureIds: {e}"))?;
            let input_id = match args.get("inputId") {
                Some(v) if !v.is_null() => Some(
                    serde_json::from_value(v.clone())
                        .map_err(|e| format!("invalid inputId: {e}"))?,
                ),
                _ => None,
            };
            grab_into_the_programmer(deps, input_id, fixture_ids).await
        }
        "audio.devices" => {
            let audio = deps.audio.as_ref().ok_or_else(no_audio)?;
            serde_json::to_value(audio.devices().await).map_err(|e| e.to_string())
        }
        "variable.set" | "variable.add" | "variable.toggle" | "variable.reset" => {
            use pult_schema::types::VariableOp;
            let variable_id: uuid::Uuid = serde_json::from_value(args["variableId"].clone())
                .map_err(|e| format!("invalid variableId: {e}"))?;
            let number = |name: &str| {
                args.get(name)
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| format!("{method} needs a number `{name}`"))
            };
            let op = match method {
                "variable.set" => VariableOp::Set(number("value")?),
                "variable.add" => VariableOp::Add(number("by")?),
                "variable.toggle" => VariableOp::Toggle,
                _ => VariableOp::Reset,
            };
            deps.engine.set_variable(variable_id, op).await.map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "ok": true }))
        }
        "timeline.detect" => {
            let audio = deps.audio.as_ref().ok_or_else(no_audio)?;
            let timeline_id: uuid::Uuid = serde_json::from_value(args["timelineId"].clone())
                .map_err(|e| format!("invalid timelineId: {e}"))?;
            let clip_id = match args.get("clipId") {
                Some(v) if !v.is_null() => Some(
                    serde_json::from_value(v.clone()).map_err(|e| format!("invalid clipId: {e}"))?,
                ),
                _ => None,
            };
            let found = audio.detect(timeline_id, clip_id).await?;
            serde_json::to_value(found).map_err(|e| e.to_string())
        }
        "timeline.model" => {
            // A read unless `full` is true, and even then it answers at once and
            // downloads on a task — eighty megabytes is not something to hold an RPC
            // open for, and a browser asking again is how it watches the progress.
            let state = match args.get("full").and_then(|v| v.as_bool()).unwrap_or(false) {
                true => crate::infra::audio::models::fetch().await,
                false => crate::infra::audio::models::state().await,
            };
            serde_json::to_value(state).map_err(|e| e.to_string())
        }
        "log.tail" => {
            let log = deps.log.as_ref().ok_or_else(no_log)?;
            let limit = args["limit"].as_u64().unwrap_or(1_000).min(5_000) as usize;
            let level = level_arg(&args, "level")?;
            serde_json::to_value(serde_json::json!({
                "lines": log.tail(limit, level),
                // The panel's header is built from these: which station this is,
                // what it is keeping, what it is telling its peers, and where the
                // rest of the log went when it scrolled out of the ring.
                "nodeId": log.node_id(),
                "captureLevel": log.capture_level(),
                "publishLevel": log.publish_level(),
                "file": log.file_path().map(|p| p.display().to_string()),
                "raised": deps.log_watchers.raised(),
            }))
            .map_err(|e| e.to_string())
        }
        "log.setLevel" => {
            let log = deps.log.as_ref().ok_or_else(no_log)?;
            let mut prefs = crate::infra::preferences::load();
            if let Some(level) = level_arg(&args, "level")? {
                log.set_capture_level(level);
                prefs.log_level = level.as_str().to_string();
            }
            if let Some(level) = level_arg(&args, "peerLevel")? {
                prefs.peer_log_level = level.as_str().to_string();
            }
            // Re-read through `sane`, which is what applies the rule that a station
            // never promises its peers more than it keeps for itself.
            let prefs = prefs.sane();
            log.set_capture_level(prefs.capture_level());
            log.set_publish_level(prefs.peer_level());
            // A level that cannot be written down still takes effect for this run;
            // the panel said what it said, and refusing the change because the disk
            // is read-only would be the wrong half to give up on.
            if let Err(e) = crate::infra::preferences::save(&prefs) {
                tracing::warn!("could not write the log level to preferences: {e}");
            }
            Ok(serde_json::json!({
                "captureLevel": log.capture_level(),
                "publishLevel": log.publish_level(),
            }))
        }
        "log.watch" => {
            let caller = deps.caller.ok_or_else(|| {
                "log.watch is about one client watching, so it needs one".to_string()
            })?;
            let node_id: uuid::Uuid = serde_json::from_value(args["nodeId"].clone())
                .map_err(|e| format!("invalid nodeId: {e}"))?;
            let level = level_arg(&args, "level")?.unwrap_or(pult_schema::ws::LogLevel::Debug);
            if let Some(level) = deps.log_watchers.watch(node_id, caller, level) {
                raise_peer(deps, node_id, level).await;
            }
            Ok(Value::Null)
        }
        "log.unwatch" => {
            let caller = deps.caller.ok_or_else(|| "nobody is watching".to_string())?;
            let node_id: uuid::Uuid = serde_json::from_value(args["nodeId"].clone())
                .map_err(|e| format!("invalid nodeId: {e}"))?;
            if let Some(level) = deps.log_watchers.unwatch(node_id, caller) {
                raise_peer(deps, node_id, level).await;
            }
            Ok(Value::Null)
        }
        "log.report" => {
            let log = deps.log.as_ref().ok_or_else(no_log)?;
            let level = level_arg(&args, "level")?.unwrap_or(pult_schema::ws::LogLevel::Error);
            let message = args["message"].as_str().unwrap_or_default();
            if message.is_empty() {
                return Err("nothing to report".to_string());
            }
            // The browser has already deduped and rate-limited; `count` is how many
            // times the thing it is reporting happened. Said here rather than
            // repeated, so a panel erroring every frame is one line and a number.
            let count = args["count"].as_u64().unwrap_or(1);
            let message = if count > 1 {
                format!("{message} (×{count})")
            } else {
                message.to_string()
            };
            // Truncated, because a stack trace is not a log line and a browser is
            // not a trusted length.
            let message: String = message.chars().take(4_000).collect();
            let source = pult_schema::ws::LogSource::Browser(
                deps.caller.map(short_id).unwrap_or_else(|| "?".into()),
            );
            log.emit(level, "browser", source, message);
            Ok(Value::Null)
        }
        "wing.claim" | "wing.release" | "wing.context" | "wing.learn" | "wing.calculator" | "wing.map" => {
            use crate::infra::wing::Inbound;
            let wing = deps
                .wing
                .as_ref()
                .ok_or_else(|| "this station has no command wing".to_string())?;
            if method == "wing.map" {
                return Ok(wing.map());
            }
            // The rest are a page showing a desk, so they need a page.
            let caller = deps
                .caller
                .ok_or_else(|| format!("{method} is a page showing the wing, so it needs one"))?;
            match method {
                "wing.claim" => serde_json::to_value(wing.claim(caller).await).map_err(|e| e.to_string()),
                "wing.release" => {
                    serde_json::to_value(wing.release(caller).await).map_err(|e| e.to_string())
                }
                "wing.context" => {
                    let context = serde_json::from_value(args).map_err(|e| format!("wing.context: {e}"))?;
                    wing.from_page(caller, Inbound::Context(context))?;
                    Ok(Value::Null)
                }
                "wing.learn" => {
                    let on = args.get("on").and_then(Value::as_bool).unwrap_or(false);
                    wing.from_page(caller, Inbound::Learn(on))?;
                    Ok(Value::Null)
                }
                _ => {
                    let slot = args.get("slot").and_then(Value::as_u64).map(|s| s.min(4) as u8);
                    let text = args.get("text").and_then(Value::as_str).map(String::from);
                    let set = args.get("set").and_then(Value::as_bool).unwrap_or(false);
                    wing.from_page(caller, Inbound::Calculator { slot, text, set })?;
                    Ok(Value::Null)
                }
            }
        }
        "client.report" => {
            let caller = deps.caller.ok_or_else(|| {
                "client.report is a browser describing itself, so it needs one".to_string()
            })?;
            let clients = deps
                .clients
                .as_ref()
                .ok_or_else(|| "this station is not serving browsers".to_string())?;
            let stats: pult_schema::types::client::ClientStats =
                serde_json::from_value(args["stats"].clone())
                    .map_err(|e| format!("invalid stats: {e}"))?;
            // Trimmed for the same reason a reported message is: a browser is not a
            // trusted length, and this one ends up on every panel showing the station.
            let stats = pult_schema::types::client::ClientStats {
                label: stats.label.chars().take(120).collect(),
                ..stats
            };
            // What this station has sent down that socket since the page last
            // reported — the one figure here the page cannot honestly supply, because
            // no browser API says how many bytes arrived on a WebSocket. Drained
            // here, on the page's own schedule, so the window is the gap between two
            // of its reports.
            let sent_bytes = deps
                .ws_registry
                .as_ref()
                .map(|registry| registry.take_sent_bytes(caller))
                .unwrap_or(0);
            let key = clients.report(caller, stats, sent_bytes).await;
            // Answering the key it wrote under is the whole of how a page knows
            // which row in the panel is itself: a browser is not told its session id
            // anywhere else, and it must not be able to name one for itself.
            Ok(Value::String(key))
        }
        "output.watch" | "output.unwatch" => {
            let caller = deps.caller.ok_or_else(|| {
                "watching is about one client looking, so it needs one".to_string()
            })?;
            let output_id: uuid::Uuid = serde_json::from_value(args["outputId"].clone())
                .map_err(|e| format!("invalid outputId: {e}"))?;
            // The station that holds the socket, defaulting to this one — which is
            // what an output with no station of its own means anyway.
            let node = match args.get("nodeId") {
                Some(v) if !v.is_null() => pult_schema::events::operation::NodeId(
                    serde_json::from_value(v.clone())
                        .map_err(|e| format!("invalid nodeId: {e}"))?,
                ),
                _ => deps.node_id,
            };
            let moved = if method == "output.watch" {
                let focus = args["focus"].as_str().map(str::to_string);
                deps.viewers.watch(node, output_id, caller, focus)
            } else {
                deps.viewers.unwatch(node, output_id, caller)
            };
            // A peer's connector only draws while it is asked to, so the new answer
            // goes down the link — including the answer "nobody, stop". Nothing is
            // sent for an ask that did not move.
            if node != deps.node_id {
                if let (Some(sync), Some(ask)) = (&deps.sync, moved) {
                    sync.watch_peer_output(node, output_id, ask).await;
                }
            }
            Ok(Value::Null)
        }
        "output.test" => {
            let caller = deps.caller.ok_or_else(|| {
                "forcing channels is about one client holding them, so it needs one".to_string()
            })?;
            let output_id: uuid::Uuid = serde_json::from_value(args["outputId"].clone())
                .map_err(|e| format!("invalid outputId: {e}"))?;
            let node = match args.get("nodeId") {
                Some(v) if !v.is_null() => pult_schema::events::operation::NodeId(
                    serde_json::from_value(v.clone())
                        .map_err(|e| format!("invalid nodeId: {e}"))?,
                ),
                _ => deps.node_id,
            };
            // A missing test is the withdrawal, so a page that unmounts can say so
            // without building one.
            let test: pult_schema::types::output::ChannelTest = match args.get("test") {
                Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid test: {e}"))?,
                _ => Default::default(),
            };
            let test = test.checked()?;
            let moved = deps.viewers.tests.set(node, output_id, caller, test);
            if node != deps.node_id {
                if let Some(sync) = &deps.sync {
                    // Always sent for a peer's output, moved or not, because the link
                    // can have dropped and taken the test with it since the page last
                    // said it. The page repeats the same test every few seconds while
                    // it holds one, and this is what makes that mean something.
                    let merged = deps
                        .viewers
                        .tests
                        .merged_for(node, output_id)
                        .or(moved)
                        .unwrap_or_default();
                    sync.test_peer_output(node, output_id, merged).await;
                }
            }
            Ok(Value::Null)
        }
        "show.new" | "show.open" | "show.close" | "show.saveAs" => {
            open_a_show(method, &args, &deps.shows).await
        }
        "show.restore" => {
            let version: uuid::Uuid = serde_json::from_value(args["versionId"].clone())
                .map_err(|e| format!("invalid versionId: {e}"))?;
            restore_a_show(version, &deps.shows, &deps.engine, asking_user(deps)).await
        }
        "show.list" => list_shows(&deps.shows).await,
        "paperwork.tables" => {
            let request: pult_schema::types::paperwork::TableRequest =
                serde_json::from_value(args.clone())
                    .map_err(|e| format!("invalid table request: {e}"))?;
            paperwork_table(&deps.engine, request).await
        }
        "paperwork.cueValues" => {
            let shots: Vec<pult_schema::types::paperwork::CueShot> =
                serde_json::from_value(args["cues"].clone())
                    .map_err(|e| format!("invalid cues: {e}"))?;
            paperwork_cue_values(&deps.engine, shots).await
        }
        "xchange.commit" => {
            let comment =
                args["comment"].as_str().ok_or_else(|| "missing comment".to_string())?.to_string();
            let xchange = deps.xchange.as_ref().ok_or("no exchange on this station")?;
            xchange
                .ask(pult_schema::types::XchangeAsk::Commit {
                    comment,
                    user_id: asking_user(deps),
                })
                .await
        }
        "xchange.apply" => {
            let file_uuid: uuid::Uuid = serde_json::from_value(args["fileUuid"].clone())
                .map_err(|e| format!("invalid fileUuid: {e}"))?;
            let xchange = deps.xchange.as_ref().ok_or("no exchange on this station")?;

            // Warned, not refused. Whether the house is in is not a thing the console
            // can know, and a rule that refused while anything was live would be a
            // rule that refused all afternoon — a sequence is nearly always parked
            // live during exactly the iteration this feature is for.
            if !args["confirm"].as_bool().unwrap_or(false) {
                let running = running_sequences(&deps.engine).await;
                if !running.is_empty() {
                    return Ok(serde_json::json!({ "needsConfirm": true, "running": running }));
                }
            }

            xchange
                .ask(pult_schema::types::XchangeAsk::Apply {
                    file_uuid,
                    user_id: asking_user(deps),
                })
                .await
        }
        "xchange.followHost" => {
            let follow = args["follow"].as_bool().ok_or_else(|| "missing follow".to_string())?;
            let xchange = deps.xchange.as_ref().ok_or("no exchange on this station")?;
            xchange.ask(pult_schema::types::XchangeAsk::FollowHost { follow }).await
        }
        "selection.resolve" => {
            let group_id: uuid::Uuid = serde_json::from_value(args["groupId"].clone())
                .map_err(|e| format!("invalid groupId: {e}"))?;
            resolve_group(&deps.engine, group_id).await
        }
        "selection.targets" => {
            let ids: Vec<uuid::Uuid> = serde_json::from_value(args["fixtureIds"].clone())
                .map_err(|e| format!("invalid fixtureIds: {e}"))?;
            let cells: Vec<pult_schema::types::group::CellSpan> = match args.get("cells") {
                Some(c) if !c.is_null() => {
                    serde_json::from_value(c.clone()).map_err(|e| format!("invalid cells: {e}"))?
                }
                _ => Vec::new(),
            };
            let kind: pult_schema::types::fixture::ParameterKind =
                serde_json::from_value(args["parameterKind"].clone())
                    .map_err(|e| format!("invalid parameterKind: {e}"))?;
            selection_targets(&deps.engine, &ids, &cells, &kind).await
        }
        "session.join" => {
            let session_id: uuid::Uuid = serde_json::from_value(args["sessionId"].clone())
                .map_err(|e| format!("invalid sessionId: {e}"))?;
            deps.session.join_session(session_id).await?;
            Ok(Value::Null)
        }
        "session.joinAt" => {
            let address = args["address"].as_str().ok_or("address is a host:port")?;
            let addr = resolve_sync_address(address).await?;
            deps.session.join_at(addr).await?;
            Ok(Value::Null)
        }
        "session.invite" => {
            let guest = match args["address"].as_str().map(str::trim).filter(|a| !a.is_empty()) {
                Some(address) => Invitee::At(address.to_string()),
                None => Invitee::Node(
                    serde_json::from_value(args["nodeId"].clone())
                        .map_err(|e| format!("invalid nodeId: {e}"))?,
                ),
            };
            invite(deps, guest).await?;
            Ok(Value::Null)
        }
        "session.watchNearby" => {
            let caller = deps.caller.ok_or("session.watchNearby is a page's, so it needs one")?;
            let on = args["on"].as_bool().ok_or("on is true or false")?;
            deps.session.watch_nearby(caller, on).await;
            Ok(Value::Null)
        }
        "session.leave" => {
            let _ = deps
                .session
                .0
                .send(crate::infra::session::SessionCommand::Leave)
                .await;
            Ok(Value::Null)
        }
        "session.create" => {
            let show_name = args["showName"].as_str().unwrap_or("Untitled Show").to_string();
            let show_id: uuid::Uuid = serde_json::from_value(args["showId"].clone())
                .map_err(|e| format!("invalid showId: {e}"))?;
            match deps.session.create_session(show_name, show_id).await {
                Some(id) => serde_json::to_value(id).map_err(|e| e.to_string()),
                None => Err("failed to create session".into()),
            }
        }
        _ => Err(format!("unknown method: {method}")),
    }
}

/// Ask the station `node_id` to join this one, at this station's sync address.
///
/// The invited station does the joining, exactly as if somebody had pressed Join on
/// it: this one connects to its socket and calls its own `session.joinAt`. So nothing
/// about the sync handshake changes, and a station the operator cannot walk to — a Pi
/// in a rack — is brought in from the desk. The ask is answered once the invited
/// station has joined or failed to, and its reason comes back with it.
/// Who an invitation goes to: a station found on the network, or one named by the
/// address its page is served from, for a network mDNS does not cross.
enum Invitee {
    Node(uuid::Uuid),
    At(String),
}

async fn invite(deps: &LocalRpcDeps, invitee: Invitee) -> Result<(), String> {
    use pult_schema::types::session::SessionState;

    let read = |path: Vec<PathSegment>| async {
        deps.engine.get(path).await.map_err(|e| e.to_string())
    };
    let session: SessionState = serde_json::from_value(read(vec![PathSegment::Key("session".into())]).await?)
        .map_err(|e| e.to_string())?;
    if session.is_follower {
        return Err("this station follows another; invite from the one it follows".into());
    }
    // What the station is called in a message, and where its socket is.
    let (hostname, http_addr) = match invitee {
        Invitee::Node(node_id) => {
            let guest = session
                .nearby
                .iter()
                .find(|s| s.node_id.0 == node_id)
                .ok_or("that station is not on the network as seen from here")?;
            (guest.hostname.clone(), guest.http_addr.clone())
        }
        Invitee::At(address) => (address.clone(), address),
    };

    // A session to be invited into. A lone station leads one as soon as anybody joins
    // it, but would say Idle about it; saying Advertising is the truth.
    if !session.is_advertising {
        let show = read(vec![PathSegment::Key("show".into())]).await?;
        let show_id: uuid::Uuid = serde_json::from_value(show["id"].clone())
            .map_err(|_| "no show is open here, so there is nothing to invite anybody into")?;
        let show_name = show["name"].as_str().unwrap_or("Untitled Show").to_string();
        deps.session.create_session(show_name, show_id).await.ok_or("could not start a session")?;
    }

    // Where the guest should dial: the address this station tells every peer, and the
    // one its row already publishes for exactly that.
    let me = read(vec![PathSegment::Key("stations".into()), PathSegment::Id(deps.node_id.0)]).await?;
    let sync_addr = me["sync_addr"].as_str().filter(|a| !a.is_empty()).ok_or("this station has not said its sync address yet; try again in a moment")?.to_string();

    call_station(
        &hostname,
        &http_addr,
        "session.joinAt",
        serde_json::json!({ "address": sync_addr }),
        // Joining is a handshake and a whole show's snapshot, which on a Pi is seconds.
        std::time::Duration::from_secs(30),
    )
    .await
    .map(|_| ())
    .map_err(|why| format!("{hostname} did not join: {why}"))
}

/// Call one RPC on another station, over its own page socket, and answer what it said.
///
/// The same door a browser uses, so the station asked cannot tell this from a person
/// at a tablet — and needs nothing added to the sync protocol to be asked.
async fn call_station(
    hostname: &str,
    http_addr: &str,
    method: &str,
    args: Value,
    within: std::time::Duration,
) -> Result<Value, String> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Frame;

    let url = format!("ws://{http_addr}/ws");
    let ask = async {
        let (mut socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| format!("could not reach {hostname} at {http_addr}: {e}"))?;
        let call = serde_json::json!({
            "type": "Call",
            "payload": { "request_id": "relay", "method": method, "args": args }
        });
        socket.send(Frame::Text(call.to_string().into())).await.map_err(|e| e.to_string())?;
        while let Some(frame) = socket.next().await {
            let Ok(Frame::Text(text)) = frame else { continue };
            let Ok(message) = serde_json::from_str::<Value>(&text) else { continue };
            let payload = &message["payload"];
            if payload["request_id"] != "relay" {
                continue;
            }
            let _ = socket.close(None).await;
            return match payload["error"].as_str() {
                Some(why) => Err(why.to_string()),
                None => Ok(payload["result"].clone()),
            };
        }
        Err(format!("{hostname} hung up before answering"))
    };
    tokio::time::timeout(within, ask)
        .await
        .map_err(|_| format!("{hostname} did not answer within {} s", within.as_secs()))?
}

/// The leader's name and page address, when this station is somebody's follower.
///
/// `None` on the leader itself and on a station in no session, which both act here.
async fn leader_to_relay_to(deps: &LocalRpcDeps) -> Option<(String, String)> {
    let session: pult_schema::types::session::SessionState =
        serde_json::from_value(deps.engine.get(vec![PathSegment::Key("session".into())]).await.ok()?).ok()?;
    if !session.is_follower {
        return None;
    }
    let leader = deps.sync.as_ref()?.leader().await?;
    if leader == deps.node_id {
        return None;
    }
    let row = deps
        .engine
        .get(vec![PathSegment::Key("stations".into()), PathSegment::Id(leader.0)])
        .await
        .ok()?;
    let http_addr = row["http_addr"].as_str().filter(|a| !a.is_empty())?.to_string();
    let hostname = row["hostname"].as_str().unwrap_or("the leader").to_string();
    Some((hostname, http_addr))
}

/// `host:port`, where the host may be a name: a station somebody reads off another
/// console's Stations panel is as often `booth.local` as it is an address.
async fn resolve_sync_address(address: &str) -> Result<std::net::SocketAddr, String> {
    let mut found = tokio::net::lookup_host(address)
        .await
        .map_err(|e| format!("{address} is not a host:port this station can reach: {e}"))?;
    found
        .find(|a| a.is_ipv4())
        .ok_or_else(|| format!("{address} has no IPv4 address"))
}

/// Does this method name belong to the station rather than to an entity?
pub fn is_local_rpc(method: &str) -> bool {
    method.starts_with("show.")
        || method.starts_with("session.")
        || method.starts_with("device.")
        || method.starts_with("selection.")
        || method.starts_with("parameter.")
        || method.starts_with("paperwork.")
        || method.starts_with("log.")
        || method.starts_with("output.")
        || method.starts_with("input.")
        || method.starts_with("audio.")
        // Singular, and it does not collide with the `timelines` collection: an
        // entity command is a *path* write (`["timelines", id, "play"]`) and never
        // reaches this function, which only ever sees the name of a `Call`.
        || method.starts_with("timeline.")
        || method.starts_with("variable.")
        || method.starts_with("client.")
        || method.starts_with("xchange.")
        || method.starts_with("wing.")
        // Camel case, unlike the `fixture_types` collection, for the reason above.
        || method.starts_with("fixtureTypes.")
}

/// The four acts that change which show this console has open.
///
/// Each answers `{ok: true}` and *then* the station stops — the caller is told the
/// act was taken, not that it finished, because finishing it means closing the
/// socket the answer would have gone down. The client sees a disconnect and a
/// reconnect, which is what it already does whenever a station goes away.
///
/// Takes the shows handle rather than the whole of [`LocalRpcDeps`], which is the
/// whole of what it touches — and is what lets a test drive the same arm a browser
/// reaches without standing up a browser to do it.
pub async fn open_a_show(
    method: &str,
    args: &Value,
    shows: &crate::ShowsHandle,
) -> Result<Value, String> {
    use crate::infra::showfile::bundle;

    let asked = match method {
        "show.close" => crate::ShowSwitch::Close,
        "show.open" => {
            let path = args["path"].as_str().unwrap_or_default();
            if path.is_empty() {
                return Err("which show?".into());
            }
            // Opened here rather than after the restart, so a path that is not a show
            // is refused while somebody is still looking at the dialog they typed it
            // into — instead of taking the console down and bringing it back with
            // nothing open and an error in the log.
            let bundle = bundle::Bundle::open(path).map_err(|e| format!("{e:#}"))?;
            crate::ShowSwitch::Open {
                path: bundle.path().to_path_buf(),
                then_join: None,
                demo: None,
            }
        }
        "show.new" => {
            let name = args["name"].as_str().unwrap_or("Untitled Show").to_string();
            let dir = shows
                .shows_dir
                .clone()
                .ok_or_else(|| "this console has nowhere to keep shows".to_string())?;
            let path = bundle::free_path_in(&dir, &name);
            bundle::Bundle::create(&path, &name).map_err(|e| format!("{e:#}"))?;
            let then_join = match args.get("thenJoin") {
                Some(v) if !v.is_null() => Some(
                    serde_json::from_value::<uuid::Uuid>(v.clone())
                        .map_err(|e| format!("invalid thenJoin: {e}"))?,
                ),
                _ => None,
            };
            let demo = match args.get("demo") {
                Some(v) if !v.is_null() => Some(
                    v.as_str()
                        .and_then(crate::demo::Demo::parse)
                        .ok_or_else(|| format!("no such demo: {v}"))?,
                ),
                _ => None,
            };
            crate::ShowSwitch::Open { path, then_join, demo }
        }
        "show.saveAs" => {
            let name = args["name"].as_str().unwrap_or_default().trim().to_string();
            if name.is_empty() {
                return Err("what should the copy be called?".into());
            }
            let open = shows
                .bundle
                .as_ref()
                .ok_or_else(|| "there is no show open to copy".to_string())?;
            // Beside the show it is a copy of, when that is where the show lives —
            // an operator working out of a folder on a stick expects the copy on the
            // stick, not in the console's own shows directory.
            let dir = open
                .path()
                .parent()
                .map(std::path::Path::to_path_buf)
                .or_else(|| shows.shows_dir.clone())
                .ok_or_else(|| "this console has nowhere to keep shows".to_string())?;
            crate::ShowSwitch::SaveAs { path: bundle::free_path_in(&dir, &name) }
        }
        other => return Err(format!("unknown method: {other}")),
    };

    shows.ask(asked).await?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Put a saved version back as the show.
///
/// Refused with anybody else in the session, and refused **here** rather than after
/// the switch. A peer holds the show as it is now and will replay its newer
/// operations over whatever this station puts back, so restoring in company does not
/// restore anything: it briefly shows an older show and then loses it again.
pub async fn restore_a_show(
    version: uuid::Uuid,
    shows: &crate::ShowsHandle,
    engine: &EngineHandle,
    user: uuid::Uuid,
) -> Result<Value, String> {
    let bundle = shows.bundle.as_ref().ok_or_else(|| "no show is open".to_string())?;

    // A point to come back *from*, taken before the show is overwritten. Restoring is
    // the one act in this console that throws work away, and an operator who restores
    // the wrong version must have somewhere to go — beside the Ctrl-Z that now takes a
    // restore back, because a version outlives an undo history.
    //
    // Automatic and unattributed: nobody asked for this one, and it must not be the
    // thing Ctrl-Z reaches for.
    let name = restoring_from(engine, version).await;
    let taken = engine
        .set(
            vec![
                PathSegment::Key("versions".into()),
                PathSegment::Key("__checkpoint".into()),
            ],
            pult_schema::lifecycle::Lifecycle::Persisted,
            serde_json::json!({ "name": name, "automatic": true }),
        )
        .await;
    if let Err(e) = taken {
        // Said and then carried on: an operator who asked to restore should not be
        // stopped by the console's own safety net failing to be written.
        tracing::warn!("[versions] could not save a point to come back from: {e}");
    }

    // In place, as writes: see `versions::restore_in_place` for why not a file swap.
    let changed = crate::infra::showfile::versions::restore_in_place(bundle, version, engine, user)
        .await
        .map_err(|e| format!("{e:#}"))?;
    Ok(serde_json::json!({ "ok": true, "changed": changed }))
}

/// What to call the version taken just before a restore.
///
/// The name of the one being restored, where it has one — and *only* then. A version
/// nobody named is shown by when it was taken, and a station has no business putting
/// a time in a name: it would format in UTC, and sit in the list beside its own row
/// rendered in the reader's local time, disagreeing with itself by an hour or nine.
/// The row already says when, so the name does not have to.
async fn restoring_from(engine: &EngineHandle, version: uuid::Uuid) -> String {
    let named = engine
        .get(vec![PathSegment::Key("versions".into())])
        .await
        .ok()
        .and_then(|v| serde_json::from_value::<Vec<pult_schema::types::Version>>(v).ok())
        .and_then(|rows| rows.into_iter().find(|row| row.id == version))
        .and_then(|row| row.named().map(str::to_string));
    match named {
        Some(name) => format!("Before restoring “{name}”"),
        None => "Before restoring".to_string(),
    }
}

/// What this console can offer to open.
///
/// Three answers in one, because a welcome screen shows all three at once: where new
/// shows go, what was opened recently, and what is actually in that directory. A
/// recent entry whose folder has gone is listed with `missing` set rather than
/// dropped — a show on a stick that is not plugged in is exactly the row somebody is
/// looking for.
pub async fn list_shows(shows: &crate::ShowsHandle) -> Result<Value, String> {
    use crate::infra::showfile::{bundle, recent};

    let mut recents = Vec::new();
    for entry in recent::load().shows {
        let summary = match bundle::Bundle::open(&entry.path) {
            Ok(bundle) => serde_json::to_value(bundle.summary().await).unwrap_or_default(),
            Err(_) => serde_json::json!({
                "path": entry.path,
                "name": bundle::name_from_path(&entry.path),
                "missing": true,
            }),
        };
        let mut summary = summary;
        summary["lastOpened"] = serde_json::json!(entry.last_opened);
        recents.push(summary);
    }

    let mut in_dir = Vec::new();
    if let Some(dir) = &shows.shows_dir {
        for bundle in bundle::bundles_in(dir) {
            in_dir.push(serde_json::to_value(bundle.summary().await).unwrap_or_default());
        }
    }

    Ok(serde_json::json!({
        // The four demos, from the same table `--demo` reads, so a card on the
        // welcome screen cannot offer one the console does not have.
        "demos": crate::demo::ALL
            .iter()
            .map(|demo| serde_json::json!({
                "id": demo.id(),
                "title": demo.title(),
                "blurb": demo.blurb(),
            }))
            .collect::<Vec<_>>(),
        "showsDir": shows.shows_dir,
        "open": shows.bundle.as_ref().map(|b| b.path()),
        "recent": recents,
        "inDir": in_dir,
    }))
}

/// What a fixture's parameters are putting out, right now.
///
/// One parameter when named, and every one an operator can set when not — the two
/// shapes `__home` and `__set_home` take, so the trio reads as a trio. The answer is a
/// map keyed by parameter key, which is the same key `home_values` and `live_fades`
/// use, so a caller can line it up with anything else it has read.
///
/// Evaluated at one instant for the whole fixture. Two readings of the clock would put
/// a mover's pan and its tilt a millisecond apart, which is a pose nothing ever struck.
async fn what_is_it_doing(
    engine: &EngineHandle,
    fixture_id: uuid::Uuid,
    kind: Option<ParameterKind>,
) -> Result<Value, String> {
    let row = engine
        .get(vec![PathSegment::Key("fixtures".into()), PathSegment::Id(fixture_id)])
        .await
        .map_err(|_| format!("there is no fixture {fixture_id}"))?;
    if row.is_null() {
        return Err(format!("there is no fixture {fixture_id}"));
    }
    let fixture: Fixture = serde_json::from_value(row)
        .map_err(|e| format!("fixture {fixture_id} does not parse: {e}"))?;

    let types = engine
        .get(vec![PathSegment::Key("fixture_types".into())])
        .await
        .map_err(|e| format!("cannot read the fixture types: {e}"))?;
    let types: Vec<FixtureType> = serde_json::from_value(types).unwrap_or_default();
    let fixture_type = types.iter().find(|t| t.id == fixture.fixture_type_id);

    let entries = engine
        .get(vec![PathSegment::Key("programmer_values".into())])
        .await
        .map_err(|e| format!("cannot read the programmer: {e}"))?;
    let entries: Vec<pult_schema::types::programmer::ProgrammerValue> =
        serde_json::from_value(entries).unwrap_or_default();
    let held = HeldByProgrammer::of(&entries);
    let sequences = engine
        .get(vec![PathSegment::Key("sequences".into())])
        .await
        .map_err(|e| format!("cannot read the sequences: {e}"))?;
    let sequences: Vec<pult_schema::types::sequence::Sequence> =
        serde_json::from_value(sequences).unwrap_or_default();
    let masters = pult_schema::types::sequence::Masters::of(&sequences);

    let keys: Vec<String> = match (&kind, fixture_type) {
        (Some(kind), _) => vec![parameter_key(kind)],
        (None, Some(fixture_type)) => {
            fixture_type.controls().iter().map(|(kind, _)| parameter_key(kind)).collect()
        }
        // Patched to a type this station has not got. Whatever the fixture overrides
        // for itself is still the truth about it, so that is what it can answer with.
        (None, None) => fixture.home_values.keys().cloned().collect(),
    };

    let now_ms = pult_schema::types::sequence::now_ms();
    let values: std::collections::HashMap<String, pult_schema::types::fixture::ParameterValue> =
        keys.into_iter()
            .filter_map(|key| {
                let driving = driving(&fixture, fixture_type, held.get(fixture_id, &key), &key, &masters);
                pult_render::value_at(&driving, now_ms).map(|value| (key, value))
            })
            .collect();
    serde_json::to_value(values).map_err(|e| e.to_string())
}

/// Answer a timeline's pending take: merge what was chosen, drop the rest, and clear
/// the review — `None` for a list means all of it, an empty list none.
///
/// Every write under the caller's name and one gesture, which is what makes the answer
/// one Ctrl-Z: the rows, the flashes, the cues and the review come back together, so undoing a
/// Merge puts the take up for review again rather than losing it.
async fn answer_a_take(
    deps: &LocalRpcDeps,
    timeline_id: uuid::Uuid,
    takes: Option<Vec<(uuid::Uuid, String)>>,
    flashes: Option<Vec<(uuid::Uuid, String)>>,
    events: Option<Vec<uuid::Uuid>>,
) -> Result<Value, String> {
    use pult_schema::{lifecycle::Lifecycle, types::timeline::Timeline};

    let path = vec![PathSegment::Key("timelines".into()), PathSegment::Id(timeline_id)];
    let timeline: Timeline = deps
        .engine
        .get(path.clone())
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or_else(|| "no such timeline".to_string())?;
    let pending = timeline.pending.clone().ok_or_else(|| "nothing is waiting to be merged".to_string())?;
    let chosen = |list: &Option<Vec<(uuid::Uuid, String)>>, fixture_id: uuid::Uuid, key: &str| {
        list.as_ref().is_none_or(|keys| keys.iter().any(|(id, k)| *id == fixture_id && k == key))
    };
    let mut rows = timeline.tracks.clone();
    let mut kept = timeline.all_flashes().to_vec();
    pending.merge(&mut rows, &mut kept, |id, key| chosen(&takes, id, key), |id, key| chosen(&flashes, id, key));
    let mut cues = timeline.events.clone();
    pending.merge_events(&mut cues, |id| events.as_ref().is_none_or(|ids| ids.contains(&id)));

    let user_id = asking_user(deps);
    let gesture = Some(uuid::Uuid::new_v4());
    let field = |name: &str| {
        let mut path = path.clone();
        path.push(PathSegment::Key(name.into()));
        path
    };
    if rows != timeline.tracks {
        let value = serde_json::to_value(&rows).map_err(|e| e.to_string())?;
        deps.engine.set_as(user_id, gesture, field("tracks"), Lifecycle::Persisted, value).await.map_err(|e| e.to_string())?;
    }
    if kept.len() != timeline.all_flashes().len() {
        let value = serde_json::to_value(&kept).map_err(|e| e.to_string())?;
        deps.engine.set_as(user_id, gesture, field("flashes"), Lifecycle::Persisted, value).await.map_err(|e| e.to_string())?;
    }
    if cues.len() != timeline.events.len() {
        let value = serde_json::to_value(&cues).map_err(|e| e.to_string())?;
        deps.engine.set_as(user_id, gesture, field("events"), Lifecycle::Persisted, value).await.map_err(|e| e.to_string())?;
    }
    deps.engine
        .set_as(user_id, gesture, field("pending"), Lifecycle::Persisted, Value::Null)
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "rows": rows.len(), "flashes": kept.len(), "events": cues.len() }))
}

/// Read a wire and put what is on it into the programmer.
///
/// The act a grab is: decode now, write once, under the caller's authorship and in one
/// gesture, so an operator's Ctrl-Z takes the whole grab back rather than one parameter
/// of it.
///
/// It refuses **by name** when this station is not listening on that input, because the
/// alternative — asking the station that is — would mean putting a universe on the link
/// carrying the show every time somebody pressed a button. The page can read `node_id`
/// off the row and say which console to open.
async fn grab_into_the_programmer(
    deps: &LocalRpcDeps,
    input_id: Option<uuid::Uuid>,
    fixture_ids: Vec<uuid::Uuid>,
) -> Result<Value, String> {
    use pult_schema::{
        lifecycle::Lifecycle,
        types::programmer::{programmer_entry_id, ProgrammerValue},
    };

    if fixture_ids.is_empty() {
        return Err("nothing is selected".to_string());
    }
    let input = deps
        .input
        .as_ref()
        .ok_or_else(|| "this station has no inputs; it was started without them".to_string())?;
    let values = input.grab(input_id, fixture_ids).await?;
    if values.is_empty() {
        return Err("nothing on that input reaches those fixtures".to_string());
    }

    // The kind, not only the key: a programmer entry carries the `ParameterKind`, and
    // the wire only ever said which key it was. The type is what knows the difference.
    let rig = deps.engine.get(vec![PathSegment::Key("fixtures".into())]).await;
    let fixtures: Vec<Fixture> = rig.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let types = deps.engine.get(vec![PathSegment::Key("fixture_types".into())]).await;
    let types: Vec<FixtureType> =
        types.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let held = deps.engine.get(vec![PathSegment::Key("programmer_values".into())]).await;
    let held: Vec<ProgrammerValue> =
        held.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();

    let user_id = asking_user(deps);
    let gesture = uuid::Uuid::new_v4();
    let mut written = 0usize;

    for (fixture_id, key, value) in values {
        let Some(kind) = fixtures
            .iter()
            .find(|fixture| fixture.id == fixture_id)
            .and_then(|fixture| types.iter().find(|t| t.id == fixture.fixture_type_id))
            .and_then(|fixture_type| {
                fixture_type.controls().into_iter().find(|(kind, _)| parameter_key(kind) == key).map(|(kind, _)| kind)
            })
        else {
            continue;
        };
        let Ok(entry_id) = programmer_entry_id(&fixture_id.to_string(), &key).parse::<uuid::Uuid>()
        else {
            continue;
        };
        let Ok(value) = serde_json::to_value(&value) else { continue };

        // A parked value is the operator's decision and a grab is not allowed to
        // overrule it, exactly as sending a fixture home is not.
        if held.iter().any(|entry| entry.id == entry_id && entry.locked) {
            continue;
        }
        // The id is derived from the fixture and the key, so a grab over a parameter
        // an operator already has hold of moves that entry rather than making a
        // second one holding the same thing.
        let holding = held.iter().any(|entry| entry.id == entry_id);
        let (path, body) = if holding {
            (
                vec![
                    PathSegment::Key("programmer_values".into()),
                    PathSegment::Id(entry_id),
                    PathSegment::Key("value".into()),
                ],
                value,
            )
        } else {
            (
                vec![
                    PathSegment::Key("programmer_values".into()),
                    PathSegment::Key("__create".into()),
                ],
                serde_json::json!({
                    "id": entry_id,
                    "fixture_id": fixture_id,
                    "parameter_kind": kind,
                    "value": value,
                    "locked": false,
                }),
            )
        };
        if deps
            .engine
            .set_as(user_id, Some(gesture), path, Lifecycle::Synced, body)
            .await
            .is_ok()
        {
            written += 1;
        }
    }
    Ok(serde_json::json!({ "written": written }))
}

/// The fixtures a group picks out of the rig as it is now.
///
/// Reads the rig every time and caches nothing, which is what makes a group survive
/// a re-patch: a fixture hung this afternoon is in this morning's group without
/// anybody re-saving anything. The evaluator is `pult-schema`'s, the same one the
/// browser runs, held to it by `testdata/selection-queries.json`.
///
/// A group that is not there is an error rather than an empty answer, so a command
/// line can tell "you have no such group" from "that group is currently empty".
async fn resolve_group(engine: &EngineHandle, group_id: uuid::Uuid) -> Result<Value, String> {
    let row = engine
        .get(vec![PathSegment::Key("groups".into()), PathSegment::Id(group_id)])
        .await
        .map_err(|_| format!("there is no group {group_id}"))?;
    if row.is_null() {
        return Err(format!("there is no group {group_id}"));
    }
    let group: Group =
        serde_json::from_value(row).map_err(|e| format!("group {group_id} does not parse: {e}"))?;

    let rig = engine
        .get(vec![PathSegment::Key("fixtures".into())])
        .await
        .map_err(|e| format!("cannot read the rig: {e}"))?;
    let fixtures: Vec<Fixture> = serde_json::from_value(rig).unwrap_or_default();

    // A light on a truss is where the truss put it, so the objects it may hang off
    // are part of the question.
    let drawing = engine
        .get(vec![PathSegment::Key("scene_objects".into())])
        .await
        .map_err(|e| format!("cannot read the rig: {e}"))?;
    let objects: Vec<SceneObject> = serde_json::from_value(drawing).unwrap_or_default();

    let ids = evaluate(&group.query, &fixtures, None, &objects);
    serde_json::to_value(ids).map_err(|e| e.to_string())
}

/// Where a value of `kind` set on these fixtures, cut to these cells, lands: the
/// selection's elements and then [`pult_schema::types::group::targets`].
async fn selection_targets(
    engine: &EngineHandle,
    ids: &[uuid::Uuid],
    cells: &[pult_schema::types::group::CellSpan],
    kind: &pult_schema::types::fixture::ParameterKind,
) -> Result<Value, String> {
    use pult_schema::types::fixture::FixtureType;
    use pult_schema::types::group::{elements, targets};
    let read = |table: &'static str| async move {
        engine
            .get(vec![PathSegment::Key(table.into())])
            .await
            .map_err(|e| format!("cannot read the rig: {e}"))
    };
    let fixtures: Vec<Fixture> = serde_json::from_value(read("fixtures").await?).unwrap_or_default();
    let types: Vec<FixtureType> =
        serde_json::from_value(read("fixture_types").await?).unwrap_or_default();
    let types: std::collections::HashMap<uuid::Uuid, FixtureType> =
        types.into_iter().map(|t| (t.id, t)).collect();
    let type_of_fixture: std::collections::HashMap<uuid::Uuid, &FixtureType> = fixtures
        .iter()
        .filter_map(|f| types.get(&f.fixture_type_id).map(|t| (f.id, t)))
        .collect();
    // Cut, the cells it picks; not cut, each fixture whole — so a bar selected whole
    // takes its master and a strip selected whole every cell, as `targets` decides.
    // The browser's `targetElements` is the same.
    let picked = if cells.is_empty() {
        ids.iter().map(|id| pult_schema::types::group::Element { fixture: *id, cell: None }).collect()
    } else {
        elements(ids, cells, |id| {
            type_of_fixture.get(id).filter(|t| t.has_cells()).map_or(0, |t| t.cells)
        })
    };
    let address: std::collections::HashMap<uuid::Uuid, &pult_schema::types::fixture::FixtureAddress> =
        fixtures.iter().map(|f| (f.id, &f.address)).collect();
    let landed = targets(
        kind,
        &picked,
        |id| type_of_fixture.get(id).copied(),
        |id, key| match (type_of_fixture.get(id), address.get(id)) {
            (Some(fixture_type), Some(address)) => fixture_type.reaches(address, key),
            _ => true,
        },
    );
    serde_json::to_value(landed).map_err(|e| e.to_string())
}

/// Build one paperwork table out of what the station holds.
///
/// Five reads and a pure function. The arithmetic itself is
/// [`pult_schema::types::paperwork::table`] — deliberately in the schema crate rather
/// than here, so that the browser building a PDF, a plugin asking what a truss weighs
/// and this RPC are all the same implementation and cannot come to different totals.
async fn paperwork_table(
    engine: &EngineHandle,
    request: pult_schema::types::paperwork::TableRequest,
) -> Result<Value, String> {
    async fn rows<T: serde::de::DeserializeOwned>(
        engine: &EngineHandle,
        collection: &str,
    ) -> Result<Vec<T>, String> {
        let value = engine
            .get(vec![PathSegment::Key(collection.into())])
            .await
            .map_err(|e| format!("cannot read {collection}: {e}"))?;
        Ok(serde_json::from_value(value).unwrap_or_default())
    }

    let fixtures: Vec<Fixture> = rows(engine, "fixtures").await?;
    let fixture_types: Vec<FixtureType> = rows(engine, "fixture_types").await?;
    let scene_objects: Vec<SceneObject> = rows(engine, "scene_objects").await?;
    let layers: Vec<pult_schema::types::scene::Layer> = rows(engine, "layers").await?;
    let classes: Vec<pult_schema::types::scene::SceneClass> = rows(engine, "classes").await?;

    let rig = pult_schema::types::paperwork::Rig {
        fixtures: &fixtures,
        fixture_types: &fixture_types,
        scene_objects: &scene_objects,
        layers: &layers,
        classes: &classes,
    };
    let table = pult_schema::types::paperwork::table(&request, &rig);
    serde_json::to_value(table).map_err(|e| e.to_string())
}

/// What the rig would be doing with a set of cues running.
///
/// The state is *worked out*, never taken. Each cue's stack is tracked through with
/// [`pult_schema::types::cue::tracked_through`] — the same function playback's own Go
/// uses, so a sheet cannot draw a different answer from the one the console would
/// produce — and each resulting capture is turned into the fade or the effect it would
/// have started, anchored so that at the moment of evaluation it has been running for
/// exactly the time the viewport asked for.
///
/// **The time is the point.** A fade sampled at zero is the state *before* the cue, so a
/// five-second fade up from black renders black; an effect at zero has every head at the
/// same phase. Both are the least useful frame of the cue.
///
/// Fades run from the parameter's **home** value rather than from what the rig happens
/// to be doing now. A picture on a sheet documents a cue taken on a rig at rest, and
/// anchoring it to whatever the console is showing at the moment somebody pressed Export
/// would make the same sheet come out differently every time.
async fn paperwork_cue_values(
    engine: &EngineHandle,
    shots: Vec<pult_schema::types::paperwork::CueShot>,
) -> Result<Value, String> {
    use pult_render::{driving::Driving, effect::RunningFade};
    use pult_schema::types::{
        cue::{tracked_through, Cue},
        effect::EffectSource,
        Sequence,
    };

    async fn rows<T: serde::de::DeserializeOwned>(
        engine: &EngineHandle,
        collection: &str,
    ) -> Result<Vec<T>, String> {
        let value = engine
            .get(vec![PathSegment::Key(collection.into())])
            .await
            .map_err(|e| format!("cannot read {collection}: {e}"))?;
        Ok(serde_json::from_value(value).unwrap_or_default())
    }

    let cues: Vec<Cue> = rows(engine, "cues").await?;
    let sequences: Vec<Sequence> = rows(engine, "sequences").await?;
    let fixtures: Vec<Fixture> = rows(engine, "fixtures").await?;
    let types: Vec<FixtureType> = rows(engine, "fixture_types").await?;
    let masters: Vec<pult_schema::types::SpeedMaster> = rows(engine, "speed_masters").await?;
    let presets: Vec<pult_schema::types::Preset> = rows(engine, "presets").await?;
    let show: Option<pult_schema::types::Show> = engine
        .get(vec![PathSegment::Key("show".into())])
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok());
    let curves = show.map(|s| s.fade_curves).unwrap_or_default();

    let by_id: std::collections::HashMap<uuid::Uuid, &Cue> =
        cues.iter().map(|cue| (cue.id, cue)).collect();
    let fixtures_by_id: std::collections::HashMap<uuid::Uuid, &Fixture> =
        fixtures.iter().map(|f| (f.id, f)).collect();
    let types_by_id: std::collections::HashMap<uuid::Uuid, &FixtureType> =
        types.iter().map(|t| (t.id, t)).collect();
    // A capture may be a reference rather than a value, and it resolves in one place —
    // so a sheet draws what a Go would put on stage rather than the literal the cue
    // happened to be stored as.
    let preset_index = pult_schema::types::preset_index(&presets);

    // A round number well past any anchor, so the arithmetic below never underflows
    // when a shot asks for a moment before the cue was taken.
    const NOW: u64 = 1_000_000_000;
    let mut out = serde_json::Map::new();

    for shot in &shots {
        let Some(cue) = by_id.get(&shot.cue) else { continue };
        // The stack up to this cue, which needs the sequence holding it.
        let Some(sequence) = sequences.iter().find(|s| s.cue_ids.contains(&shot.cue)) else {
            continue;
        };
        let index = sequence.cue_ids.iter().position(|id| *id == shot.cue).unwrap_or(0);
        let through: Vec<&uuid::Uuid> = sequence.cue_ids.iter().take(index + 1).collect();
        let tracked = tracked_through(through, |id| by_id.get(id).copied());

        // The cue was taken `at_ms` ago.
        let anchor = NOW.saturating_sub(shot.at_ms as u64);

        for (owner, capture) in tracked {
            let key = parameter_key(&capture.parameter_kind);
            let Some(fixture) = fixtures_by_id.get(&capture.fixture_id) else { continue };
            let kind = types_by_id.get(&fixture.fixture_type_id).copied();
            let home = pult_schema::types::home_value_by_key(fixture, kind, &key);

            let to = capture.value_in(&preset_index);
            let value = if let Some(spec) = &capture.effect {
                let running = crate::model::effects::resolve(
                    spec,
                    &masters,
                    anchor,
                    EffectSource::Cue(owner.id),
                );
                pult_render::driving::value_at(
                    &Driving { effect: Some(&running), ..Default::default() },
                    NOW,
                )
            } else {
                // From home rather than from now: see the note above.
                let from = home.clone().unwrap_or_else(|| to.clone());
                let up = if capture.fade_in_ms > 0 { capture.fade_in_ms } else { cue.fade_in_ms };
                let running = RunningFade {
                    from,
                    to: to.clone(),
                    t0: anchor + capture.delay_in_ms as u64,
                    duration_ms: up,
                    easing: curves.resolve(capture.easing, cue.easing, &key),
                    cue_id: owner.id,
                };
                pult_render::driving::value_at(
                    &Driving { fade: Some(&running), ..Default::default() },
                    NOW,
                )
            };

            if let Some(value) = value {
                if let Ok(value) = serde_json::to_value(&value) {
                    // Keyed the way `readingOf` in the browser keys a snapshot, so the
                    // answer drops straight into a `Showing`.
                    out.insert(format!("{}/{}", capture.fixture_id, key), value);
                }
            }
        }
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every listed RPC must reach a real arm in `dispatch`, and every arm must
    /// be listed: the table is what a command line offers, the match is what
    /// happens, and they must not drift apart. "Missing serial" and the like
    /// prove the arm was entered; only "unknown method" proves it was not.
    #[tokio::test]
    async fn the_table_and_the_dispatcher_agree() {
        let (session_tx, _session_rx) = tokio::sync::mpsc::channel(1);
        let (device_tx, _device_rx) = tokio::sync::mpsc::channel(1);
        let (engine_tx, _engine_rx) = tokio::sync::mpsc::channel(1);
        let deps = LocalRpcDeps {
            session: SessionHandle(session_tx),
            devices: DeviceHandle(device_tx),
            engine: EngineHandle(engine_tx),
            // Nothing here asks about the log, and a station without one is a
            // real configuration rather than a test fiction.
            log: None,
            log_watchers: Default::default(),
            sync: None,
            caller: None,
            clients: None,
            node_id: pult_schema::events::operation::NodeId::new(),
            viewers: Default::default(),
            input: None,
            audio: None,
            ws_registry: None,
            shows: crate::ShowsHandle::detached(),
            xchange: None,
            wing: None,
        };
        for meta in LOCAL_RPCS {
            assert!(is_local_rpc(meta.method), "{} is not routed here", meta.method);
            // Empty args, so every arm answers fast — a validation error, or a
            // shrug at the closed channels above — instead of waiting.
            if let Err(err) = dispatch(meta.method, Value::Null, &deps).await {
                assert!(
                    !err.starts_with("unknown method"),
                    "{} is listed but not dispatched",
                    meta.method
                );
            }
            let parsed: serde_json::Value =
                serde_json::from_str(meta.args_schema).expect("args_schema is JSON");
            assert!(parsed.is_array(), "{} args_schema is not an array", meta.method);
        }
    }

    /// A plugin, or anything else that can call but cannot read the rig, asking what
    /// a light is doing mid-fade — and getting the value for the moment it asked.
    ///
    /// This is what replaced reading a stored map. Nothing keeps the number any more,
    /// so the caller that could once have read it has to be able to ask instead, and
    /// asking must not write anything down: it is a read, so it is an RPC rather than
    /// a command.
    #[tokio::test]
    async fn asking_what_a_parameter_is_doing_answers_for_the_moment_it_was_asked() {
        use pult_schema::{
            lifecycle::Lifecycle,
            types::fixture::{
                FixtureType, ParameterDefinition,
                ParameterKind, ParameterValue,
            },
        };

        let pool =
            std::sync::Arc::new(crate::infra::showfile::open_in_memory().await.expect("showfile"));
        let (engine, handle, _broadcast) = crate::engine::ShowEngine::new(
            pult_schema::events::operation::NodeId(uuid::Uuid::new_v4()),
            pool,
            None,
        );
        tokio::spawn(engine.run());

        let fixture_type = FixtureType {
            id: uuid::Uuid::new_v4(),
            name: "Dimmer".into(),
            manufacturer: "Acme".into(),
            channel_count: 1,
            parameters: vec![ParameterDefinition::new(
                ParameterKind::Intensity,
                ParameterValue::Float(0.0),
            )],
            ..FixtureType::default()
        };
        let fixture_id = uuid::Uuid::new_v4();
        let create = |table: &str| {
            vec![PathSegment::Key(table.into()), PathSegment::Key("__create".into())]
        };
        handle
            .set(
                create("fixture_types"),
                Lifecycle::Persisted,
                serde_json::to_value(&fixture_type).unwrap(),
            )
            .await
            .unwrap();
        handle
            .set(
                create("fixtures"),
                Lifecycle::Persisted,
                serde_json::json!({
                    "id": fixture_id,
                    "name": "Spot",
                    "fixture_type_id": fixture_type.id,
                    "address": { "Dmx": { "mode": "Default", "breaks": [{ "universe": 1, "address": 1 }] } },
                    "position": null,
                }),
            )
            .await
            .unwrap();

        // Nothing driving it: it answers with where it rests.
        let deps = LocalRpcDeps {
            session: SessionHandle(tokio::sync::mpsc::channel(1).0),
            devices: DeviceHandle(tokio::sync::mpsc::channel(1).0),
            engine: handle.clone(),
            // Nothing here asks about the log, and a station without one is a
            // real configuration rather than a test fiction.
            log: None,
            log_watchers: Default::default(),
            sync: None,
            caller: None,
            clients: None,
            node_id: pult_schema::events::operation::NodeId::new(),
            viewers: Default::default(),
            input: None,
            audio: None,
            ws_registry: None,
            shows: crate::ShowsHandle::detached(),
            xchange: None,
            wing: None,
        };
        let ask = || {
            dispatch("parameter.value", serde_json::json!({ "fixtureId": fixture_id }), &deps)
        };
        assert_eq!(ask().await.unwrap()["Intensity"]["value"], 0.0, "where it rests");

        // A four second fade, one second old.
        let now = pult_schema::types::sequence::now_ms();
        handle
            .set(
                vec![
                    PathSegment::Key("fixtures".into()),
                    PathSegment::Id(fixture_id),
                    PathSegment::Key("live_fades".into()),
                ],
                Lifecycle::Local,
                serde_json::json!({
                    "Intensity": {
                        "from": ParameterValue::Float(0.0),
                        "to": ParameterValue::Float(1.0),
                        "t0": now - 1_000,
                        "duration_ms": 4_000,
                        "easing": "Linear",
                        "cue_id": uuid::Uuid::nil(),
                    }
                }),
            )
            .await
            .unwrap();

        let first = ask().await.unwrap()["Intensity"]["value"].as_f64().unwrap();
        assert!(first > 0.2 && first < 0.35, "a quarter of the way up: {first}");

        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let later = ask().await.unwrap()["Intensity"]["value"].as_f64().unwrap();
        assert!(later > first, "and it has moved on since, with nothing written: {later}");
    }
}
