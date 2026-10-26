//! The wing, on one station: who is showing it, and the row a page reads it from.
//!
//! The station runs the whole wing (`infra/wing_control/`), page or no page. A page
//! claims it to share its selection with it and to show what it is doing; with none
//! claiming, the wing goes on doing everything it does and simply has nobody to show
//! the command line to. Only one page claims it at a time, so a second tab cannot
//! stand in for the operator at the desk.
//!
//! The status and the operator — the line, the encoders, the calculator, learning, the
//! selection — are the LOCAL `wing` row, so every page on this station can see whether
//! a wing is here, who has it and what it is doing. What the station needs a screen for
//! rides `UpdateBroadcast` on `wing_asks` the way log lines do: no `ShowState` entry,
//! nothing replicated, and a page that has not claimed the wing does not subscribe.

use std::sync::{Arc, Mutex};

use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::wing::{WingContext, WingOperator, WingState},
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{engine::EngineHandle, infra::clients::short_id, wing::WingHandle};

/// Where the station asks the claiming page for what needs a screen.
pub const ASKS_PATH: &str = "wing_asks";

/// What a claiming page says to the wing, handed to the loop that runs it.
#[derive(Debug)]
pub enum Inbound {
    Context(WingContext),
    /// The page that claimed the wing has let go of it, or gone.
    Released,
    Learn(bool),
    /// Open the calculator on an encoder (`None` closes it), with what is typed, and
    /// set it on the selection when `set`.
    Calculator { slot: Option<u8>, text: Option<String>, set: bool },
}

#[derive(Clone)]
pub struct WingStation {
    wing: WingHandle,
    claimed: Arc<Mutex<Option<Uuid>>>,
    /// The digital inputs as last reported, kept whether or not a page is claiming.
    digital: Arc<Mutex<Option<u8>>>,
    operator: Arc<Mutex<WingOperator>>,
    inbound: mpsc::UnboundedSender<Inbound>,
    inbound_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<Inbound>>>>,
    engine: EngineHandle,
}

impl WingStation {
    /// Take over the process's wing for this station.
    ///
    /// Dark to begin with rather than as the last station left it, because a show was
    /// just switched and whatever it was lighting meant the previous show.
    pub fn new(wing: WingHandle, engine: EngineHandle) -> Self {
        wing.go_dark();
        let (inbound, rx) = mpsc::unbounded_channel();
        Self {
            wing,
            claimed: Arc::new(Mutex::new(None)),
            digital: Arc::new(Mutex::new(None)),
            operator: Arc::new(Mutex::new(WingOperator::default())),
            inbound,
            inbound_rx: Arc::new(Mutex::new(Some(rx))),
            engine,
        }
    }

    /// Follow the wing for as long as the station runs: its status and its digital
    /// inputs into the `wing` row.
    pub async fn run(self) {
        let mut report = self.wing.report();
        let mut events = self.wing.events();
        self.publish().await;
        // A wing that can never change state — no grandMA3 to boot it from — has a
        // status nobody will write again, and that is no reason to stop reading it.
        let mut reporting = true;
        loop {
            tokio::select! {
                changed = report.changed(), if reporting => {
                    match changed {
                        Ok(()) => { self.publish().await; }
                        Err(_) => reporting = false,
                    }
                }
                batch = events.recv() => match batch {
                    Ok(batch) => {
                        let level = batch.iter().rev().find_map(|event| match event {
                            pult_wing::Event::Digital { bits } => Some(*bits),
                            _ => None,
                        });
                        if let Some(bits) = level {
                            let moved = self.digital.lock().unwrap().replace(bits) != Some(bits);
                            if moved {
                                self.publish().await;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    }

    /// What the loop running the wing hears from the page. Once: there is one loop.
    pub fn take_inbound(&self) -> Option<mpsc::UnboundedReceiver<Inbound>> {
        self.inbound_rx.lock().unwrap().take()
    }

    /// This page shows the wing from now on — taking it from another page if one had
    /// it, because the operator standing at the wing is the one who asked.
    pub async fn claim(&self, session: Uuid) -> WingState {
        let previous = self.claimed.lock().unwrap().replace(session);
        if previous.is_some() && previous != Some(session) {
            let _ = self.inbound.send(Inbound::Released);
        }
        self.publish().await
    }

    /// Let go, if this page is the one holding it.
    pub async fn release(&self, session: Uuid) -> WingState {
        self.forget(session).await;
        self.state()
    }

    /// A page has gone. If it was claiming the wing, nobody is now.
    pub async fn forget(&self, session: Uuid) {
        let was = {
            let mut claimed = self.claimed.lock().unwrap();
            if *claimed == Some(session) {
                *claimed = None;
                true
            } else {
                false
            }
        };
        if was {
            let _ = self.inbound.send(Inbound::Released);
            self.publish().await;
        }
    }

    /// Whether some page is claiming the wing.
    pub fn claimed(&self) -> bool {
        self.claimed.lock().unwrap().is_some()
    }

    /// Something only the claiming page may say: anybody else is refused, so a second
    /// tab cannot change what the wing acts on under the operator.
    pub fn from_page(&self, session: Uuid, message: Inbound) -> Result<(), String> {
        if *self.claimed.lock().unwrap() != Some(session) {
            return Err("this page is not driving the wing".into());
        }
        self.inbound.send(message).map_err(|_| "the wing is not running".to_string())
    }

    /// The operator as it is now.
    pub fn operator(&self) -> WingOperator {
        self.operator.lock().unwrap().clone()
    }

    /// Change the operator and publish the row, if anything changed.
    pub async fn update_operator(&self, f: impl FnOnce(&mut WingOperator)) {
        let changed = {
            let mut operator = self.operator.lock().unwrap();
            let before = operator.clone();
            f(&mut operator);
            *operator != before
        };
        if changed {
            self.publish().await;
        }
    }

    /// MA's names for every control, where this machine has a grandMA3 to read them
    /// from.
    pub fn map(&self) -> serde_json::Value {
        self.wing
            .map()
            .and_then(|map| serde_json::to_value(&*map).ok())
            .unwrap_or(serde_json::Value::Null)
    }

    pub fn state(&self) -> WingState {
        let report = self.wing.report().borrow().clone();
        WingState {
            phase: report.phase,
            device: report.device,
            claimed_by: self.claimed.lock().unwrap().map(short_id),
            digital: *self.digital.lock().unwrap(),
            operator: self.operator(),
        }
    }

    async fn publish(&self) -> WingState {
        let state = self.state();
        if let Ok(value) = serde_json::to_value(&state) {
            let path = vec![PathSegment::Key("wing".into())];
            let _ = self.engine.set(path, Lifecycle::Local, value).await;
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ShowEngine;
    use crate::infra::showfile;
    use pult_schema::{events::operation::NodeId, types::wing::WingPhase};
    use pult_wing::Event;

    async fn a_station() -> (WingStation, EngineHandle, tokio::sync::broadcast::Sender<Vec<Event>>) {
        let pool = Arc::new(showfile::open_in_memory().await.expect("in-memory showfile"));
        let (engine, handle, _broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
        tokio::spawn(engine.run());
        let (wing, _report, events) = WingHandle::detached(WingPhase::Running);
        let station = WingStation::new(wing, handle.clone());
        tokio::spawn(station.clone().run());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        (station, handle, events)
    }

    async fn row(engine: &EngineHandle) -> WingState {
        serde_json::from_value(engine.get(vec![PathSegment::Key("wing".into())]).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn the_row_says_whether_a_wing_is_here_and_who_has_it() {
        let (station, engine, _) = a_station().await;
        assert_eq!(row(&engine).await.phase, WingPhase::Running);
        assert_eq!(row(&engine).await.claimed_by, None);

        let page = Uuid::new_v4();
        station.claim(page).await;
        assert_eq!(row(&engine).await.claimed_by, Some(short_id(page)));

        station.forget(page).await;
        assert_eq!(row(&engine).await.claimed_by, None, "a page that goes takes its claim with it");
    }

    /// The digital inputs are a level the row keeps whether or not anybody is claiming,
    /// so a page knows where they rest.
    #[tokio::test]
    async fn the_row_keeps_the_digital_inputs_with_nobody_claiming() {
        let (_station, engine, events) = a_station().await;
        assert_eq!(row(&engine).await.digital, None, "nothing reported yet");
        events.send(vec![Event::Digital { bits: 0b10 }]).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(row(&engine).await.digital, Some(0b10));
    }

    /// Only the claiming page may tell the wing what it is acting on.
    #[tokio::test]
    async fn only_the_claiming_page_is_heard() {
        let (station, _engine, _) = a_station().await;
        let (page, other) = (Uuid::new_v4(), Uuid::new_v4());
        station.claim(page).await;
        assert!(station.from_page(other, Inbound::Learn(true)).is_err());
        assert!(station.from_page(page, Inbound::Learn(true)).is_ok());
    }

    #[tokio::test]
    async fn the_operator_is_in_the_row() {
        let (station, engine, _) = a_station().await;
        station.update_operator(|op| op.line = "fixture 1".into()).await;
        assert_eq!(row(&engine).await.operator.line, "fixture 1");
    }
}
