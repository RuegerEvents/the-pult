//! The panel: one page, one socket.
//!
//! The page is served from the binary the way the console's frontend is, so the tool is
//! one file and a browser. Everything it needs to draw the wing — every control, its
//! index, its name and where it sits — is sent once on connect, because the layout is
//! the map plus geometry and neither changes while the tool runs.

use crate::demos::{Demo, Runner, ALL};
use crate::wing::{self, Command, Report, Shared};
use axum::extract::ws::{Message as Ws, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use pult_wing::map::WingMap;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct App {
    pub map: Arc<WingMap>,
    pub layout: Arc<serde_json::Value>,
    pub out: Shared,
    pub demos: Arc<Runner>,
    pub reports: broadcast::Sender<Report>,
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/ws", get(ws))
        .with_state(app)
}

/// The panel, and **never from a cache**.
///
/// The page is compiled into the binary, so it changes whenever the tool is rebuilt —
/// which during this work was every few minutes. A browser reusing a cached copy looks
/// exactly like a bug in whatever was just changed: the log fills up correctly while
/// the drawing does nothing, because the old script has no idea about the new layout.
/// One header is cheaper than diagnosing that twice.
async fn page() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store, must-revalidate")],
        Html(include_str!("../ui/index.html")),
    )
}

async fn ws(ws: WebSocketUpgrade, State(app): State<App>) -> impl IntoResponse {
    ws.on_upgrade(|socket| talk(socket, app))
}

async fn talk(mut socket: WebSocket, app: App) {
    // Everything static, once: the panel cannot draw anything before it has this and
    // none of it changes while the tool is running.
    let hello = serde_json::json!({
        "kind": "hello",
        "module": app.map.module,
        "layout": *app.layout,
        "hardkeys": app.map.hardkeys,
        "faders": app.map.faders,
        "encoders": app.map.encoders,
        "leds": app.map.leds,
        "demos": ALL.iter().map(|d| serde_json::json!({
            "slug": d.slug(), "describe": d.describe()
        })).collect::<Vec<_>>(),
        "demo": app.demos.current().slug(),
    });
    if socket
        .send(Ws::Text(hello.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    let mut reports = app.reports.subscribe();
    loop {
        tokio::select! {
            r = reports.recv() => match r {
                Ok(report) => {
                    let text = serde_json::to_string(&report).unwrap_or_default();
                    if socket.send(Ws::Text(text.into())).await.is_err() { return; }
                }
                // A slow page misses reports rather than holding up the wing. Input is
                // a stream of the present, and the next frame carries the truth.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Ws::Text(t))) => handle(&app, &t),
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}

fn handle(app: &App, text: &str) {
    #[derive(serde::Deserialize)]
    #[serde(tag = "kind", rename_all = "camelCase")]
    enum In {
        Demo { slug: String },
        #[serde(other)]
        Other,
    }

    // A control command and a demo choice arrive on the same socket; try the wing's
    // vocabulary first and fall back to the tool's own.
    if let Ok(cmd) = serde_json::from_str::<Command>(text) {
        // Anything the operator sets by hand stops the demo painting over it. Note
        // `Manual`, not `Off`: `Off` keeps painting black and flickers the slider.
        app.demos.set(Demo::Manual);
        wing::apply(&app.out, &cmd);
        return;
    }
    if let Ok(In::Demo { slug }) = serde_json::from_str::<In>(text) {
        if let Some(d) = ALL.iter().find(|d| d.slug() == slug) {
            app.demos.set(*d);
        }
    }
}
