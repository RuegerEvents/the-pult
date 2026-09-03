//! The asset store: the first bytes in a system that is otherwise all fields.
//!
//! A stage plan is a few megabytes of image. Everything else in a show replicates
//! through the oplog as JSON, and putting an image there would put a copy of it in
//! every operation, every snapshot, and every catch-up. So assets sit beside the
//! show rather than inside it: **files in the bundle's `assets/` directory**, named
//! by the sha256 of their own contents, with the mime and the length kept in a row so
//! the show can be asked about them without reading them.
//!
//! Files rather than blobs, which is where they started. A 256 MB GDTF inside
//! `show.db` is 256 MB every `VACUUM` rewrites, every backup copies and — the one
//! that decided it — every version snapshot duplicates. As files, a show with fifty
//! saved versions holds one copy of each mesh, because a `VACUUM INTO` snapshot
//! shares the folder the assets are in.
//!
//! Content addressing is what makes that safe. The id *is* the check — a station
//! that fetches an asset from a peer verifies what came back before storing it, the
//! same image uploaded twice is stored once, and a URL can be cached forever
//! because its contents cannot change.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{bail, Result};
use chrono::Utc;
use pult_schema::path::PathSegment;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
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

/// A GDTF fixture definition: a zip holding `description.xml` and the meshes and
/// gobo images it names.
///
/// The **file** is the record of an imported fixture type, not the row derived from
/// it: the row is a reading, and a later version of this console will read more out of
/// the same bytes than this one does. Keeping the archive whole is what makes that a
/// re-read rather than a re-download, and what lets the type be exported again exactly
/// as it arrived.
pub const GDTF_MIME: &str = "application/vnd.gdtf+zip";

/// The archive an MVR arrives in, kept whole for the same reason a `.gdtf` is.
pub const MVR_MIME: &str = "application/vnd.mvr-scene+zip";

/// A mesh, as the two formats an MVR carries them in.
pub const GLB_MIME: &str = "model/gltf-binary";
pub const TDS_MIME: &str = "model/3ds";

/// What kind of file a name says it is, for the resources inside an archive.
///
/// By extension, because that is all an archive entry gives: an MVR names its meshes
/// `Geometrie_<uuid>.glb` and its textures `tx603.jpg`, and nothing inside says more.
/// `None` for anything this console will not store, which becomes a warning naming the
/// file rather than a refusal of the rig it was part of.
pub fn mime_for_name(name: &str) -> Option<&'static str> {
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "glb" | "gltf" => GLB_MIME,
        "3ds" => TDS_MIME,
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gdtf" => GDTF_MIME,
        _ => return None,
    })
}

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
    // 256 MB. A GDTF carries a mesh per moving part and an image per gobo, and a
    // detailed moving head from the Share is tens of megabytes before anybody has done
    // anything unusual. The archive's own unpacked ceiling is in `pult-gdtf`, and it
    // is the one that matters: a zip can claim to be small and not be.
    (GDTF_MIME, 256 * 1024 * 1024),
    // 256 MB, the same. An MVR is a GDTF per fixture type plus a mesh per truss
    // section, so it is the larger of the two by construction.
    (MVR_MIME, 256 * 1024 * 1024),
    // 128 MB each. A single mesh out of a real drawing runs to a few megabytes; the
    // 3.8 MB glb in the corpus is the biggest one seen, and this sits well above
    // anything an exporter produces on purpose.
    (GLB_MIME, 128 * 1024 * 1024),
    (TDS_MIME, 128 * 1024 * 1024),
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

/// Where a show's bytes are, and the rows that describe them.
///
/// A pair rather than a bare pool, because the two halves live in different places
/// now and every caller needs both: the row says an asset exists and what kind it
/// is, the file is what it is. Cheap to clone — a path and a pool handle.
#[derive(Clone)]
pub struct AssetStore {
    /// The bundle's `assets/` directory. `None` when **no show is open**: the engine,
    /// the sync layer and the HTTP server all run in that state, serving the welcome
    /// screen, and this is the one thing that has nowhere to put anything. Refusing
    /// here rather than upstream keeps every route and every importer written once.
    dir: Option<PathBuf>,
    pool: Arc<SqlitePool>,
}

impl AssetStore {
    pub fn new(dir: Option<PathBuf>, pool: Arc<SqlitePool>) -> Self {
        AssetStore { dir, pool }
    }

    /// A store with nowhere to write, for a console with no show open.
    pub fn closed(pool: Arc<SqlitePool>) -> Self {
        AssetStore { dir: None, pool }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    fn file(&self, sha: &str) -> Option<PathBuf> {
        // Nothing but hex reaches the filesystem: a sha comes off a URL path, and a
        // name with a separator in it would be a way to read the rest of the disk.
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(self.dir.as_ref()?.join(sha))
    }

    /// Store one asset and return its hash. Storing the same bytes twice is a no-op.
    ///
    /// The file is written first and the row second, which is the safe order: a file
    /// with no row is invisible and costs disk, where a row with no file is an asset
    /// the show believes in and nothing can serve. A half-written file cannot be
    /// mistaken for a whole one either — it goes to a temporary name and is renamed,
    /// and a rename within one directory is atomic.
    pub async fn put(&self, mime: &str, bytes: &[u8]) -> Result<String> {
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
        let Some(path) = self.file(&sha) else {
            bail!("no show is open, so there is nowhere to keep this");
        };
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let staging = path.with_extension(format!("{}.part", std::process::id()));
            std::fs::write(&staging, bytes)?;
            std::fs::rename(&staging, &path)?;
        }

        sqlx::query(
            "INSERT INTO assets (sha256, mime, byte_len, created_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(sha256) DO NOTHING",
        )
        .bind(&sha)
        .bind(mime)
        .bind(bytes.len() as i64)
        .bind(Utc::now().to_rfc3339())
        .execute(&*self.pool)
        .await?;
        Ok(sha)
    }

    /// One asset, or `None` if this station has not got it.
    ///
    /// A row whose file is missing answers `None` rather than an error, which is what
    /// a half-copied folder looks like — and `None` is the answer that sends the
    /// caller down the peer-fetch path, which is exactly the recovery wanted.
    pub async fn get(&self, sha: &str) -> Result<Option<Asset>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT mime FROM assets WHERE sha256 = ?")
            .bind(sha)
            .fetch_optional(&*self.pool)
            .await?;
        let Some((mime,)) = row else { return Ok(None) };
        let Some(path) = self.file(sha) else { return Ok(None) };
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(Asset { mime, bytes })),
            Err(e) => {
                debug!("[assets] {sha} has a row but no file ({e})");
                Ok(None)
            }
        }
    }

    /// Whether the bytes are here, without reading them.
    pub fn holds(&self, sha: &str) -> bool {
        self.file(sha).map(|path| path.exists()).unwrap_or(false)
    }
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

/// How long one peer gets to answer before it counts as unreachable.
///
/// Named rather than written inline because it is half of how long a station can sit
/// in *Fetching*, and the other half is [`crate::infra::plugins::ASK_PEERS_TIMES`].
/// Anything waiting on that state — a test, a panel's patience — has to be able to work
/// the budget out rather than guess it.
pub const PEER_ANSWERS_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

pub async fn fetch_from_peers(
    store: &AssetStore,
    sha: &str,
    peers: &[String],
) -> Result<Fetched> {
    let mut unreachable = 0usize;
    for addr in peers {
        let url = format!("http://{addr}/assets/{sha}");
        let response = match reqwest::Client::new()
            // Stops a ring of stations forwarding one request round for ever: a
            // relayed request is answered from local storage or not at all.
            .get(&url)
            .header("x-pult-asset-relay", "1")
            .timeout(PEER_ANSWERS_WITHIN)
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
        store.put(&mime, &bytes).await?;
        return Ok(Fetched::Got(Asset { mime, bytes: bytes.to_vec() }));
    }
    Ok(if unreachable > 0 { Fetched::Unreachable(unreachable) } else { Fetched::NobodyHasIt })
}

#[cfg(test)]
mod tests;
