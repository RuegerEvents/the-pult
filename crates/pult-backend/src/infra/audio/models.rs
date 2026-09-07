//! The full-accuracy beat model, which is not in the binary.
//!
//! `pult-audio` embeds the log-mel front end and the small beat model, so a console
//! detects beats with no internet and no installation — which is the case that matters,
//! because a venue's network is a thing that may or may not exist. The full model is
//! 83 MB, more than the rest of the console put together, and most rigs will never ask
//! for it.
//!
//! So it is **fetched on request into the station's config directory**, beside
//! `preferences.toml` and for the same reasons the MVR-xchange cache lives there: it is
//! this machine's, not this show's, and a showfile that carried it would carry it into
//! every `.pultz` and across every sync link.
//!
//! **Verified by sha256 before it is written**, which is not a formality. The bytes come
//! off the internet over a link somebody else's proxy may be on, and what they are fed
//! to is an inference runtime parsing a binary format. `pult_audio::beats` owns the
//! checksum, because the crate that knows how to run a model is the one that should say
//! which model it will run.

use std::path::PathBuf;

use serde::Serialize;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Where the fetched model lives.
///
/// `PULT_BEAT_MODEL` names it outright, for the two reasons `PULT_PREFERENCES` is
/// overridable: a test wanting its own, and two consoles on one machine.
pub fn path() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("PULT_BEAT_MODEL") {
        return Some(PathBuf::from(named));
    }
    Some(crate::infra::preferences::config_dir()?.join("the-pult").join("beat_this.onnx"))
}

/// What `timeline.model` answers.
#[derive(Debug, Clone, Serialize)]
pub struct ModelState {
    /// The full model is on this station's disk and will be used.
    pub present: bool,
    /// It is on its way. Ask again.
    pub fetching: bool,
    pub path: Option<String>,
    pub bytes: u64,
    /// What went wrong last time somebody asked.
    pub fault: Option<String>,
}

/// One fetch at a time, per process.
///
/// A guard rather than a queue: two operators pressing the button together should cost
/// one download, and the second should be told it is already happening rather than
/// starting an 83 MB duplicate over the venue's wifi.
static FETCHING: Mutex<Option<String>> = Mutex::const_new(None);

/// The bytes of the full model, where this station has it and it verifies.
///
/// `None` is not a fault: it is a console using the model it was shipped with, which is
/// the ordinary case. A file that is *there* and does not verify **is** a fault and is
/// said out loud, since silently falling back would leave somebody wondering why the
/// model they downloaded made no difference.
pub async fn full_model() -> Option<Vec<u8>> {
    let path = path()?;
    let bytes = tokio::fs::read(&path).await.ok()?;
    if crate::infra::assets::digest(&bytes) != pult_audio::beats::FULL_MODEL_SHA256 {
        warn!("[audio] {} is not the model it should be; using the small one", path.display());
        return None;
    }
    Some(bytes)
}

/// What this station has, right now.
pub async fn state() -> ModelState {
    let fetching = FETCHING.lock().await.clone();
    let Some(path) = path() else {
        return ModelState {
            present: false,
            fetching: false,
            path: None,
            bytes: 0,
            fault: Some("this station has nowhere to keep a model".into()),
        };
    };
    let bytes = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    ModelState {
        present: bytes > 0 && fetching.is_none(),
        fetching: fetching.is_some(),
        path: Some(path.display().to_string()),
        bytes,
        fault: None,
    }
}

/// Fetch it, unless it is already here or already coming.
///
/// Answers immediately with what is happening; the download runs on a task. A caller
/// asks again to find out how it went, which is the shape a browser polling a progress
/// bar wants and avoids holding an RPC open for eighty megabytes.
pub async fn fetch() -> ModelState {
    let existing = state().await;
    if existing.present || existing.fetching {
        return existing;
    }
    let Some(path) = path() else { return existing };

    *FETCHING.lock().await = Some(pult_audio::beats::FULL_MODEL_URL.to_string());
    tokio::spawn(async move {
        let outcome = download(&path).await;
        let mut fetching = FETCHING.lock().await;
        *fetching = None;
        match outcome {
            Ok(bytes) => info!("[audio] the full beat model is here: {bytes} bytes"),
            Err(e) => warn!("[audio] the full beat model could not be fetched: {e}"),
        }
    });
    state().await
}

async fn download(path: &std::path::Path) -> Result<u64, String> {
    let response = reqwest::Client::new()
        .get(pult_audio::beats::FULL_MODEL_URL)
        .timeout(std::time::Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| format!("{e}"))?;
    if !response.status().is_success() {
        return Err(format!("the server answered {}", response.status()));
    }
    let bytes = response.bytes().await.map_err(|e| format!("{e}"))?;
    if crate::infra::assets::digest(&bytes) != pult_audio::beats::FULL_MODEL_SHA256 {
        // Refused rather than kept: what these bytes are fed to is a parser, and a
        // checksum that does not match is the one signal there is.
        return Err("what came back is not the model it should be".into());
    }
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| format!("{e}"))?;
    }
    // Written to a temporary name and renamed, so a half-downloaded file can never be
    // mistaken for a whole one — the rule the asset store already follows.
    let temporary = path.with_extension("onnx.part");
    tokio::fs::write(&temporary, &bytes).await.map_err(|e| format!("{e}"))?;
    tokio::fs::rename(&temporary, path).await.map_err(|e| format!("{e}"))?;
    Ok(bytes.len() as u64)
}
