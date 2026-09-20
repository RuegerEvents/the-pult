//! The USB half: owning a handle, booting the wing, and keeping it up.
//!
//! Everything about *what* the bytes mean is `pult-wing`'s. What lives here is the part
//! that cannot be tested without hardware — a claimed interface, two blocking pipes and
//! the rule that the host has to keep talking.

use anyhow::{anyhow, Context, Result};
use pult_wing::proto::{Boot, Step};
use pult_wing::{Event, InputState, Message, NodeId, OutputState};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const VID: u16 = 0x2dbe;
pub const PID: u16 = 0xb5c8;
const EP_IN: u8 = 0x81;
const EP_OUT: u8 = 0x02;

/// What the host calls itself. grandMA3 uses its own station id here; any value works
/// as long as it is not zero, which is what the wing sends.
const HOST_ID: NodeId = NodeId(0x0331_0200_0100_007F);

/// What the explorer tells the world.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Report {
    Status { state: String, detail: String },
    DeviceType { name: String, digital_in: u32 },
    Events { events: Vec<Event> },
    /// Loading the application, so the panel can show a bar rather than nothing.
    Loading { sent: usize, total: usize },
}

/// What the world asks of the wing.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Command {
    SetLed { channel: usize, level: u8 },
    SetRgb { r: usize, g: usize, b: usize, colour: [u8; 3] },
    AllLeds { level: u8 },
    SetFader { index: usize, value: u16 },
    ReleaseFader { index: usize },
}

/// The output frame the sender thread keeps putting on the wire.
pub type Shared = Arc<Mutex<OutputState>>;
/// What the desk currently reads as. Shared so that something painting LEDs can answer
/// "is this key down" without following the event stream itself.
pub type SharedInput = Arc<Mutex<InputState>>;

pub struct Wing {
    pub out: Shared,
    pub input: SharedInput,
    pub reports: Receiver<Report>,
}

/// Open the wing and run it until the process ends.
///
/// `image` is the application the wing asks for at every connect; see
/// `docs/WING-PROTOCOL.md` on why a host has to carry it and must not ship it.
pub fn spawn(image: Vec<u8>) -> Result<Wing> {
    let (tx, reports) = std::sync::mpsc::channel();
    let out: Shared = Arc::new(Mutex::new(OutputState::default()));
    let input: SharedInput = Arc::new(Mutex::new(InputState::default()));
    let shared = out.clone();
    let shared_input = input.clone();
    std::thread::Builder::new()
        .name("wing".into())
        .spawn(move || {
            // The boot outlives the connection, because the handover *is* a
            // disconnection: the wing takes its application, drops off the bus and
            // comes back, and the host has to remember on the far side that it has
            // already fed this wing. A `Boot` made fresh per connection forgets that
            // and leaves the wing holding at `Loading` for ever.
            let mut boot = Boot::default();
            loop {
            if let Err(e) = run(&mut boot, &image, &shared, &shared_input, &tx) {
                let _ = tx.send(Report::Status {
                    state: "lost".into(),
                    detail: e.to_string(),
                });
            }
            // The wing falls back to its bootloader whenever the host stops, so a lost
            // handle is an ordinary event and the answer is to start the boot again —
            // once the device is really there to talk to.
            wait_for_the_wing();
            }
        })
        .context("spawning the wing thread")?;
    Ok(Wing { out, input, reports })
}

/// Wait for the wing to go away and come back.
///
/// After the download it re-enumerates, and grandMA3 does not reappear on the bus for
/// about three seconds. Polling on a fixed timer instead catches the bootloader still
/// up, which restarts the whole download — so this insists on seeing it *absent* before
/// it will call it present again.
fn wait_for_the_wing() {
    let present = || rusb::open_device_with_vid_pid(VID, PID).is_some();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut was_absent = false;
    while Instant::now() < deadline {
        match (present(), was_absent) {
            (false, _) => was_absent = true,
            (true, true) => {
                // Give it a moment to finish settling before claiming it.
                std::thread::sleep(Duration::from_millis(300));
                return;
            }
            (true, false) => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    tracing::debug!("the wing never went away; trying anyway");
}

fn open() -> Result<rusb::DeviceHandle<rusb::GlobalContext>> {
    let handle = rusb::open_device_with_vid_pid(VID, PID)
        .ok_or_else(|| anyhow!("no grandMA3 command wing at {VID:04x}:{PID:04x}"))?;
    handle.claim_interface(0).context("claiming interface 0")?;
    // Alt 0 has no endpoints at all; the pipes only exist in alt 1. And the device
    // STALLs this for a moment after another host has just let go, so a first refusal
    // is not a verdict.
    let mut last = None;
    for _ in 0..10 {
        match handle.set_alternate_setting(0, 1) {
            Ok(()) => return Ok(handle),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    Err(anyhow!("could not select alt setting 1: {}", last.unwrap()))
}

fn run(
    boot: &mut Boot,
    image: &[u8],
    out: &Shared,
    input_shared: &SharedInput,
    tx: &Sender<Report>,
) -> Result<()> {
    let handle = open()?;
    let _ = tx.send(Report::Status {
        state: "booting".into(),
        detail: "claimed the interface".into(),
    });

    // A fresh picture of the desk per connection: nothing about what was held before
    // the wing restarted is still true.
    let mut input = InputState::default();
    *input_shared.lock().unwrap() = InputState::default();
    let mut buf = [0u8; 1024];
    let mut last_frame = Instant::now();
    let mut running = false;

    loop {
        // A read with a short timeout is the loop's clock: the wing talks when it has
        // something to say, and the timeout is what lets the host get a word in.
        match handle.read_bulk(EP_IN, &mut buf, Duration::from_millis(50)) {
            Ok(n) => {
                let (_from, msg) = match Message::decode(&buf[..n]) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!("undecodable {n}-byte frame: {e}");
                        continue;
                    }
                };
                tracing::debug!("rx {} bytes: {}", n, label(&msg));
                if let Message::Io(c) = &msg {
                    let events = input.absorb(c).unwrap_or_default();
                    for e in &events {
                        // Raw byte and bit beside the index, so a mis-derived index
                        // rule can be read straight off a key press instead of argued
                        // about. The index rule was first taken from grandMA3's *log*
                        // rendering, which has already been wrong once.
                        if let Event::Key { index, down } = e {
                            if *down {
                                let (byte, bit) = pult_wing::input::key_bit(*index);
                                tracing::info!("KEY index={index} byte={byte} bit={bit}");
                            }
                        }
                    }
                    if !events.is_empty() {
                        *input_shared.lock().unwrap() = input.clone();
                        let _ = tx.send(Report::Events { events });
                    }
                }
                if let Message::Capabilities(c) = &msg {
                    let _ = tx.send(Report::DeviceType {
                        name: c.device_type.clone(),
                        digital_in: c.digital_in,
                    });
                }
                match boot.on(&msg, Some(image)) {
                    Step::Send(reply) => {
                        if let Message::SoftwarePacket { .. } = reply {
                            let _ = tx.send(Report::Loading {
                                sent: boot.sent as usize,
                                total: image.len(),
                            });
                        }
                        write(&handle, &reply)?;
                    }
                    Step::Running => {
                        if !running {
                            running = true;
                            let _ = tx.send(Report::Status {
                                state: "running".into(),
                                detail: format!(
                                    "{} up",
                                    boot.caps
                                        .as_ref()
                                        .map(|c| c.device_type.clone())
                                        .unwrap_or_else(|| "wing".into())
                                ),
                            });
                        }
                    }
                    Step::Restarting => {
                        // Let go, and do not come back until the wing has actually been
                        // away. Reconnecting on a timer catches it before the handover
                        // and starts the download again, for ever.
                        let _ = tx.send(Report::Status {
                            state: "restarting".into(),
                            detail: "the wing is starting what it was given".into(),
                        });
                        return Ok(());
                    }
                    Step::Wait => {}
                }
            }
            Err(rusb::Error::Timeout) => {}
            Err(rusb::Error::Pipe) => {
                // A stall is recoverable and the endpoint keeps working once it is
                // cleared, so this is not a reason to drop the handle and start the
                // boot again.
                tracing::debug!("IN pipe stalled; clearing");
                handle.clear_halt(EP_IN).ok();
            }
            Err(e) => return Err(anyhow!("read: {e}")),
        }

        // **Nothing unsolicited during the boot.** The wing drives it - announce,
        // capabilities, software request, ready - and the host only ever answers. A
        // heartbeat injected into that conversation stalls the IN pipe, which is what
        // the first version of this loop did every 500 ms, right after a download that
        // had otherwise worked perfectly.
        //
        // Once it is running the traffic reverses: the host sends a real-time frame
        // about every 16 ms and that is also what keeps the wing up, because a wing
        // whose host goes quiet drops back to its bootloader within a few seconds.
        if running && last_frame.elapsed() >= Duration::from_millis(16) {
            last_frame = Instant::now();
            let frames = {
                let mut o = out.lock().unwrap();
                o.sync.tick();
                o.frames()
            };
            // One block per frame, which is what the wing expects: faders, LEDs, sync.
            for f in frames {
                write_frame(&handle, &f)?;
            }
        }
    }
}

/// A short name for a log line. `Debug` on a real-time frame is 253 bytes of LED.
fn label(m: &Message) -> String {
    match m {
        Message::Heartbeat(s) => format!("heartbeat {s:?}"),
        Message::CapabilitiesRequest => "capabilities request".into(),
        Message::Capabilities(c) => format!("capabilities {:?}", c.device_type),
        Message::Io(_) => "real-time frame".into(),
        Message::Output(_) => "real-time frame".into(),
        Message::SoftwareRequest(n) => format!("software request {n:?}"),
        Message::SoftwarePacket { offset, last, data } => {
            format!("software packet at {offset} ({} bytes{})", data.len(),
                    if *last { ", last" } else { "" })
        }
        Message::SoftwareProgress(b) => format!("software progress ({} bytes)", b.len()),
        Message::Text(t) => format!("text {t:?}"),
        Message::Ready => "ready".into(),
        Message::Certificate => "certificate (not implemented)".into(),
        Message::Unknown(c) => format!("unknown container 0x{:04x}", c.tag()),
    }
}

/// One already-built container, wrapped in a frame and sent.
fn write_frame(
    handle: &rusb::DeviceHandle<rusb::GlobalContext>,
    container: &pult_wing::Chunk,
) -> Result<()> {
    use pult_wing::chunk::{ADDRESS, ROOT};
    let frame = pult_wing::Chunk::group(
        ROOT,
        vec![
            pult_wing::Chunk::bytes(ADDRESS, HOST_ID.0.to_le_bytes().to_vec()),
            container.clone(),
        ],
    );
    handle
        .write_bulk(EP_OUT, &frame.encode(), Duration::from_millis(500))
        .map(|_| ())
        .map_err(|e| anyhow!("write: {e}"))
}

fn write(handle: &rusb::DeviceHandle<rusb::GlobalContext>, msg: &Message) -> Result<()> {
    let bytes = msg.encode(HOST_ID);
    tracing::debug!("tx {} bytes: {}", bytes.len(), label(msg));
    handle
        .write_bulk(EP_OUT, &bytes, Duration::from_millis(500))
        .map(|_| ())
        .map_err(|e| anyhow!("write: {e}"))
}

/// Apply a command to the frame the sender thread is repeating.
pub fn apply(out: &Shared, cmd: &Command) {
    let mut o = out.lock().unwrap();
    match *cmd {
        Command::SetLed { channel, level } => o.set_led(channel, level),
        Command::SetRgb { r, g, b, colour } => o.set_rgb(r, g, b, colour),
        Command::AllLeds { level } => o.all_leds(level),
        Command::SetFader { index, value } => o.set_fader(index, value),
        Command::ReleaseFader { index } => o.release_fader(index),
    }
}
