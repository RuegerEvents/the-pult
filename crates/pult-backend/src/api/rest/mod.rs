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
    /// One conversation with the GDTF Share for the whole station: the session is a
    /// cookie, and two clients would be two logins where the Share expects one.
    pub share: infra::interop::share::ShareHandle,
}

impl FromRef<AppState> for AssetState {
    fn from_ref(state: &AppState) -> Self {
        AssetState {
            pool: state.pool.clone(),
            engine: state.engine.clone(),
            node_id: state.node_id,
            share: state.share.clone(),
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
        // Installing a plugin is the same shape of request as uploading a plan:
        // bytes in a body, too large for the WebSocket, and the answer is what
        // the show now says about it.
        .route("/api/plugins", post(install_plugin))
        // A fixture definition, in and out. The same shape as a plugin bundle: bytes
        // in a body, too large for the WebSocket, and the answer is what the show now
        // says about it.
        .route("/api/import/gdtf", post(import_gdtf))
        .route("/api/export/gdtf/{fixture_type_id}", get(export_gdtf))
        // The Share. A search is a read and could have been an RPC; importing from it
        // is a write, and the rule that the RPC table is read-only is worth more than
        // the symmetry — so all three are here, where writes belong.
        .route("/api/gdtf-share/status", get(share_status))
        .route("/api/gdtf-share/search", get(share_search))
        .route("/api/gdtf-share/import", post(share_import))
        // Raw bytes rather than multipart: there is one file and its type is in the
        // header, so a form encoding would only be something else to get wrong.
        .layer(axum::extract::DefaultBodyLimit::max(assets::MAX_BYTES))
}

/// Install a plugin from its bundle.
///
/// The order matters and is the whole of why this is a route rather than two.
/// The manifest is read and validated **before** anything is stored, so a
/// rejected upload leaves neither an asset nor a roster row behind — a console
/// that accumulated the bundles it had refused would be a console nobody could
/// explain.
async fn install_plugin(
    State(state): State<AssetState>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, Response> {
    use pult_schema::{lifecycle::Lifecycle, path::PathSegment, types::plugin::PluginPackage};

    // Validated against where it *would* be unpacked, so the relative paths in
    // it are checked the same way they will be when it is.
    let would_be = infra::plugins::cache::root()
        .unwrap_or_else(std::env::temp_dir)
        .join(assets::digest(&body));
    let info = infra::plugins::bundle::read_manifest(&body, &would_be)
        .map_err(|e| bad_request(&format!("{e:#}")))?;
    let manifest = info.manifest;

    let sha256 = assets::put(&state.pool, assets::BUNDLE_MIME, &body)
        .await
        .map_err(|e| bad_request(&e.to_string()))?;

    // One row per plugin id. Replacing rather than adding is what makes
    // installing a new build of something an upgrade instead of a conflict.
    let roster: Vec<PluginPackage> = state
        .engine
        .get(vec![PathSegment::Key("plugin_packages".into())])
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let existing = roster.iter().find(|p| p.plugin_id == manifest.plugin.id);

    let package = PluginPackage {
        id: existing.map(|p| p.id).unwrap_or_else(uuid::Uuid::new_v4),
        plugin_id: manifest.plugin.id.clone(),
        name: manifest.plugin.name.clone(),
        version: manifest.plugin.version.clone(),
        api: manifest.plugin.api.clone(),
        sha256: sha256.clone(),
        // An upgrade keeps whatever the operator had chosen: switching a plugin
        // off and installing a new build of it should not switch it back on.
        enabled: existing.map(|p| p.enabled).unwrap_or(true),
        stage: existing.map(|p| p.stage).unwrap_or_default(),
        config: existing.map(|p| p.config.clone()).unwrap_or(serde_json::Value::Null),
    };
    let value = serde_json::to_value(&package).map_err(|e| bad_request(&e.to_string()))?;

    let path = match existing {
        Some(p) => vec![PathSegment::Key("plugin_packages".into()), PathSegment::Id(p.id)],
        None => vec![
            PathSegment::Key("plugin_packages".into()),
            PathSegment::Key("__create".into()),
        ],
    };
    state
        .engine
        .set(path, Lifecycle::Persisted, value.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())?;

    Ok(Json(json!({ "installed": value, "replaced": existing.is_some() })))
}

/// Whether this console can talk to the Share, and how fresh its list is.
async fn share_status(State(state): State<AssetState>) -> Json<serde_json::Value> {
    Json(state.share.status().await)
}

#[derive(serde::Deserialize)]
struct ShareQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    /// Fetch the list again rather than searching the cached one.
    #[serde(default)]
    refresh: bool,
}

/// Search the Share, locally, over a list fetched at most once a day.
async fn share_search(
    State(state): State<AssetState>,
    axum::extract::Query(query): axum::extract::Query<ShareQuery>,
) -> Result<Json<serde_json::Value>, Response> {
    if query.refresh {
        state.share.list(true).await.map_err(share_error)?;
    }
    let hits = state
        .share
        .search(&query.q, query.manufacturer.as_deref(), query.limit.unwrap_or(50).min(500))
        .await
        .map_err(share_error)?;
    Ok(Json(json!({ "fixtures": hits })))
}

#[derive(serde::Deserialize)]
struct ShareImport {
    rid: u32,
}

/// Download one file from the Share and import it, in one go.
///
/// The same import the upload route runs — the bytes reached this console a different
/// way and nothing after that differs, which is what keeps there from being two import
/// paths to keep in step.
async fn share_import(
    State(state): State<AssetState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<ShareImport>,
) -> Result<Json<serde_json::Value>, Response> {
    let bytes = state.share.download(query.rid).await.map_err(share_error)?;
    let mut answer = import_gdtf_bytes(&state, &headers, Bytes::from(bytes)).await?;
    // Where it came from, so the Fixture Types panel can say "revision 2.0 from the
    // Share" rather than only "GDTF".
    answer["rid"] = json!(query.rid);
    Ok(Json(answer))
}

/// A Share failure in the terms a browser can act on.
fn share_error(error: infra::interop::share::ShareError) -> Response {
    use infra::interop::share::ShareError;
    let status = match error {
        ShareError::NoCredentials | ShareError::BadCredentials => StatusCode::UNAUTHORIZED,
        ShareError::Status(_) | ShareError::Network(_) | ShareError::Unreadable(_) => {
            StatusCode::BAD_GATEWAY
        }
    };
    (status, error.to_string()).into_response()
}

/// The user an HTTP write is attributed to./// The user an HTTP write is attributed to.
///
/// The socket learns who is writing from `Identify`; a request has no such
/// conversation, so the page sends the same id in a header. A request that does not
/// falls back to the show's default operator, exactly as a socket that never
/// identified does — because a write carrying no author can never be taken back, and
/// an import is the last thing an operator should be unable to undo.
fn user_for_writes(headers: &HeaderMap) -> uuid::Uuid {
    headers
        .get("x-pult-user")
        .and_then(|value| value.to_str().ok())
        .and_then(|text| uuid::Uuid::parse_str(text).ok())
        .unwrap_or(pult_schema::types::user::User::DEFAULT_ID)
}

/// Import a fixture definition from a `.gdtf`.
///
/// The order is the same as installing a plugin's, and for the same reason: the file
/// is parsed and read into a plan **before** anything is stored, so a body that is not
/// a GDTF leaves neither an asset nor a row behind.
///
/// The whole import is one gesture, so an operator who imported the wrong file takes
/// it back with one Ctrl-Z.
async fn import_gdtf(
    State(state): State<AssetState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, Response> {
    Ok(Json(import_gdtf_bytes(&state, &headers, body).await?))
}

/// The import itself, shared by the upload route and the Share one.
///
/// How the bytes arrived is the only thing those two differ about, and this is
/// everything after that — so there is one import path rather than two to keep in
/// step.
async fn import_gdtf_bytes(
    state: &AssetState,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<serde_json::Value, Response> {
    use pult_schema::types::fixture::FixtureType;

    if !pult_gdtf::GdtfFile::sniff(&body) {
        // The type header is not enough to decide by: a browser sends
        // `application/octet-stream` for a file it has no type for, and the honest
        // check is whether the bytes are a zip with a description in them.
        return Err(bad_request("this is not a GDTF file: no description.xml in it"));
    }

    let existing: Vec<FixtureType> = state
        .engine
        .get(vec![pult_schema::path::PathSegment::Key("fixture_types".into())])
        .await
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let replaced = {
        let ids: Vec<uuid::Uuid> = existing.iter().map(|each| each.id).collect();
        move |id: uuid::Uuid| ids.contains(&id)
    };

    let (plan, fixture_type_id) = infra::interop::gdtf::plan_import(&body, &existing)
        .map_err(|error| bad_request(&error.to_string()))?;
    let was_there = replaced(fixture_type_id);

    let report =
        infra::interop::apply::apply(plan, &state.pool, &state.engine, user_for_writes(headers))
            .await
            .map_err(|error| {
                (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
            })?;

    Ok(json!({
        "fixture_type_id": fixture_type_id,
        "replaced": was_there,
        "warnings": report.warnings,
    }))
}

/// Hand back a fixture type as a `.gdtf`.
///
/// The archive it was imported from where there is one, and a generated file
/// otherwise — so a type this console made for itself is still something another
/// console can open.
async fn export_gdtf(
    State(state): State<AssetState>,
    Path(fixture_type_id): Path<uuid::Uuid>,
) -> Result<Response, Response> {
    use pult_schema::types::fixture::FixtureType;

    let types: Vec<FixtureType> = state
        .engine
        .get(vec![pult_schema::path::PathSegment::Key("fixture_types".into())])
        .await
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let Some(fixture_type) = types.into_iter().find(|each| each.id == fixture_type_id) else {
        return Err((StatusCode::NOT_FOUND, "no such fixture type").into_response());
    };

    let (bytes, filename) = infra::interop::gdtf::export(&state.pool, &fixture_type)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response())?;

    Ok((
        [
            (header::CONTENT_TYPE, assets::GDTF_MIME.to_string()),
            // An attachment, so a zip can never be navigated to as a document.
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        bytes,
    )
        .into_response())
}

/// Kept apart from [`routes`] because the body limit those two carry is for the/// Kept apart from [`routes`] because the body limit those two carry is for the
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
    // Read, change, write. Building a fresh `Preferences` from the body would
    // quietly drop every setting this route does not mention — the per-plugin
    // configuration among them, which is where an operator's API keys live.
    let mut asked = infra::preferences::load();
    // Each setting is changed only if it is named, so a panel that knows about one
    // of them does not reset the others by not mentioning them.
    let number = |field: &str| -> Result<Option<u32>, Response> {
        match body.get(field) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(v) => v
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .map(Some)
                .ok_or_else(|| bad_request(&format!("{field} has to be a number"))),
        }
    };
    if let Some(depth) = number("historyDepth")? {
        asked.history_depth = depth;
    }
    if let Some(ms) = number("homeFadeMs")? {
        asked.home_fade_ms = ms;
    }

    // The Share login. Named as a pair, so setting one without the other is a
    // half-credential nothing can use; `null` clears it, which is how somebody logs
    // this console out of the Share for good.
    match body.get("gdtfShare") {
        None => {}
        Some(serde_json::Value::Null) => asked.gdtf_share = None,
        Some(value) => {
            let user = value.get("user").and_then(|v| v.as_str()).unwrap_or_default();
            // A blank password means "leave the one you have": a settings form that
            // never shows the password cannot send it back, and re-typing it to change
            // an email address would be a trap.
            let password = value.get("password").and_then(|v| v.as_str());
            let kept = asked.gdtf_share.as_ref().map(|each| each.password.clone());
            let password = match (password, kept) {
                (Some(typed), _) if !typed.is_empty() => typed.to_string(),
                (_, Some(kept)) => kept,
                _ => String::new(),
            };
            if user.is_empty() || password.is_empty() {
                return Err(bad_request("a Share login needs both a user and a password"));
            }
            asked.gdtf_share =
                Some(infra::preferences::ShareCredentials { user: user.to_string(), password });
        }
    }

    let asked = asked.sane();

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
        "homeFadeMs": prefs.home_fade_ms,
        "homeFadeMsMax": pult_schema::types::show::HOME_FADE_MS_MAX,
        // The user, and whether there is a password — never the password. A settings
        // form needs both of those and an onlooker can use neither.
        "gdtfShare": prefs.gdtf_share.as_ref().map(|each| json!({
            "user": each.user,
            "hasPassword": !each.password.is_empty(),
        })),
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
    let peers = assets::peer_addresses(&state.engine, state.node_id.0).await;
    match assets::fetch_from_peers(&state.pool, &sha, &peers).await {
        Ok(fetched) => match fetched.asset() {
            Some(asset) => serve(asset),
            // Whether nobody had it or nobody could be reached, this station does
            // not have it to serve; a viewer asking again is the retry.
            None => StatusCode::NOT_FOUND.into_response(),
        },
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

#[cfg(test)]
mod tests;
