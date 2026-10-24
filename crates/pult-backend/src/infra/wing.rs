//! The wing, on one station: who is driving it, and the relay between it and that page.
//!
//! A wing acts on one operator's selection, command line and gestures, and those live
//! in a page. So the station does no programming here at all: a page claims the wing,
//! the station relays what is pressed to it on the `wing_events` path, and the page
//! answers with the frame of lights and motor positions it wants (`wing.feedback`).
//! With no page claiming it the wing is dark and does nothing, and the panel says so.
//!
//! Events ride `UpdateBroadcast` the way log lines do — no `ShowState` entry, nothing
//! replicated, and a page that has not claimed the wing simply does not subscribe.
//! The status is the LOCAL `wing` row, so every page on this station can see whether a
//! wing is here and who has it.

use std::sync::{Arc, Mutex};

use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::wing::WingState,
};
use uuid::Uuid;

use crate::{
    engine::{EngineHandle, UpdateBroadcast},
    infra::clients::short_id,
    wing::WingHandle,
};

/// Where pressed controls are relayed to the page driving the wing.
pub const EVENTS_PATH: &str = "wing_events";

#[derive(Clone)]
pub struct WingStation {
    wing: WingHandle,
    claimed: Arc<Mutex<Option<Uuid>>>,
    engine: EngineHandle,
}

impl WingStation {
    /// Take over the process's wing for this station, dark until a page claims it.
    ///
    /// Dark rather than as the last station left it, because a show was just switched
    /// and whatever the previous page was lighting meant the previous show.
    pub fn new(wing: WingHandle, engine: EngineHandle) -> Self {
        wing.go_dark();
        Self { wing, claimed: Arc::new(Mutex::new(None)), engine }
    }

    /// Follow the wing for as long as the station runs: its status into the `wing`
    /// row, and what is pressed onto the broadcast.
    pub async fn run(self, broadcast: UpdateBroadcast) {
        let mut report = self.wing.report();
        let mut events = self.wing.events();
        self.publish().await;
        // A wing that can never change state — no grandMA3 to boot it from — has a
        // status nobody will write again, and that is no reason to stop relaying.
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
                        // Nobody claiming it is nobody to tell: a wing on a station whose
                        // pages have all gone does nothing, rather than acting for a page
                        // that has not asked.
                        if self.claimed.lock().unwrap().is_none() { continue }
                        if let Ok(value) = serde_json::to_value(&batch) {
                            let path = vec![PathSegment::Key(EVENTS_PATH.into())];
                            let _ = broadcast.0.send((path, value));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("the wing relay fell behind by {n} batches");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    }

    /// This page drives the wing from now on — taking it from another page if one had
    /// it, because the operator standing at the wing is the one who asked.
    pub async fn claim(&self, session: Uuid) -> WingState {
        let previous = self.claimed.lock().unwrap().replace(session);
        if previous != Some(session) {
            self.wing.go_dark();
        }
        self.publish().await
    }

    /// Let go, if this page is the one holding it.
    pub async fn release(&self, session: Uuid) -> WingState {
        self.forget(session).await;
        self.state()
    }

    /// A page has gone. If it was driving the wing, nobody is now.
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
            self.wing.go_dark();
            self.publish().await;
        }
    }

    /// The frame a page wants on the wing: every LED channel, and per fader either a
    /// position for its motor to hold or `None` to leave it to the hand.
    ///
    /// Only from the page driving it, so a second tab cannot paint the desk the first
    /// one is reading.
    pub fn feedback(
        &self,
        session: Uuid,
        leds: Option<Vec<u8>>,
        faders: Option<Vec<Option<u16>>>,
    ) -> Result<(), String> {
        if *self.claimed.lock().unwrap() != Some(session) {
            return Err("this page is not driving the wing".into());
        }
        self.wing.drive(|out| {
            if let Some(leds) = leds {
                for (channel, level) in leds.into_iter().enumerate() {
                    out.set_led(channel, level);
                }
            }
            if let Some(faders) = faders {
                for (index, position) in faders.into_iter().enumerate() {
                    match position {
                        Some(value) => out.set_fader(index, value),
                        None => out.release_fader(index),
                    }
                }
            }
        });
        Ok(())
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
    use futures::StreamExt;
    use pult_schema::{events::operation::NodeId, path::PathPattern, types::wing::WingPhase};
    use pult_wing::Event;

    async fn a_station() -> (WingStation, EngineHandle, UpdateBroadcast, tokio::sync::broadcast::Sender<Vec<Event>>) {
        let pool = Arc::new(showfile::open_in_memory().await.expect("in-memory showfile"));
        let (engine, handle, broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
        tokio::spawn(engine.run());
        let (wing, _report, events) = WingHandle::detached(WingPhase::Running);
        let station = WingStation::new(wing, handle.clone());
        tokio::spawn(station.clone().run(broadcast.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        (station, handle, broadcast, events)
    }

    async fn row(engine: &EngineHandle) -> WingState {
        serde_json::from_value(engine.get(vec![PathSegment::Key("wing".into())]).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn the_row_says_whether_a_wing_is_here_and_who_has_it() {
        let (station, engine, _, _) = a_station().await;
        assert_eq!(row(&engine).await.phase, WingPhase::Running);
        assert_eq!(row(&engine).await.claimed_by, None);

        let page = Uuid::new_v4();
        station.claim(page).await;
        assert_eq!(row(&engine).await.claimed_by, Some(short_id(page)));

        station.forget(page).await;
        assert_eq!(row(&engine).await.claimed_by, None, "a page that goes takes the wing with it");
    }

    /// Nobody claiming it is nobody to act for: what is pressed goes nowhere.
    #[tokio::test]
    async fn a_press_reaches_a_page_only_once_one_has_claimed_the_wing() {
        let (station, _engine, broadcast, events) = a_station().await;
        let mut relayed = broadcast.subscribe_filtered(PathPattern::new(EVENTS_PATH));
        let press = vec![Event::Key { index: 155, down: true }];

        events.send(press.clone()).unwrap();
        let nothing =
            tokio::time::timeout(std::time::Duration::from_millis(100), relayed.next()).await;
        assert!(nothing.is_err(), "unclaimed, a press is relayed to nobody");

        station.claim(Uuid::new_v4()).await;
        events.send(press.clone()).unwrap();
        let got = tokio::time::timeout(std::time::Duration::from_secs(1), relayed.next())
            .await
            .expect("relayed")
            .expect("a value");
        assert_eq!(got, serde_json::to_value(&press).unwrap());
    }

    /// Only the page driving the wing paints it, and when it lets go the desk goes dark
    /// with every motor released.
    #[tokio::test]
    async fn only_the_driving_page_lights_the_wing() {
        let (station, _engine, _, _) = a_station().await;
        let driver = Uuid::new_v4();
        let other = Uuid::new_v4();
        station.claim(driver).await;

        assert!(station.feedback(other, Some(vec![9; 3]), None).is_err());
        station.feedback(driver, Some(vec![9; 3]), Some(vec![Some(2048), None])).unwrap();
        let frame = station.wing.frame();
        assert_eq!(&frame.leds[..3], &[9, 9, 9]);
        assert_eq!(frame.faders[0], Some(2048));

        station.forget(driver).await;
        let frame = station.wing.frame();
        assert!(frame.leds.iter().all(|l| *l == 0));
        assert!(frame.faders.iter().all(Option::is_none));
    }
}
