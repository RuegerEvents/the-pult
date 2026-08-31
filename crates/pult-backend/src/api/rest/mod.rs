//! The HTTP side of the API, which until now was one WebSocket route.
//!
//! Everything a show is made of travels as JSON over `/ws`. Assets do not: they are
//! bytes, they are large, and they never change once stored — three properties that
//! make an ordinary HTTP request the right shape and the WebSocket the wrong one.
//!
//! `/api/config` is the third thing here, and the smallest: it is how a page that
//! has just been loaded finds out what it loaded from.

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

use crate::{engine::EngineHandle, infra, infra::assets, state::AppState};

/// What a freshly loaded page needs to know about the station that served it.
#[derive(Clone)]
pub struct ConfigState {
    pub node_id: NodeId,
    pub http_port: u16,
    pub sync_port: u16,
}

impl FromRef<AppState> for ConfigState {
    fn from_ref(state: &AppState) -> Self {
        ConfigState {
            node_id: state.node_id,
            http_port: state.http_port,
            sync_port: state.config.sync_port,
        }
    }
}

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

/// Kept apart from [`routes`] because the body limit those two carry is for the
/// one route that takes megabytes, and this one takes nothing at all.
pub fn config_routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ConfigState: FromRef<S>,
{
    Router::new()
        .route("/api/config", get(config))
        // Stateless on purpose: the preferences live in a file, not in `AppState`,
        // so a second console on the same machine sees a change without either of
        // them being restarted, and there is no copy in memory to go stale.
        .route("/api/preferences", get(preferences).put(set_preferences))
}

/// Where the socket is, and what this station is.
///
/// `wsPath` is a path and not a URL on purpose. A station listens on every
/// interface it has, so the only honest answer to "where do I connect" is
/// "wherever you reached me, plus this" — the client joins it to its own origin
/// and is right on the loopback, on the LAN, and behind whatever is in front of us.
async fn config(State(state): State<ConfigState>) -> Json<serde_json::Value> {
    Json(json!({
        "wsPath": "/ws",
        "port": state.http_port,
        "syncPort": state.sync_port,
        "nodeId": state.node_id.0,
        "version": crate::VERSION,
    }))
}

/// What this console prefers, whichever show it has open.
///
/// Not part of `/api/config`, which answers "what did I just load from" and is read
/// once at start-up. These change while the console is running.
async fn preferences() -> Json<serde_json::Value> {
    Json(as_json(&infra::preferences::load()))
}

/// Change them. Answers with what was actually stored, which is not always what was
/// asked for: a depth outside what the console will do comes back at the nearest
/// value that is.
async fn set_preferences(Json(body): Json<serde_json::Value>) -> Result<Json<serde_json::Value>, Response> {
    let asked = infra::preferences::Preferences {
        history_depth: body
            .get("historyDepth")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| bad_request("historyDepth has to be a number"))?,
    }
    .sane();

    infra::preferences::save(&asked).map_err(|e| {
        // Worth an error rather than a silent success: an operator who set something
        // and was told it took wants to know when it did not.
        (StatusCode::INTERNAL_SERVER_ERROR, format!("could not write preferences: {e}")).into_response()
    })?;
    Ok(Json(as_json(&asked)))
}

fn as_json(prefs: &infra::preferences::Preferences) -> serde_json::Value {
    json!({
        "historyDepth": prefs.history_depth,
        "historyDepthMin": pult_schema::types::show::HISTORY_DEPTH_MIN,
        "historyDepthMax": pult_schema::types::show::HISTORY_DEPTH_MAX,
    })
}

fn bad_request(why: &str) -> Response {
    (StatusCode::BAD_REQUEST, why.to_owned()).into_response()
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
    let mut headers = HeaderMap::new();
    if let Ok(mime) = asset.mime.parse() {
        headers.insert(header::CONTENT_TYPE, mime);
    }
    // The name is the contents, so this response can never go stale.
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    // A bundle is inert in a browser, but it is never a document either. Handing it
    // back as an attachment means a link to one can only ever download it.
    if asset.mime == assets::BUNDLE_MIME {
        headers.insert(header::CONTENT_DISPOSITION, header::HeaderValue::from_static("attachment"));
    }
    (headers, asset.bytes).into_response()
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
