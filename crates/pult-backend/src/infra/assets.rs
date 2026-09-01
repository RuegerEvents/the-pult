//! The asset store: the first bytes in a system that is otherwise all fields.
//!
//! A stage plan is a few megabytes of image. Everything else in a show replicates
//! through the oplog as JSON, and putting an image there would put a copy of it in
//! every operation, every snapshot, and every catch-up. So assets sit beside the
//! show rather than inside it: stored as a blob in the same SQLite file, addressed
//! by the sha256 of their own contents, and moved between stations over HTTP only
//! when somebody actually asks for one.
//!
//! Content addressing is what makes that safe. The id *is* the check — a station
//! that fetches an asset from a peer verifies what came back before storing it, the
//! same image uploaded twice is stored once, and a URL can be cached forever
//! because its contents cannot change.

use anyhow::{bail, Result};
use chrono::Utc;
use pult_schema::path::PathSegment;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tracing::debug;

use crate::engine::EngineHandle;

/// What the store holds for one asset.
pub struct Asset {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// A plugin bundle: the zip holding a manifest, a component and any web assets.
///
/// It is stored here rather than in a store of its own because everything that
/// makes this one right for a stage plan is right for a bundle too — the digest is
/// the check, the same bundle carried by two shows is stored once, and a station
/// that has never seen it can fetch it from a peer and verify what came back.
pub const BUNDLE_MIME: &str = "application/vnd.pult.plugin+zip";

/// What may be stored, and how big each kind may be.
///
/// No SVG: it is a document with scripts in it, and serving one from the console's
/// own origin would let an uploaded file run as the console. A PDF never reaches
/// here either — the frontend rasterises page one and uploads that, so the backend
/// and both stage views only ever handle an image.
///
/// A zip does not execute in a browser, so serving one is inert — but it is served
/// as an attachment all the same (see the REST layer), so a bundle can never be
/// navigated to as a document.
pub const ACCEPTED: &[(&str, usize)] = &[
    // 32 MB. A ground plan is a drawing, not a photograph of one.
    ("image/png", 32 * 1024 * 1024),
    ("image/jpeg", 32 * 1024 * 1024),
    ("image/webp", 32 * 1024 * 1024),
    // 64 MB. A component with a JavaScript panel beside it, not a media library —
    // but wasm is bulkier than a drawing, so it gets more room than one.
    (BUNDLE_MIME, 64 * 1024 * 1024),
];

/// The largest anything may be, which is what the HTTP body limit has to be set to.
/// One route takes every kind, so the limit is the widest of them and `put` is what
/// enforces the per-kind ceiling.
pub const MAX_BYTES: usize = {
    let mut max = 0;
    let mut i = 0;
    while i < ACCEPTED.len() {
        if ACCEPTED[i].1 > max {
            max = ACCEPTED[i].1;
        }
        i += 1;
    }
    max
};

/// How big this kind of asset may be, or `None` if it may not be stored at all.
pub fn ceiling_for(mime: &str) -> Option<usize> {
    ACCEPTED.iter().find(|(kind, _)| *kind == mime).map(|(_, max)| *max)
}

pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Store one asset and return its hash. Storing the same bytes twice is a no-op.
pub async fn put(pool: &SqlitePool, mime: &str, bytes: &[u8]) -> Result<String> {
    let Some(ceiling) = ceiling_for(mime) else {
        bail!("{mime} is not something this console will store");
    };
    if bytes.is_empty() {
        bail!("an asset with no bytes in it");
    }
    if bytes.len() > ceiling {
        bail!("{} bytes is more than the {ceiling} a {mime} may be", bytes.len());
    }

    let sha = digest(bytes);
    sqlx::query(
        "INSERT INTO assets (sha256, mime, byte_len, bytes, created_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(sha256) DO NOTHING",
    )
    .bind(&sha)
    .bind(mime)
    .bind(bytes.len() as i64)
    .bind(bytes)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(sha)
}

pub async fn get(pool: &SqlitePool, sha: &str) -> Result<Option<Asset>> {
    let row = sqlx::query("SELECT mime, bytes FROM assets WHERE sha256 = ?")
        .bind(sha)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|row| Asset { mime: row.get("mime"), bytes: row.get("bytes") }))
}

/// Fetch an asset this station has never seen from one that has.
///
/// A plan uploaded on the console at front of house has to reach the one backstage,
/// and it cannot ride the oplog. Every station publishes where its HTTP API is, so
/// the ones that might have it are simply the other rows in `stations`.
///
/// What comes back is hashed before it is stored: a peer cannot answer a request for
/// one asset with a different one, whether by mistake or otherwise.
/// What came of asking the other stations for an asset.
///
/// The two ways of not getting it are worth telling apart. "Nobody has it" is
/// something an operator can act on — install the bundle somewhere. "I could not
/// reach two of them" is a network to look at, and it is also a reason to ask again
/// in a moment rather than give up, because a station that is busy coming up is not
/// a station without the bundle.
pub enum Fetched {
    Got(Asset),
    /// Every peer answered, and none of them had it.
    NobodyHasIt,
    /// This many peers could not be asked at all.
    Unreachable(usize),
}

impl Fetched {
    pub fn asset(self) -> Option<Asset> {
        match self {
            Fetched::Got(asset) => Some(asset),
            _ => None,
        }
    }
}

/// Where the *other* stations serve HTTP, which is where an asset can be had.
///
/// Every station publishes its own row into `stations`, so "the other ones" has to
/// be said out loud or a station asks itself — which it already knows the answer to,
/// since not having the asset locally is what started the search. It was said out
/// loud in one of the two places that wanted it and not the other, and the one that
/// forgot spent a round trip, and then a retry, learning nothing.
pub async fn peer_addresses(engine: &EngineHandle, me: uuid::Uuid) -> Vec<String> {
    let path = vec![PathSegment::Key("stations".into())];
    let Ok(value) = engine.get(path).await else { return Vec::new() };
    let Ok(stations) = serde_json::from_value::<Vec<pult_schema::types::station::Station>>(value)
    else {
        return Vec::new();
    };
    stations
        .into_iter()
        .filter(|s| s.id != me && !s.http_addr.is_empty())
        .map(|s| s.http_addr)
        .collect()
}

pub async fn fetch_from_peers(pool: &SqlitePool, sha: &str, peers: &[String]) -> Result<Fetched> {
    let mut unreachable = 0usize;
    for addr in peers {
        let url = format!("http://{addr}/assets/{sha}");
        let response = match reqwest::Client::new()
            // Stops a ring of stations forwarding one request round for ever: a
            // relayed request is answered from local storage or not at all.
            .get(&url)
            .header("x-pult-asset-relay", "1")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => response,
            // An answer, and the answer is no.
            Ok(_) => continue,
            // Not an answer at all. Counted rather than folded in with the above,
            // because a station that could not be reached has not said it lacks the
            // asset — and reporting that it did would send an operator looking in
            // the wrong place.
            Err(e) => {
                debug!("[assets] {addr} could not be asked for {sha}: {e}");
                unreachable += 1;
                continue;
            }
        };

        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let Ok(bytes) = response.bytes().await else { continue };

        if digest(&bytes) != sha {
            debug!("[assets] {addr} answered {sha} with something else");
            continue;
        }
        put(pool, &mime, &bytes).await?;
        return Ok(Fetched::Got(Asset { mime, bytes: bytes.to_vec() }));
    }
    Ok(if unreachable > 0 { Fetched::Unreachable(unreachable) } else { Fetched::NobodyHasIt })
}

#[cfg(test)]
mod tests;
