//! `/ws/monitor`: a timeline's sound for a page to listen to while somebody programs.
//!
//! A socket of its own rather than a path on the show socket: the chunks are binary and
//! twenty a second, and the show socket carries JSON a page parses on its main thread.
//! Opening it is asking; closing it, or the page going, is the end of the rendering —
//! see `infra/audio/monitor.rs`, which says why any station can serve it.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct Ask {
    /// One timeline, or every running one when left out.
    timeline: Option<Uuid>,
}

pub async fn monitor_handler(
    ws: WebSocketUpgrade,
    Query(ask): Query<Ask>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| listen(socket, state, ask.timeline))
}

async fn listen(socket: WebSocket, state: AppState, timeline: Option<Uuid>) {
    let (mut sink, mut stream) = socket.split();
    let Some(audio) = state.audio.clone() else {
        let _ = sink.send(Message::Close(None)).await;
        return;
    };
    let Some(mut chunks) = audio.monitor(timeline).await else { return };
    let mut stopping = state.stopping.clone();
    loop {
        tokio::select! {
            chunk = chunks.recv() => {
                let Some(chunk) = chunk else { break };
                if sink.send(Message::Binary(chunk.into())).await.is_err() {
                    break;
                }
            }
            // The page says nothing on this socket; anything from it is the close.
            got = stream.next() => match got {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = stopping.changed() => break,
        }
    }
    let _ = sink.send(Message::Close(None)).await;
}
