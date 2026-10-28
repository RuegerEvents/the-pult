//! A grandMA3 command wing, for the process.
//!
//! Installed from `main`, never from `start`, for the reason the log is: the wing's
//! interface can be claimed once per *process*, and a station is a library a process may
//! start more than one of — and starts again every time a show is opened. The wing also
//! takes seconds to boot, because its flash holds only a bootloader and the host feeds
//! it its application at every connect; a wing owned by a station would be rebooted on
//! every show switch. So `install` spawns the USB thread for the life of the program and
//! hands back a [`WingHandle`] that both binaries put in `Config`. A station given none
//! has no wing, which is what every test wants.
//!
//! Two files from a local grandMA3 install are needed and neither is shipped: the
//! wing's application image and MA's control map. Without the image the wing cannot
//! run and says so on its status row; without the map it runs and the panel shows
//! numbers where it would show names.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pult_schema::types::wing::WingPhase;
use pult_wing::{usb, Event, OutputState, WingMap};
use tokio::sync::{broadcast, watch};

/// What the wing is doing and what it is called, as the USB thread last said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WingReport {
    pub phase: WingPhase,
    pub device: Option<String>,
}

/// The process's wing. Cheap to clone; every station started in this process shares it.
#[derive(Clone)]
pub struct WingHandle {
    report: watch::Receiver<WingReport>,
    events: broadcast::Sender<Vec<Event>>,
    out: Arc<Mutex<OutputState>>,
    map: Option<Arc<WingMap>>,
    /// The faders a hand is on, by fader index, as the wing itself reports touch.
    ///
    /// Kept here, beside the frame, so a motor lets go **in the next USB frame** rather
    /// than when a browser has heard about the touch and sent a frame back: wing → Pi →
    /// socket → page → its next 40 ms feedback → Pi → wing was a tenth of a second and
    /// more in which the motor held against the hand, and the faders felt heavy (found
    /// at the gig). Whatever a page asks for, a touched fader is not driven.
    hands: Arc<Mutex<HashSet<usize>>>,
}

impl std::fmt::Debug for WingHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WingHandle").field("report", &*self.report.borrow()).finish_non_exhaustive()
    }
}

impl WingHandle {
    /// What the wing is doing, followed.
    pub fn report(&self) -> watch::Receiver<WingReport> {
        self.report.clone()
    }

    /// Everything pressed, moved and turned from now on.
    pub fn events(&self) -> broadcast::Receiver<Vec<Event>> {
        self.events.subscribe()
    }

    /// MA's names for every control, where a grandMA3 install had them.
    pub fn map(&self) -> Option<Arc<WingMap>> {
        self.map.clone()
    }

    /// Change the frame the USB thread keeps putting on the wire. A fader somebody's
    /// hand is on stays released, whatever `f` asked for it — and so is every fader on
    /// a station started with `PULT_WING_MOTORS=off`.
    pub fn drive(&self, f: impl FnOnce(&mut OutputState)) {
        let mut out = self.out.lock().unwrap();
        f(&mut out);
        if !motors_on() {
            for index in 0..out.faders.len() {
                out.release_fader(index);
            }
            return;
        }
        for index in self.hands.lock().unwrap().iter() {
            out.release_fader(*index);
        }
    }

    /// Whether a hand is on this fader, as the wing's own touch contact says.
    pub fn touched(&self, fader: usize) -> bool {
        self.hands.lock().unwrap().contains(&fader)
    }

    /// Every light off and every motor let go: what a wing nobody is driving looks
    /// like. A motor held where the last page left it is a fader that fights the hand.
    pub fn go_dark(&self) {
        self.drive(|out| {
            out.all_leds(0);
            for index in 0..out.faders.len() {
                out.release_fader(index);
            }
        });
    }

    /// A handle with nothing behind it, for a test that wants the station's half.
    pub fn detached(phase: WingPhase) -> (Self, watch::Sender<WingReport>, broadcast::Sender<Vec<Event>>) {
        let (report_tx, report) = watch::channel(WingReport { phase, device: None });
        let (events, _) = broadcast::channel(256);
        let handle = WingHandle {
            report,
            events: events.clone(),
            out: Arc::new(Mutex::new(OutputState::default())),
            map: None,
            hands: Default::default(),
        };
        (handle, report_tx, events)
    }

    /// What the frame says now, for a test.
    pub fn frame(&self) -> OutputState {
        self.out.lock().unwrap().clone()
    }
}

/// Find a grandMA3 install, start the USB thread, and hand back the wing.
///
/// Never fails: a machine with no grandMA3 gets a wing whose status row says why it
/// cannot run, which is what an operator needs to read and a station needs to carry on.
pub fn install() -> WingHandle {
    install_from(WingFiles::default())
}

/// Where MA's two files are, when they are not in a grandMA3 install on this machine.
///
/// grandMA3 onPC does not exist for every machine a wing is plugged into — a Raspberry
/// Pi at front of house has none — so the operator copies the two files over and says
/// where. Each one left `None` is looked for in `~/MALightingTechnology` as before.
#[derive(Debug, Clone, Default)]
pub struct WingFiles {
    /// `command_wing_app.bin`, the application the wing asks for at every connect.
    pub image: Option<std::path::PathBuf>,
    /// `hardware_configurations.xml`, which names every control.
    pub map: Option<std::path::PathBuf>,
}

/// [`install`], with the files named.
pub fn install_from(files: WingFiles) -> WingHandle {
    let map = files.map.or_else(usb::find_map).and_then(|path| {
        let text = std::fs::read_to_string(&path).ok()?;
        match WingMap::from_xml(&text, pult_wing::COMMAND_WING) {
            Ok(map) => Some(Arc::new(map)),
            Err(e) => {
                tracing::warn!(path = %path.display(), "the wing's control map would not read: {e}");
                None
            }
        }
    });

    let named = files.image.clone();
    let mut refused = None;
    let image = files.image.or_else(usb::find_image).and_then(|path| match usb::read_image(&path) {
        Ok(image) => Some(image),
        Err(e) => {
            refused = Some(format!("{e:#}"));
            None
        }
    });
    let Some(image) = image else {
        let reason = match (refused, named) {
            (Some(why), _) => why,
            (None, Some(path)) => format!("cannot read the wing's application image at {}", path.display()),
            (None, None) => "no grandMA3 install with command_wing_app.bin under ~/MALightingTechnology; \
                     the wing needs MA's application image and this console does not ship it — \
                     copy it here and pass --wing-image"
                .into(),
        };
        let (mut handle, _, _) = WingHandle::detached(WingPhase::Fault { reason });
        handle.map = map;
        return handle;
    };

    let wing = match usb::spawn(image) {
        Ok(wing) => wing,
        Err(e) => {
            let (mut handle, _, _) =
                WingHandle::detached(WingPhase::Fault { reason: format!("{e:#}") });
            handle.map = map;
            return handle;
        }
    };

    let (report_tx, report) = watch::channel(WingReport { phase: WingPhase::Absent, device: None });
    let (events, _) = broadcast::channel(256);
    let events_tx = events.clone();
    let hands: Arc<Mutex<HashSet<usize>>> = Default::default();
    let touches = map.as_deref().map(touch_keys).unwrap_or_default();
    let (hands_tx, out_tx) = (hands.clone(), wing.out.clone());
    // The USB thread talks over a std channel and the stations want a watch and a
    // broadcast; one bridge thread rather than teaching the USB loop about either.
    let reports = wing.reports;
    let spawned = std::thread::Builder::new().name("wing-reports".into()).spawn(move || {
        while let Ok(r) = reports.recv() {
            match r {
                usb::Report::Events { events } => {
                    // A hand on a fader frees its motor here, before anything else hears
                    // about it; see `WingHandle::hands`.
                    for event in &events {
                        if let Event::Key { index, down } = event {
                            if let Some(fader) = touches.get(index) {
                                let mut held = hands_tx.lock().unwrap();
                                if *down {
                                    held.insert(*fader);
                                    out_tx.lock().unwrap().release_fader(*fader);
                                } else {
                                    held.remove(fader);
                                }
                            }
                        }
                    }
                    let _ = events_tx.send(events);
                }
                usb::Report::Status { state, detail } => {
                    let phase = phase_of(&state, &detail);
                    if matches!(phase, WingPhase::Running | WingPhase::Fault { .. }) {
                        tracing::info!(%state, %detail, "wing");
                    }
                    report_tx.send_modify(|r| {
                        // A wing that went away is a new wing when it comes back, and
                        // might be another one.
                        if phase == WingPhase::Absent {
                            r.device = None;
                        }
                        r.phase = phase;
                    });
                }
                usb::Report::DeviceType { name, .. } => {
                    report_tx.send_modify(|r| r.device = Some(name));
                }
                usb::Report::Loading { sent, total } => {
                    report_tx.send_modify(|r| {
                        r.phase = WingPhase::Loading { sent: sent as u32, total: total as u32 }
                    });
                }
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("the wing's report thread did not start: {e}");
    }

    WingHandle { report, events, out: wing.out, map, hands }
}

/// Which hardkeys are a hand on which fader: MA models touching a fader as a hardkey
/// with the code `FADER` and that fader's own executor. The station's copy of
/// `faderKeys` in the frontend's `wing.ts`, the `touch` half of it only.
fn touch_keys(map: &WingMap) -> HashMap<u16, usize> {
    let by_executor: HashMap<&str, usize> = map
        .faders
        .iter()
        .filter_map(|(index, control)| Some((control.executor.as_deref()?, *index as usize)))
        .collect();
    map.hardkeys
        .iter()
        .filter(|(_, control)| control.code.as_deref() == Some("FADER"))
        .filter_map(|(index, control)| Some((*index, *by_executor.get(control.executor.as_deref()?)?)))
        .collect()
}

/// What the USB thread's own words come to.
///
/// A lost handle is ordinary — the wing drops off the bus after taking its image, and
/// whenever nothing is plugged in the thread says so every few seconds — so only a
/// loss that names something other than absence is a fault.
fn phase_of(state: &str, detail: &str) -> WingPhase {
    match state {
        "running" => WingPhase::Running,
        "booting" | "restarting" => WingPhase::Booting,
        "lost" if detail.starts_with("no grandMA3 command wing") => WingPhase::Absent,
        _ => WingPhase::Fault { reason: detail.to_string() },
    }
}

/// Whether the motor faders are driven at all. `PULT_WING_MOTORS=off` lets every one
/// go, so the faders only ever report what a hand does: at Heinz Horror Garten
/// (2026-10-28) the motors fought the operator, and the faders are worth more as
/// plain inputs than as a picture of the show. Read once per process.
fn motors_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        !matches!(std::env::var("PULT_WING_MOTORS").as_deref().map(str::trim), Ok("off" | "0" | "false" | "no"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_plugged_in_is_absence_and_not_a_fault() {
        assert_eq!(phase_of("lost", "no grandMA3 command wing at 2dbe:b5c8"), WingPhase::Absent);
        assert_eq!(
            phase_of("lost", "claiming interface 0: Access denied"),
            WingPhase::Fault { reason: "claiming interface 0: Access denied".into() }
        );
        assert_eq!(phase_of("restarting", "the wing is starting what it was given"), WingPhase::Booting);
    }

    #[test]
    fn a_touched_fader_is_not_driven_whatever_a_page_asks() {
        let (wing, _, _) = WingHandle::detached(WingPhase::Running);
        wing.hands.lock().unwrap().insert(3);
        wing.drive(|out| {
            out.set_fader(3, 2000);
            out.set_fader(4, 2000);
        });
        let frame = wing.frame();
        assert_eq!(frame.faders[3], None, "the hand has it");
        assert!(frame.faders[4].is_some());
    }

    #[test]
    fn going_dark_lets_every_motor_go() {
        let (wing, _, _) = WingHandle::detached(WingPhase::Running);
        wing.drive(|out| {
            out.set_led(4, 200);
            out.set_fader(2, 1000);
        });
        wing.go_dark();
        let frame = wing.frame();
        assert!(frame.leds.iter().all(|l| *l == 0));
        assert!(frame.faders.iter().all(Option::is_none));
    }
}
