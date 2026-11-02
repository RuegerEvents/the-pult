//! An OctoBanger prop, moved onto this console.
//!
//! `pult-octobanger` reads the `.OBC`; this is what one becomes in a show, and it is
//! built out of what the console already had rather than anything new — because the
//! Heinz Horror Garten's hand-built scares were already exactly this shape (task 85):
//!
//! - **The scare is a timeline.** A recording with a row per channel that is used —
//!   each the channel's change points, stepwise, which is what an OctoBanger does at
//!   twenty frames a second — and the scare's sound as a clip, cut where the sequence
//!   ends, because the firmware stops the MP3 board there (`PlayAmbient` sends a stop
//!   first). It runs on past the sequence for the prop's reset delay with nothing left
//!   on it, which is the whole of the lockout: the trigger plays it with *Ignore if
//!   running*, so a trigger while it is cold does nothing at all.
//! - **The ambience is a timeline that goes round**, where the prop had one: the scare
//!   stops it at zero and plays it again where the sequence ends, which is what the
//!   firmware did with the MP3 board.
//! - **The trigger is a flow**: the input's edge, and a button beside it, both playing
//!   the scare. The button is there because the first thing anybody does with an
//!   imported prop is press it to see whether the right relays clatter.
//!
//! **The controller's name is its identity.** Every row's id is a v5 of the name and
//! what the row is, so importing the same prop again — with a channel moved onto a
//! node that has since been adopted, say — rewrites what the last import wrote rather
//! than doubling it. Which is the rule MVR follows with the file's own uuids, and what
//! an `.OBC`, which has none, has instead.
//!
//! **A channel nobody assigned still plays**, onto a placeholder fixture with a type
//! of no channels at all — so nothing reaches any wire and nothing is lost, and the
//! recording is there to repoint by importing again. Unless it is *discarded*, which
//! is for a device that is not coming over at all. And what the Arduino needed and a
//! node does not — which pins, the active-low relay boards, the MP3 board's start-up
//! delay — is said in the report rather than carried, because none of it is the show.

use std::collections::{BTreeMap, BTreeSet};

use pult_octobanger::{Controller, MediaKind, Project};
use pult_schema::types::fixture::{
    parameter_key, Fixture, FixtureType, FixtureTypeSource, ParameterDefinition, ParameterDirection, ParameterKind,
    ParameterValue,
};
use pult_schema::types::flow::{
    Flow, FlowEdge, FlowNode, FlowNodeKind, IfRunning, TriggerAction, TriggerCondition, TriggerSource,
};
use pult_schema::types::timeline::{
    split_row, AtEnd, AudioClip, AudioDestination, Timeline, TimelineAction, TimelineEnd, TimelineEvent,
    TimelineSource, TimelineTrack,
};
use pult_schema::types::zone::SpeakerZone;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::apply::{ImportPlan, ImportReport};
use crate::engine::EngineHandle;
use crate::infra::assets::{self, AssetStore};
use crate::infra::audio::decode::{survey, Survey};

/// What every id an import makes is a v5 in.
const NAMESPACE: Uuid = Uuid::from_u128(0x6f63_746f_6261_6e67_6572_0000_0000_0001);

/// The id of whatever `what` is, for the controller called `name`.
fn id(name: &str, what: &str) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("{name}\u{0}{what}").as_bytes())
}

/// The type every placeholder is: one switch on a port, so no channels at all.
fn placeholder_type_id() -> Uuid {
    Uuid::new_v5(&NAMESPACE, b"placeholder output")
}

const SWITCH: ParameterKind = ParameterKind::Switch(0);

// ── What the dialog is told ───────────────────────────────────────────────────

/// An upload, described for somebody about to map it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    pub controllers: Vec<Described>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Described {
    /// Where in the upload it was; what the mapping names it by.
    pub path: String,
    pub controller: Controller,
    pub channels: Vec<DescribedChannel>,
    pub trigger_pin: Option<&'static str>,
    pub sounds: Vec<DescribedSound>,
    /// What this prop did that nothing here will, said before anybody imports it.
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribedChannel {
    /// 1 to 8.
    pub number: u8,
    pub used: bool,
    pub spans: Vec<(u32, u32)>,
    pub pin: Option<&'static str>,
    /// Whether this channel's row in this show already exists — an earlier import
    /// left a placeholder for it that is still the one it points at.
    pub placeholder: Option<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribedSound {
    /// `scare` or `ambient`.
    pub role: &'static str,
    pub name: String,
    /// Whether the upload carried it.
    pub found: bool,
    pub duration_ms: Option<u64>,
    pub silent: Option<bool>,
    /// Why it could not be read, where it could not.
    pub error: Option<String>,
}

/// Read an upload and say what is in it. Writes nothing.
pub async fn describe(bytes: &[u8], existing: &Existing<'_>) -> Result<Description, String> {
    let project = Project::read(bytes).map_err(|e| e.to_string())?;
    let mut controllers = Vec::new();
    for found in &project.controllers {
        let controller = &found.controller;
        let mut sounds = Vec::new();
        for (role, name) in wanted_sounds(controller) {
            let mut sound = DescribedSound { role, name: name.clone(), found: false, duration_ms: None, silent: None, error: None };
            if let Some(file) = project.file(&found.path, &name).map_err(|e| e.to_string())? {
                sound.found = true;
                match surveyed(file).await {
                    Ok(survey) => {
                        sound.duration_ms = Some(survey.duration_ms);
                        sound.silent = Some(survey.silent());
                    }
                    Err(error) => sound.error = Some(error),
                }
            }
            sounds.push(sound);
        }
        let pins = controller.pins();
        let channels = (1..=8u8)
            .map(|number| DescribedChannel {
                number,
                used: controller.used(number as usize),
                spans: controller.spans(number as usize),
                pin: pins.map(|p| p[number as usize - 1]),
                placeholder: Some(placeholder_id(&controller.name, number))
                    .filter(|id| existing.fixtures.iter().any(|f| f.id == *id)),
            })
            .collect();
        controllers.push(Described {
            path: found.path.clone(),
            controller: controller.clone(),
            channels,
            trigger_pin: controller.trigger_pin(),
            sounds,
            notes: notes(controller),
        });
    }
    Ok(Description { controllers })
}

/// The sounds a controller plays, by the role each has and the name it gives it.
fn wanted_sounds(controller: &Controller) -> Vec<(&'static str, String)> {
    let media = &controller.media;
    let mut out = Vec::new();
    if matches!(media.kind, MediaKind::Mp3ScareOnly | MediaKind::Mp3AmbientScare) {
        if let Some(name) = &media.scare {
            out.push(("scare", name.clone()));
        }
    }
    if media.kind == MediaKind::Mp3AmbientScare {
        if let Some(name) = &media.ambient {
            out.push(("ambient", name.clone()));
        }
    }
    out
}

/// What the prop did that will not happen here, in the words the operator reads.
fn notes(controller: &Controller) -> Vec<String> {
    let mut notes = Vec::new();
    for (index, channel) in controller.channels.iter().enumerate() {
        if channel.active_low && controller.used(index + 1) {
            notes.push(format!(
                "{} was Active Low on the Arduino. Here the output is on when the channel is on: check the relay the first time it fires.",
                channel.name
            ));
        }
    }
    if controller.boot_delay_s > 0 {
        notes.push(format!(
            "The prop waited {} s after power-up before it took a trigger. Nothing here does that.",
            controller.boot_delay_s
        ));
    }
    if controller.media.delay_cs > 0 {
        notes.push(format!(
            "The prop held its outputs back {} ms after starting the sound, for the MP3 board's slow start. Here sound and outputs start together.",
            controller.media.delay_cs as u32 * 10
        ));
    }
    match controller.media.kind {
        MediaKind::Video => notes.push("The prop started a video player. Nothing of that is imported.".into()),
        MediaKind::Other(n) => notes.push(format!("The prop's media player is of a kind ({n}) this console does not know.")),
        _ => {}
    }
    notes
}

async fn surveyed(bytes: Vec<u8>) -> Result<Survey, String> {
    tokio::task::spawn_blocking(move || survey(bytes)).await.map_err(|e| format!("the decoder stopped: {e}"))?
}

// ── What the dialog answers ───────────────────────────────────────────────────

/// Which controller of an upload, and where each part of it goes.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    /// The controller's path in the upload, as [`Described::path`] gave it.
    #[serde(default)]
    pub controller: String,
    /// A channel left out, or given no fixture, goes to a placeholder if it is used,
    /// unless it says to be discarded.
    #[serde(default)]
    pub channels: Vec<ChannelTarget>,
    /// The input that sets the scare off. `None` leaves the flow with its button only.
    #[serde(default)]
    pub trigger: Option<Target>,
    /// Which edge of the trigger: the file's resting level says, and a sensor wired
    /// differently on the node is the reason the dialog lets this be changed.
    #[serde(default)]
    pub edge: Option<TriggerCondition>,
    /// Where the sound plays. `None` imports no sound at all.
    #[serde(default)]
    pub destination: Option<AudioDestination>,
    /// Whether an ambient loop becomes a timeline.
    #[serde(default)]
    pub ambience: bool,
    /// Sound files the upload did not carry, already in the asset store, by the name
    /// the controller gives them.
    #[serde(default)]
    pub audio: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTarget {
    pub channel: u8,
    pub fixture_id: Option<Uuid>,
    pub key: Option<String>,
    /// Not imported at all: no placeholder, no row on the recording. For a channel
    /// whose device is not coming over — a strobe the new rig does not have.
    #[serde(default)]
    pub discard: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub fixture_id: Uuid,
    pub key: String,
}

/// A sound the plan can put on a timeline.
#[derive(Debug, Clone)]
pub struct Sound {
    pub sha: String,
    /// The bytes, where they still have to be stored; `None` where they already are.
    pub bytes: Option<Vec<u8>>,
    pub duration_ms: u64,
}

/// What the show already holds that an import is matched against.
pub struct Existing<'a> {
    pub fixtures: &'a [Fixture],
    pub fixture_types: &'a [FixtureType],
    pub timelines: &'a [Timeline],
    pub flows: &'a [Flow],
    pub flow_nodes: &'a [FlowNode],
    pub flow_edges: &'a [FlowEdge],
    pub speaker_zones: &'a [SpeakerZone],
}

fn placeholder_id(name: &str, channel: u8) -> Uuid {
    id(name, &format!("placeholder {channel}"))
}

/// Everything one controller becomes, worked out before any of it is written.
///
/// Pure: the show as it is, the controller, the mapping and the sounds in; the plan
/// out, or the reason there is none. So a mapping that points at a fixture that is not
/// there is refused here, with nothing stored.
pub fn plan_import(
    controller: &Controller,
    mapping: &Mapping,
    sounds: &BTreeMap<String, Sound>,
    existing: &Existing,
) -> Result<ImportPlan, String> {
    let name = controller.name.trim();
    if name.is_empty() {
        return Err("the controller has no name, and its name is what an import is matched by".into());
    }
    let mut plan = ImportPlan::default();

    // ── Where each channel goes ──
    let mut targets: BTreeMap<u8, (Uuid, String)> = BTreeMap::new();
    let mut taken: BTreeMap<(Uuid, String), u8> = BTreeMap::new();
    let mut discarded = BTreeSet::new();
    for each in &mapping.channels {
        if !(1..=8).contains(&each.channel) {
            return Err(format!("there is no channel {}", each.channel));
        }
        if each.discard {
            discarded.insert(each.channel);
            continue;
        }
        let (Some(fixture_id), Some(key)) = (each.fixture_id, each.key.clone()) else { continue };
        let fixture = existing.fixtures.iter().find(|f| f.id == fixture_id);
        let fixture = fixture.ok_or_else(|| format!("channel {} goes to a fixture this show does not have", each.channel))?;
        let parameter = parameter_of(fixture, &key, existing)
            .ok_or_else(|| format!("{} has no {key} for channel {} to drive", fixture.name, each.channel))?;
        if parameter.direction != ParameterDirection::Output || !matches!(parameter.default_value, ParameterValue::Bool(_)) {
            return Err(format!("{key} on {} is not an output that switches", fixture.name));
        }
        if let Some(other) = taken.insert((fixture_id, key.clone()), each.channel) {
            return Err(format!("channels {other} and {} both go to {key} on {}", each.channel, fixture.name));
        }
        targets.insert(each.channel, (fixture_id, key));
    }

    let mut placeholders_needed = false;
    let mut placeholders_written = BTreeSet::new();
    for channel in 1..=8u8 {
        if targets.contains_key(&channel) || discarded.contains(&channel) || !controller.used(channel as usize) {
            continue;
        }
        let fixture_id = placeholder_id(name, channel);
        let label = controller.channels.get(channel as usize - 1).map(|c| c.name.as_str()).unwrap_or("");
        let earlier = existing.fixtures.iter().find(|f| f.id == fixture_id);
        let fixture = Fixture {
            id: fixture_id,
            name: format!("{name} · {label}"),
            fixture_type_id: placeholder_type_id(),
            // Kept across a re-import: the wing selects by it.
            fixture_number: earlier.and_then(|f| f.fixture_number),
            ..Fixture::default()
        };
        plan.write("fixtures", earlier.map(|f| f.id), json(&fixture));
        placeholders_needed = true;
        placeholders_written.insert(fixture_id);
        targets.insert(channel, (fixture_id, parameter_key(&SWITCH)));
    }
    if placeholders_needed && !existing.fixture_types.iter().any(|t| t.id == placeholder_type_id()) {
        // Ahead of the fixtures that are its instances: a thing before what points at it.
        let placeholder = FixtureType {
            id: placeholder_type_id(),
            name: "Unassigned OctoBanger output".into(),
            manufacturer: "OctoBanger".into(),
            parameters: vec![ParameterDefinition::on_port(SWITCH, 0, ParameterValue::Bool(false))],
            channel_count: 0,
            source: FixtureTypeSource::Manual,
            ..FixtureType::default()
        };
        plan.report.created += 1;
        plan.writes.insert(0, (ImportPlan::create("fixture_types"), pult_schema::lifecycle::Lifecycle::Persisted, json(&placeholder)));
    }
    for channel in 1..=8u8 {
        let fixture_id = placeholder_id(name, channel);
        if !placeholders_written.contains(&fixture_id) {
            if let Some(left) = existing.fixtures.iter().find(|f| f.id == fixture_id) {
                plan.report.missing.push(format!("{} is no longer used by this prop", left.name));
            }
        }
    }

    // ── The recording ──
    let scare_id = id(name, "scare");
    let ambience_id = id(name, "ambience");
    let mut recording = pult_render::track::Track::default();
    for (channel, (fixture_id, key)) in &targets {
        let points = controller
            .changes(*channel as usize)
            .into_iter()
            .map(|(ms, on)| pult_render::track::TrackPoint { ms, value: ParameterValue::Bool(on), release: false })
            .collect();
        recording.keys.push(pult_render::track::TrackKey { fixture_id: *fixture_id, key: key.clone(), points });

        for other in existing.timelines.iter().filter(|t| t.id != scare_id) {
            if other.tracks.iter().any(|row| row.fixture_id == Some(*fixture_id) && row.key.as_deref() == Some(key)) {
                let fixture = existing.fixtures.iter().find(|f| f.id == *fixture_id).map(|f| f.name.as_str()).unwrap_or("?");
                plan.report.warnings.push(format!(
                    "{key} on {fixture} is also recorded on {}: whichever runs last drives it",
                    other.name
                ));
            }
        }
    }
    let mut tracks = Vec::new();
    if !recording.keys.is_empty() {
        let bytes = pult_render::track::encode(&recording);
        let sha = assets::digest(&bytes);
        plan.assets.push((assets::TRACK_MIME.into(), bytes));
        let whole = TimelineTrack {
            id: id(name, "recording"),
            name: name.into(),
            asset: sha,
            input_id: None,
            offset_ms: 0,
            enabled: true,
            fixture_id: None,
            key: None,
            from_ms: None,
            to_ms: None,
        };
        let keys: Vec<(Uuid, String)> = recording.keys.iter().map(|k| (k.fixture_id, k.key.clone())).collect();
        tracks = split_row(&whole, &keys);
        // A row per channel, each with an id of its own that is the same next time.
        for (row, (_, key)) in tracks.iter_mut().zip(&keys) {
            row.id = id(name, &format!("recording {}", row.fixture_id.unwrap_or_default()));
            row.name = format!("{name} · {key}");
        }
    }

    // ── The sound ──
    if let Some(AudioDestination::Zone { zone }) = &mapping.destination {
        if !existing.speaker_zones.iter().any(|z| z.id == *zone) {
            return Err("the sound goes to a speaker zone this show does not have".into());
        }
    }
    let sound_for = |name: &Option<String>| name.as_ref().and_then(|n| sounds.get(n).map(|s| (n.clone(), s)));
    let gain_db = match controller.media.volume {
        // The MP3 board's own scale, 30 being as loud as it goes.
        v @ 1..=29 => 20.0 * (v as f32 / 30.0).log10(),
        _ => 0.0,
    };
    let clip = |file: &str, sound: &Sound, length_ms: Option<u64>, destination: &AudioDestination, what: &str| AudioClip {
        id: id(name, what),
        name: file.into(),
        asset: sound.sha.clone(),
        peaks: None,
        start_ms: 0,
        offset_ms: 0,
        length_ms,
        gain_db,
        fade_in_ms: 0,
        // The board stopped dead; a few milliseconds keep that from being a click.
        fade_out_ms: 20,
        repeats: 1,
        stored: false,
        destination: destination.clone(),
    };
    let wants_sound = !wanted_sounds(controller).is_empty();
    let mut scare_clips = Vec::new();
    let mut ambience: Option<Timeline> = None;
    match &mapping.destination {
        None if wants_sound => plan.report.warnings.push("no sound was imported: nowhere was chosen for it to play".into()),
        None => {}
        Some(destination) => {
            if wanted_sounds(controller).iter().any(|(role, _)| *role == "scare") {
                match sound_for(&controller.media.scare) {
                    Some((file, sound)) => {
                        // Cut where the sequence ends, which is where the firmware
                        // stopped the board.
                        let length = sound.duration_ms.min(controller.end_ms as u64);
                        scare_clips.push(clip(&file, sound, Some(length), destination, "scare clip"));
                        if let Some(bytes) = &sound.bytes {
                            plan.assets.push((mime_of(&file), bytes.clone()));
                        }
                    }
                    None => {
                        let file = controller.media.scare.clone().unwrap_or_default();
                        plan.report.warnings.push(format!("{file} was not supplied, so the scare has no sound"));
                    }
                }
            }
            if controller.media.kind == MediaKind::Mp3AmbientScare && mapping.ambience {
                match sound_for(&controller.media.ambient) {
                    Some((file, sound)) if sound.duration_ms > 0 => {
                        if let Some(bytes) = &sound.bytes {
                            plan.assets.push((mime_of(&file), bytes.clone()));
                        }
                        ambience = Some(Timeline {
                            clips: vec![clip(&file, sound, None, destination, "ambience clip")],
                            ..a_timeline(
                                ambience_id,
                                &format!("{name} ambience"),
                                Some(TimelineEnd { at_ms: sound.duration_ms, then: AtEnd::Loop }),
                            )
                        });
                    }
                    _ => {
                        let file = controller.media.ambient.clone().unwrap_or_default();
                        plan.report.warnings.push(format!("{file} was not supplied, so there is no ambience"));
                    }
                }
            }
        }
    }

    // ── The scare ──
    let mut events = Vec::new();
    if let Some(ambience) = &ambience {
        events.push(TimelineEvent {
            id: id(name, "stop ambience"),
            at_ms: 0,
            action: TimelineAction::StopTimeline { timeline_id: ambience.id },
        });
        events.push(TimelineEvent {
            id: id(name, "play ambience"),
            at_ms: controller.end_ms,
            action: TimelineAction::PlayTimeline { timeline_id: ambience.id, if_running: IfRunning::Ignore },
        });
    }
    let scare = Timeline {
        clips: scare_clips,
        tracks,
        events,
        ..a_timeline(
            scare_id,
            name,
            // On past the sequence for the reset delay, which is the lockout: a trigger
            // plays this with *Ignore if running*.
            Some(TimelineEnd { at_ms: controller.end_ms as u64 + controller.reset_delay_s as u64 * 1_000, then: AtEnd::Stop }),
        )
    };

    // A thing before what points at it: the ambience before the scare whose events play
    // it, and both before the flow.
    if let Some(ambience) = &ambience {
        plan.write("timelines", existing.timelines.iter().find(|t| t.id == ambience.id).map(|t| t.id), json(ambience));
    }
    plan.write("timelines", existing.timelines.iter().find(|t| t.id == scare.id).map(|t| t.id), json(&scare));

    // ── The trigger ──
    let flow_id = id(name, "flow");
    let flow = Flow { id: flow_id, name: name.into(), enabled: true, folder: None };
    plan.write("flows", existing.flows.iter().find(|f| f.id == flow_id).map(|f| f.id), json(&flow));

    let node = |what: &str, kind: FlowNodeKind, label: Option<String>, x: f32, y: f32| FlowNode {
        id: id(name, &format!("flow {what}")),
        flow_id,
        kind,
        name: label,
        x,
        y,
        active: false,
        last_fired_at: None,
        pulsed_at: None,
        due_at: None,
        disabled: None,
    };
    let play = node(
        "play",
        FlowNodeKind::Action(TriggerAction::PlayTimeline { timeline_id: scare_id, if_running: IfRunning::Ignore }),
        None,
        560.0,
        120.0,
    );
    let button = node("button", FlowNodeKind::Button, Some(format!("Trigger {name}")), 40.0, 40.0);
    let mut nodes = vec![button, play];
    let mut edges = vec![("button to play", 0usize, 1usize)];
    if let Some(trigger) = &mapping.trigger {
        let fixture = existing.fixtures.iter().find(|f| f.id == trigger.fixture_id);
        let fixture = fixture.ok_or("the trigger is on a fixture this show does not have")?;
        let parameter = parameter_of(fixture, &trigger.key, existing)
            .ok_or_else(|| format!("{} has no {} to trigger from", fixture.name, trigger.key))?;
        if parameter.direction != ParameterDirection::Input {
            return Err(format!("{} on {} is not an input", trigger.key, fixture.name));
        }
        let edge = mapping.edge.unwrap_or(match controller.trigger {
            pult_octobanger::TriggerLevel::Hi => TriggerCondition::FallingEdge,
            _ => TriggerCondition::RisingEdge,
        });
        nodes.push(node(
            "source",
            FlowNodeKind::Source(TriggerSource::Parameter { fixture_id: fixture.id, parameter: parameter.kind.clone() }),
            None,
            40.0,
            200.0,
        ));
        nodes.push(node("condition", FlowNodeKind::Condition(edge), None, 300.0, 200.0));
        edges.push(("source to condition", 2, 3));
        edges.push(("condition to play", 3, 1));
    }
    let mut wrote = BTreeSet::new();
    for each in &nodes {
        wrote.insert(each.id);
        plan.write("flow_nodes", existing.flow_nodes.iter().find(|n| n.id == each.id).map(|n| n.id), json(each));
    }
    for (what, from, to) in edges {
        let edge = FlowEdge {
            id: id(name, &format!("flow edge {what}")),
            flow_id,
            from_node: nodes[from].id,
            from_port: 0,
            to_node: nodes[to].id,
            to_port: 0,
        };
        wrote.insert(edge.id);
        plan.write("flow_edges", existing.flow_edges.iter().find(|e| e.id == edge.id).map(|e| e.id), json(&edge));
    }
    // What an earlier import of this prop drew that this one does not: a trigger taken
    // away. Listed and never deleted, the rule every importer here follows.
    for left in existing.flow_nodes.iter().filter(|n| n.flow_id == flow_id && !wrote.contains(&n.id)) {
        if ["flow source", "flow condition"].iter().any(|what| id(name, what) == left.id) {
            plan.report.missing.push(format!("the {name} flow still has its old trigger"));
            break;
        }
    }

    Ok(plan)
}

fn parameter_of<'a>(fixture: &Fixture, key: &str, existing: &'a Existing) -> Option<&'a ParameterDefinition> {
    let kind = existing.fixture_types.iter().find(|t| t.id == fixture.fixture_type_id)?;
    kind.parameter_for_key(key).filter(|p| parameter_key(&p.kind) == key)
}

fn mime_of(file: &str) -> String {
    assets::mime_for_name(file).unwrap_or("audio/mpeg").into()
}

fn json<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).expect("a schema row serializes")
}

fn a_timeline(id: Uuid, name: &str, end: Option<TimelineEnd>) -> Timeline {
    Timeline {
        folder: None,
        id,
        name: name.into(),
        clips: Vec::new(),
        detected: None,
        source: TimelineSource::Internal,
        grid: Vec::new(),
        markers: Vec::new(),
        events: Vec::new(),
        tracks: Vec::new(),
        flashes: None,
        pending: None,
        speed_master: None,
        node_id: None,
        end,
        running: false,
        anchor_ms: 0,
        position_at_anchor_ms: 0,
        rate: 1.0,
        recording: None,
        armed_by: None,
    }
}

// ── The two acts, over a station ──────────────────────────────────────────────

async fn read<T: serde::de::DeserializeOwned>(engine: &EngineHandle, table: &str) -> Vec<T> {
    engine
        .get(vec![pult_schema::path::PathSegment::Key(table.into())])
        .await
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// The show, as much of it as an import is matched against.
struct Held {
    fixtures: Vec<Fixture>,
    fixture_types: Vec<FixtureType>,
    timelines: Vec<Timeline>,
    flows: Vec<Flow>,
    flow_nodes: Vec<FlowNode>,
    flow_edges: Vec<FlowEdge>,
    speaker_zones: Vec<SpeakerZone>,
}

impl Held {
    async fn read(engine: &EngineHandle) -> Held {
        Held {
            fixtures: read(engine, "fixtures").await,
            fixture_types: read(engine, "fixture_types").await,
            timelines: read(engine, "timelines").await,
            flows: read(engine, "flows").await,
            flow_nodes: read(engine, "flow_nodes").await,
            flow_edges: read(engine, "flow_edges").await,
            speaker_zones: read(engine, "speaker_zones").await,
        }
    }

    fn existing(&self) -> Existing<'_> {
        Existing {
            fixtures: &self.fixtures,
            fixture_types: &self.fixture_types,
            timelines: &self.timelines,
            flows: &self.flows,
            flow_nodes: &self.flow_nodes,
            flow_edges: &self.flow_edges,
            speaker_zones: &self.speaker_zones,
        }
    }
}

/// `POST /api/import/obc/read`.
pub async fn describe_upload(engine: &EngineHandle, bytes: &[u8]) -> Result<Description, String> {
    let held = Held::read(engine).await;
    describe(bytes, &held.existing()).await
}

/// `POST /api/import/obc`: one controller of an upload, into the show, as `user_id`,
/// in one gesture.
pub async fn import(
    engine: &EngineHandle,
    store: &AssetStore,
    bytes: &[u8],
    mapping: &Mapping,
    user_id: Uuid,
) -> Result<ImportReport, String> {
    let project = Project::read(bytes).map_err(|e| e.to_string())?;
    let found = project
        .controllers
        .iter()
        .find(|c| c.path == mapping.controller)
        .ok_or_else(|| format!("the upload has no controller at {}", mapping.controller))?;

    let mut sounds = BTreeMap::new();
    if mapping.destination.is_some() {
        for (_, file) in wanted_sounds(&found.controller) {
            let (bytes, stored) = match project.file(&found.path, &file).map_err(|e| e.to_string())? {
                Some(bytes) => (bytes, false),
                None => match mapping.audio.get(&file) {
                    Some(sha) => match store.get(sha).await {
                        Ok(Some(asset)) => (asset.bytes, true),
                        _ => return Err(format!("{file} was uploaded and is not in this show's assets")),
                    },
                    None => continue,
                },
            };
            let survey = surveyed(bytes.clone()).await.map_err(|e| format!("{file}: {e}"))?;
            let sha = assets::digest(&bytes);
            sounds.insert(
                file,
                Sound { sha, bytes: (!stored).then_some(bytes), duration_ms: survey.duration_ms },
            );
        }
    }

    let held = Held::read(engine).await;
    let plan = plan_import(&found.controller, mapping, &sounds, &held.existing())?;
    super::apply::apply(plan, store, engine, user_id).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
