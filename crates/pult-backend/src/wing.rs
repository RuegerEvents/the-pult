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

    /// Change the frame the USB thread keeps putting on the wire.
    pub fn drive(&self, f: impl FnOnce(&mut OutputState)) {
        f(&mut self.out.lock().unwrap());
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
    let image = files.image.or_else(usb::find_image).and_then(|path| std::fs::read(path).ok());
    let Some(image) = image else {
        let reason = match named {
            Some(path) => format!("cannot read the wing's application image at {}", path.display()),
            None => "no grandMA3 install with command_wing_app.bin under ~/MALightingTechnology; \
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
    // The USB thread talks over a std channel and the stations want a watch and a
    // broadcast; one bridge thread rather than teaching the USB loop about either.
    let reports = wing.reports;
    let spawned = std::thread::Builder::new().name("wing-reports".into()).spawn(move || {
        while let Ok(r) = reports.recv() {
            match r {
                usb::Report::Events { events } => {
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

    WingHandle { report, events, out: wing.out, map }
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
