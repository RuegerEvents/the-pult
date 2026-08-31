//! A plugin's web assets, served from its directory.
//!
//! What a web-component panel's `<script>` loads comes from here. Off the disk
//! on every request, not embedded and not cached hard, because the whole point
//! of a plugin directory is that rebuilding it changes the running console.

use axum::{
    body::Body,
    extract::{Path as UrlPath, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/plugins/{id}/assets/{*path}", get(serve))
}

async fn serve(
    UrlPath((id, path)): UrlPath<(String, String)>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    // The id names a loaded plugin; anything else is a 404, not a probe of the
    // filesystem. The path stays inside that plugin's assets directory.
    if path.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(root) = state.plugins.asset_root(id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let file = root.join(&path);

    let Ok(meta) = tokio::fs::metadata(&file).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !meta.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Weak validation from mtime and size: enough for a browser to skip the
    // bytes, cheap enough to answer on every reload of a panel.
    let etag = etag_for(&meta);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|held| held == etag)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let Ok(bytes) = tokio::fs::read(&file).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = mime_guess::from_path(&file).first_or_octet_stream();

    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ETAG, etag)
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn etag_for(meta: &std::fs::Metadata) -> String {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("W/\"{}-{}\"", mtime, meta.len())
}
