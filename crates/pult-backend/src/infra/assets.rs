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
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tracing::debug;

/// What the store holds for one asset.
pub struct Asset {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// The image types a stage plan can be.
///
/// No SVG: it is a document with scripts in it, and serving one from the console's
/// own origin would let an uploaded file run as the console. A PDF never reaches
/// here either — the frontend rasterises page one and uploads that, so the backend
/// and both stage views only ever handle an image.
pub const ACCEPTED: &[&str] = &["image/png", "image/jpeg", "image/webp"];

/// 32 MB. A ground plan is a drawing, not a photograph of one.
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Store one asset and return its hash. Storing the same bytes twice is a no-op.
pub async fn put(pool: &SqlitePool, mime: &str, bytes: &[u8]) -> Result<String> {
    if !ACCEPTED.contains(&mime) {
        bail!("{mime} is not an image this console will serve");
    }
    if bytes.is_empty() {
        bail!("an asset with no bytes in it");
    }
    if bytes.len() > MAX_BYTES {
        bail!("{} bytes is more than the {MAX_BYTES} an asset may be", bytes.len());
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
pub async fn fetch_from_peers(
    pool: &SqlitePool,
    sha: &str,
    peers: &[String],
) -> Result<Option<Asset>> {
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
            Ok(_) => continue,
            Err(e) => {
                debug!("[assets] {addr} could not be asked for {sha}: {e}");
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
        return Ok(Some(Asset { mime, bytes: bytes.to_vec() }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
