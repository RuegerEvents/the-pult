//! The GDTF Share, as a client.
//!
//! gdtf-share.com is where manufacturers publish their fixture definitions. Its public
//! API is three calls behind a session cookie: log in, list everything, download one by
//! revision id. This is that, with the four things a console has to get right about it.
//!
//! **Failure looks like success.** The login endpoint answers 200 with an HTML page
//! when the credentials are wrong. Anything here that decided by status code would
//! report a working login and then fail on every download, so success is decided by
//! the body.
//!
//! **The list is tens of megabytes and unfiltered.** Fetching it per search would make
//! a search box unusable; so it is fetched once, kept in memory *and* on the disk
//! beside the station's preferences, and searched locally. It is refreshed when the
//! Share's own timestamp moves past the one that came with the cached copy, when it is
//! a day old, or when somebody asks.
//!
//! **A session goes idle after about two hours.** A request that comes back
//! unauthorised logs in again and retries, once. Twice would be a loop against
//! somebody else's server.
//!
//! **Credentials live in the station's preferences and never in the show.** A showfile
//! travels; a password in one travels with it.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::infra::preferences::ShareCredentials;

/// Where the Share's public API lives.
///
/// Overridable so the tests can point it at a stub of the three endpoints, which is
/// the only way to test the re-login and the caching without an account and a network.
pub const DEFAULT_BASE: &str = "https://gdtf-share.com/apis/public";

/// How stale the cached list may get before it is fetched again without being asked.
///
/// A day. Manufacturers publish revisions in ones and twos; nothing about a rig
/// depends on having yesterday's list, and re-fetching tens of megabytes more often
/// than this would be rude to somebody else's server.
const LIST_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// One fixture definition on the Share.
///
/// Every number is read leniently, because the Share does not always send one: an
/// unrated file's `rating` is the *string* `"N/A"`, and a strict reader fails the whole
/// list on it. This is somebody else's server and the right posture towards its answers
/// is to take what can be read and say how much could not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShareFixture {
    /// The revision id, which is what a download asks for.
    #[serde(deserialize_with = "lenient_u32")]
    pub rid: u32,
    #[serde(default)]
    pub fixture: String,
    #[serde(default)]
    pub manufacturer: String,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub creator: String,
    /// Out of five, where anybody has rated it. `None` for the many that nobody has —
    /// which the Share says with `"N/A"` rather than with a null.
    #[serde(default, deserialize_with = "lenient_f32")]
    pub rating: Option<f32>,
    /// The modes the file declares, with the footprint of each. What tells two
    /// revisions of one fixture apart when the name cannot.
    #[serde(default)]
    pub modes: Vec<ShareMode>,
    #[serde(default)]
    pub version: String,
    /// Who put this revision on the Share: `"Manuf."` where the manufacturer did, and
    /// `"User"` where somebody else did.
    ///
    /// The Share's own answer to "which of these seven MegaPointes is the real one",
    /// and worth more than any ranking this console could invent: a search puts the
    /// manufacturer's file first and the panel says which it is.
    #[serde(default)]
    pub uploader: String,
}

impl ShareFixture {
    /// Whether the manufacturer published this one themselves.
    pub fn from_manufacturer(&self) -> bool {
        self.uploader.trim().starts_with("Manuf")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShareMode {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "dmxfootprint", deserialize_with = "lenient_u32")]
    pub dmx_footprint: u32,
}

/// A JSON value that might be a number, might be the text of one, and might be neither.
#[derive(Deserialize)]
#[serde(untagged)]
enum Loose {
    Number(f64),
    Text(String),
    /// Anything else the Share might put there. Kept so an unexpected shape reads as
    /// "no number" rather than failing the row it is in.
    Other(serde::de::IgnoredAny),
}

impl Loose {
    fn number(self) -> Option<f64> {
        match self {
            Loose::Number(number) => Some(number),
            Loose::Text(text) => text.trim().parse().ok(),
            Loose::Other(_) => None,
        }
    }
}

fn lenient_f32<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f32>, D::Error> {
    Ok(Option::<Loose>::deserialize(deserializer)?
        .and_then(Loose::number)
        .map(|number| number as f32))
}

fn lenient_u32<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    Ok(Option::<Loose>::deserialize(deserializer)?
        .and_then(Loose::number)
        .filter(|number| *number >= 0.0)
        .map(|number| number as u32)
        .unwrap_or(0))
}

/// The list, and when it was taken.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedList {
    /// The Share's own `list_timestamp`, so a refresh can tell whether anything moved.
    #[serde(default)]
    pub timestamp: i64,
    /// Unix seconds when this console fetched it, for the age check.
    #[serde(default)]
    pub fetched_at: u64,
    #[serde(default)]
    pub fixtures: Vec<ShareFixture>,
}

impl CachedList {
    fn age(&self) -> Duration {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        Duration::from_secs(now.saturating_sub(self.fetched_at))
    }
}

/// What went wrong talking to the Share.
#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    #[error("no GDTF Share login on this console; set one in the Fixture Types panel")]
    NoCredentials,
    #[error("the GDTF Share rejected that login")]
    BadCredentials,
    #[error("the GDTF Share answered {0}")]
    Status(u16),
    #[error("could not reach the GDTF Share: {0}")]
    Network(String),
    #[error("the GDTF Share answered something this console could not read: {0}")]
    Unreadable(String),
}

/// A logged-in conversation with the Share, or one that can start itself.
///
/// One per station, shared: the cookie jar is the session, and two of these would be
/// two logins where the Share expects one.
#[derive(Clone)]
pub struct ShareHandle(Arc<Mutex<Share>>);

struct Share {
    base: String,
    client: reqwest::Client,
    /// True once a login has succeeded on this client's cookie jar.
    signed_in: bool,
    list: Option<CachedList>,
    /// Where to keep the list between runs. `None` disables the disk half, which is
    /// what the tests use.
    cache_path: Option<std::path::PathBuf>,
}

impl ShareHandle {
    /// A client pointed at the real Share, caching beside the station's preferences.
    pub fn new() -> Self {
        Self::with_base(
            DEFAULT_BASE,
            crate::infra::preferences::config_dir()
                .map(|dir| dir.join("the-pult").join("gdtf-share-list.json")),
        )
    }

    /// The same, pointed somewhere else. For the tests, and for anybody running a
    /// mirror.
    pub fn with_base(base: &str, cache_path: Option<std::path::PathBuf>) -> Self {
        // A cookie jar, because the session *is* a cookie: there is no token to carry.
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .timeout(Duration::from_secs(120))
            .build()
            .expect("a client with no TLS surprises in it");
        ShareHandle(Arc::new(Mutex::new(Share {
            base: base.trim_end_matches('/').to_string(),
            client,
            signed_in: false,
            list: None,
            cache_path,
        })))
    }

    /// Whether this console can talk to the Share, and how fresh its list is.
    pub async fn status(&self) -> serde_json::Value {
        let share = self.0.lock().await;
        let credentials = crate::infra::preferences::load().gdtf_share;
        serde_json::json!({
            "configured": credentials.is_some(),
            "user": credentials.map(|each| each.user),
            "signedIn": share.signed_in,
            "listSize": share.list.as_ref().map(|list| list.fixtures.len()).unwrap_or(0),
            "listAgeSeconds": share.list.as_ref().map(|list| list.age().as_secs()),
        })
    }

    /// Everything on the Share, from the cache where that is still good enough.
    pub async fn list(&self, refresh: bool) -> Result<Vec<ShareFixture>, ShareError> {
        let mut share = self.0.lock().await;
        share.load_cache_from_disk();
        let stale = share
            .list
            .as_ref()
            .map(|list| list.age() > LIST_MAX_AGE)
            .unwrap_or(true);
        if refresh || stale {
            share.fetch_list().await?;
        }
        Ok(share.list.as_ref().map(|list| list.fixtures.clone()).unwrap_or_default())
    }

    /// The rows matching a query, searched locally.
    ///
    /// Locally because the list is one fetch and tens of thousands of rows: a search
    /// box that went to the network per keystroke would be unusable and would be
    /// somebody else's bandwidth.
    ///
    /// **Word by word, and ignoring the spaces.** The Share's own name for a fixture is
    /// whatever its uploader typed — "Robin MegaPointe", by "Robe Lighting" — and an
    /// operator types "mega pointe", or "robe megapointe", or "megapointe robe". A
    /// single substring match answers nothing for all three, which is a search box that
    /// looks broken while holding the fixture.
    pub async fn search(
        &self,
        query: &str,
        manufacturer: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ShareFixture>, ShareError> {
        let all = self.list(false).await?;
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let maker = manufacturer.map(str::to_lowercase);

        let mut hits: Vec<ShareFixture> = all
            .into_iter()
            .filter(|row| {
                maker.as_ref().is_none_or(|maker| row.manufacturer.to_lowercase() == *maker)
            })
            .filter(|row| matches(row, &words))
            .collect();

        // The manufacturer's own file first, then best rated, then alphabetical.
        //
        // "megapointe" answers seven files from six people, and the Share already knows
        // which of them is authoritative — `uploader` is `"Manuf."` on exactly one. A
        // ranking this console invented out of where the words matched got that wrong
        // for "robe mega pointe", putting somebody's copy above Robe's, because the
        // copy had the word "Robe" in its *name*.
        hits.sort_by(|a, b| {
            b.from_manufacturer()
                .cmp(&a.from_manufacturer())
                .then_with(|| b.rating.unwrap_or(0.0).total_cmp(&a.rating.unwrap_or(0.0)))
                .then_with(|| a.manufacturer.cmp(&b.manufacturer))
                .then_with(|| a.fixture.cmp(&b.fixture))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    /// One file's bytes.
    pub async fn download(&self, rid: u32) -> Result<Vec<u8>, ShareError> {
        let mut share = self.0.lock().await;
        share.download(rid).await
    }
}

/// Whether a row answers a query.
///
/// Every word has to be found, in the fixture's name or its manufacturer's — otherwise
/// a two-word query matches half the catalogue.
///
/// Each word is looked for in the text *and* in the text with its spaces taken out, so
/// "mega pointe" finds "MegaPointe" and "robe megapointe" finds "Robin MegaPointe" by
/// "Robe Lighting". Somebody typing a fixture's name should not have to know how its
/// uploader spaced it.
fn matches(row: &ShareFixture, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let full = format!("{} {}", row.manufacturer.to_lowercase(), row.fixture.to_lowercase());
    let squashed = squash(&full);
    words
        .iter()
        .all(|word| full.contains(word.as_str()) || squashed.contains(&squash(word)))
}

/// The same text with nothing separating its words.
fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '_').collect()
}

impl Default for ShareHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl Share {
    /// Log in, and say whether it took.
    ///
    /// Decided by the body: the endpoint answers 200 with an HTML page on a bad
    /// password, so a status check here would report a working login and fail on every
    /// download after it.
    async fn sign_in(&mut self) -> Result<(), ShareError> {
        let credentials: ShareCredentials =
            crate::infra::preferences::load().gdtf_share.ok_or(ShareError::NoCredentials)?;

        let answer = self
            .client
            .post(format!("{}/login.php", self.base))
            .form(&[("user", credentials.user.as_str()), ("password", credentials.password.as_str())])
            .send()
            .await
            .map_err(|e| ShareError::Network(e.to_string()))?;

        let body = answer.text().await.map_err(|e| ShareError::Network(e.to_string()))?;
        let ok = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value.get("result").and_then(|r| r.as_bool()))
            .unwrap_or(false);
        if !ok {
            self.signed_in = false;
            return Err(ShareError::BadCredentials);
        }
        self.signed_in = true;
        Ok(())
    }

    /// Make a request, logging in first if we have not, and once more if the session
    /// has gone idle.
    ///
    /// Once and not in a loop: a second unauthorised answer after a fresh login means
    /// something is wrong that retrying will not fix, and hammering somebody else's
    /// server is not this console's to do.
    async fn with_session<T, F, Fut>(&mut self, make: F) -> Result<T, ShareError>
    where
        F: Fn(reqwest::Client, String) -> Fut,
        Fut: std::future::Future<Output = Result<Option<T>, ShareError>>,
    {
        if !self.signed_in {
            self.sign_in().await?;
        }
        if let Some(answer) = make(self.client.clone(), self.base.clone()).await? {
            return Ok(answer);
        }
        // Unauthorised: the two-hour idle timeout, almost certainly.
        self.signed_in = false;
        self.sign_in().await?;
        make(self.client.clone(), self.base.clone())
            .await?
            .ok_or(ShareError::Status(401))
    }

    async fn fetch_list(&mut self) -> Result<(), ShareError> {
        let body: serde_json::Value = self
            .with_session(|client, base| async move {
                let answer = client
                    .get(format!("{base}/getList.php"))
                    .send()
                    .await
                    .map_err(|e| ShareError::Network(e.to_string()))?;
                if answer.status() == reqwest::StatusCode::UNAUTHORIZED {
                    return Ok(None);
                }
                if !answer.status().is_success() {
                    return Err(ShareError::Status(answer.status().as_u16()));
                }
                let text = answer.text().await.map_err(|e| ShareError::Network(e.to_string()))?;
                serde_json::from_str(&text)
                    .map(Some)
                    .map_err(|e| ShareError::Unreadable(e.to_string()))
            })
            .await?;

        // A body that says `result: false` is the Share refusing, whatever its status
        // said — the same rule as the login.
        if body.get("result").and_then(|r| r.as_bool()) == Some(false) {
            return Err(ShareError::BadCredentials);
        }

        // Row by row, so one entry this console cannot read costs that entry and not
        // the whole catalogue. The Share has tens of thousands of them and they are
        // not all the same shape; failing all of them over one is not a trade worth
        // making, and a row with no usable `rid` cannot be downloaded anyway.
        let rows = body.get("list").and_then(|list| list.as_array()).cloned().unwrap_or_default();
        let total = rows.len();
        let fixtures: Vec<ShareFixture> = rows
            .into_iter()
            .filter_map(|row| serde_json::from_value::<ShareFixture>(row).ok())
            .filter(|row| row.rid != 0)
            .collect();
        if fixtures.len() < total {
            tracing::warn!(
                skipped = total - fixtures.len(),
                of = total,
                "[gdtf-share] some rows of the list were not in a shape this console reads"
            );
        }

        let list = CachedList {
            timestamp: body.get("list_timestamp").and_then(|v| v.as_i64()).unwrap_or_default(),
            fetched_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            fixtures,
        };
        self.write_cache_to_disk(&list);
        self.list = Some(list);
        Ok(())
    }

    async fn download(&mut self, rid: u32) -> Result<Vec<u8>, ShareError> {
        let bytes: Vec<u8> = self
            .with_session(move |client, base| async move {
                let answer = client
                    .get(format!("{base}/downloadFile.php"))
                    .query(&[("rid", rid)])
                    .send()
                    .await
                    .map_err(|e| ShareError::Network(e.to_string()))?;
                if answer.status() == reqwest::StatusCode::UNAUTHORIZED {
                    return Ok(None);
                }
                if !answer.status().is_success() {
                    return Err(ShareError::Status(answer.status().as_u16()));
                }
                let bytes = answer
                    .bytes()
                    .await
                    .map_err(|e| ShareError::Network(e.to_string()))?
                    .to_vec();
                // The Share answers a JSON error body with a 200 here too, so the
                // check is again on the content: a `.gdtf` starts with a zip header.
                if !bytes.starts_with(b"PK") {
                    return Ok(None);
                }
                Ok(Some(bytes))
            })
            .await?;
        Ok(bytes)
    }

    fn load_cache_from_disk(&mut self) {
        if self.list.is_some() {
            return;
        }
        let Some(path) = &self.cache_path else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        if let Ok(cached) = serde_json::from_str::<CachedList>(&text) {
            self.list = Some(cached);
        }
    }

    fn write_cache_to_disk(&self, list: &CachedList) {
        let Some(path) = &self.cache_path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Best effort: a console that cannot cache the list still works, it just
        // fetches it again next time.
        if let Ok(text) = serde_json::to_string(list) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests;
