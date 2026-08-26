//! The HTTP side of the API, which until now was one WebSocket route.
//!
//! Everything a show is made of travels as JSON over `/ws`. Assets do not: they are
//! bytes, they are large, and they never change once stored — three properties that
//! make an ordinary HTTP request the right shape and the WebSocket the wrong one.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{FromRef, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use pult_schema::events::operation::NodeId;
use serde_json::json;
use sqlx::SqlitePool;
use tracing::debug;

use crate::{engine::EngineHandle, infra::assets, state::AppState};

/// The part of the console an asset request touches: somewhere to keep bytes, and a
/// way to find out which other stations exist. Narrower than [`AppState`] on purpose
/// — it is the whole surface these two routes need, and a test can build it.
#[derive(Clone)]
pub struct AssetState {
    pub pool: Arc<SqlitePool>,
    pub engine: EngineHandle,
    pub node_id: NodeId,
}

impl FromRef<AppState> for AssetState {
    fn from_ref(state: &AppState) -> Self {
        AssetState {
            pool: state.pool.clone(),
            engine: state.engine.clone(),
            node_id: state.node_id,
        }
    }
}

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AssetState: FromRef<S>,
{
    Router::new()
        .route("/assets", post(upload))
        .route("/assets/{sha}", get(download))
        // Raw bytes rather than multipart: there is one file and its type is in the
        // header, so a form encoding would only be something else to get wrong.
        .layer(axum::extract::DefaultBodyLimit::max(assets::MAX_BYTES))
}

async fn upload(
    State(state): State<AssetState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, Response> {
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        // A browser sends `image/png` bare, but a charset or boundary would ride
        // along on anything else and must not become part of the type.
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    match assets::put(&state.pool, &mime, &body).await {
        Ok(sha) => Ok(Json(json!({ "sha256": sha, "mime": mime, "byte_len": body.len() }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string()).into_response()),
    }
}

async fn download(
    State(state): State<AssetState>,
    Path(sha): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Ok(Some(asset)) = assets::get(&state.pool, &sha).await {
        return serve(asset);
    }

    // A relayed request is answered from local storage or not at all, so a ring of
    // stations cannot forward one request round for ever.
    if headers.contains_key("x-pult-asset-relay") {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Somebody uploaded this on another console. Every station publishes where its
    // HTTP API is, so the ones worth asking are the other rows in `stations`.
    let peers = peer_addresses(&state).await;
    match assets::fetch_from_peers(&state.pool, &sha, &peers).await {
        Ok(Some(asset)) => serve(asset),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            debug!("[assets] fetching {sha} from peers failed: {e}");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

fn serve(asset: assets::Asset) -> Response {
    (
        [
            (header::CONTENT_TYPE, asset.mime),
            // The name is the contents, so this response can never go stale.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string()),
        ],
        asset.bytes,
    )
        .into_response()
}

/// Where the other stations serve HTTP.
async fn peer_addresses(state: &AssetState) -> Vec<String> {
    let path = vec![pult_schema::path::PathSegment::Key("stations".into())];
    let Ok(value) = state.engine.get(path).await else { return Vec::new() };
    let Ok(stations) = serde_json::from_value::<Vec<pult_schema::types::station::Station>>(value)
    else {
        return Vec::new();
    };
    stations
        .into_iter()
        .filter(|s| s.id != state.node_id.0 && !s.http_addr.is_empty())
        .map(|s| s.http_addr)
        .collect()
}

#[cfg(test)]
mod tests;
