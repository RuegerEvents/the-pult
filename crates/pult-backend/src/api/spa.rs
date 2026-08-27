//! The console itself, served by the console.
//!
//! The SvelteKit build is embedded in the binary, so one artifact is the whole
//! thing: no directory to deploy beside it, and nothing to get out of step with
//! the protocol it talks. That also settles where the WebSocket is — the page and
//! the socket come from the same origin, so the frontend can stop being told a
//! port and read `window.location` instead.
//!
//! In a debug build `rust-embed` reads from disk instead of embedding, so
//! `npm run build` shows up on the next request rather than the next `cargo build`.

use axum::{
    body::Body,
    http::{header, HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::{EmbeddedFile, RustEmbed};

#[derive(RustEmbed)]
#[folder = "../../frontend/build"]
struct Frontend;

/// The single-page app's entry point, which is also its 404: the router lives in
/// the browser, so a path this server has never heard of is the frontend's to
/// resolve rather than a missing file.
const INDEX: &str = "index.html";

/// Everything under here is named after its own contents by the bundler, so a
/// response can be cached until the heat death of the venue.
const IMMUTABLE: &str = "_app/immutable/";

pub async fn handler(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { INDEX } else { path };
    // A debug build reads these off the disk rather than out of the binary, so a
    // `..` in the path would be a way out of the build directory.
    if path.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match serve(path, &headers) {
        Some(response) => response,
        // Not a file we hold: hand over the app and let its router decide, which
        // is what makes a deep link work on a reload.
        None => match serve(INDEX, &headers) {
            Some(response) => response,
            None => (
                StatusCode::NOT_FOUND,
                "the frontend was not built into this binary",
            )
                .into_response(),
        },
    }
}

fn serve(path: &str, headers: &HeaderMap) -> Option<Response> {
    let file = Frontend::get(path)?;
    // The content type is the uncompressed file's either way — an encoding is not
    // a type, and saying `application/brotli` here would make a browser download
    // the page instead of rendering it.
    let mime = file.metadata.mimetype().to_string();
    let etag = format!("\"{}\"", hex::encode(file.metadata.sha256_hash()));

    let (file, encoding) = precompressed(path, headers).unwrap_or((file, None));

    let cache = if path.starts_with(IMMUTABLE) {
        "public, max-age=31536000, immutable"
    } else {
        // A console that came back on a new version must not be handed the old
        // page out of a cache, and every one of these is small.
        "no-cache"
    };

    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, cache)
        .header(header::ETAG, etag)
        .header(header::VARY, "accept-encoding");
    if let Some(encoding) = encoding {
        response = response.header(header::CONTENT_ENCODING, encoding);
    }

    Some(
        response
            .body(Body::from(file.data.into_owned()))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    )
}

/// `precompress: true` on the static adapter leaves a `.br` and a `.gz` beside
/// every compressible file, so the bytes on the wire were squeezed once at build
/// time rather than on every request from every tablet in the room.
fn precompressed(path: &str, headers: &HeaderMap) -> Option<(EmbeddedFile, Option<&'static str>)> {
    let accepted = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    for (encoding, suffix) in [("br", ".br"), ("gzip", ".gz")] {
        if accepted.contains(encoding) {
            if let Some(file) = Frontend::get(&format!("{path}{suffix}")) {
                return Some((file, Some(encoding)));
            }
        }
    }
    None
}

/// Whether a real frontend is in here, as opposed to the placeholder `build.rs`
/// leaves behind so that a fresh clone compiles. The bundler's output is the tell:
/// a built frontend always has `_app/`, and the placeholder is one file.
pub fn is_built() -> bool {
    Frontend::iter().any(|f| f.starts_with("_app/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &str) -> Response {
        let uri: Uri = path.parse().unwrap();
        futures::executor::block_on(handler(uri, HeaderMap::new()))
    }

    #[test]
    fn a_path_the_server_has_never_heard_of_gets_the_app() {
        // The router is in the browser, so this is the frontend's problem rather
        // than a 404 — which is what makes a deep link survive a reload.
        let response = get("/sequences/3/cues");
        assert_eq!(response.status(), StatusCode::OK);
        let mime = response.headers().get(header::CONTENT_TYPE).unwrap();
        assert!(mime.to_str().unwrap().starts_with("text/html"));
    }

    #[test]
    fn the_root_is_the_app_too() {
        assert_eq!(get("/").status(), StatusCode::OK);
    }

    #[test]
    fn a_path_cannot_climb_out_of_the_build_directory() {
        // Only reachable in a debug build, where these are read off the disk —
        // which is exactly the build where it would work.
        assert_eq!(
            get("/../../../etc/passwd").status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn there_is_always_something_to_serve() {
        // `build.rs` leaves a placeholder behind when the frontend has not been
        // built, so this holds in a fresh clone as well as a finished one.
        assert!(Frontend::get(INDEX).is_some());
    }
}
