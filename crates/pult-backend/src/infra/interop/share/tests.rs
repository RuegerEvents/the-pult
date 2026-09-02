//! The Share client, against a stub of the three endpoints it uses.
//!
//! A stub rather than the real Share, because the behaviour worth testing is exactly
//! the behaviour an account cannot demonstrate on demand: a login that answers 200
//! with a failure in the body, a session that has gone idle, a list that has not
//! changed. All three are one line to write here and impossible to arrange there.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::response::IntoResponse;
use axum::{
    extract::State,
    routing::{get, post},
    Router,
};

use super::*;

/// What the stub has been asked, so a test can assert about the *shape* of the
/// conversation rather than only its answers.
#[derive(Default)]
struct Stub {
    logins: AtomicUsize,
    lists: AtomicUsize,
    downloads: AtomicUsize,
    /// Refuse everything until a login has happened, and again once `expire_after`
    /// requests have gone by — which is what a two-hour idle timeout looks like from
    /// here.
    signed_in: std::sync::Mutex<bool>,
    expire_after: AtomicUsize,
    /// Answer the login with a failure, in a 200, the way the Share does.
    reject_login: std::sync::Mutex<bool>,
}

type Shared = Arc<Stub>;

async fn login(State(stub): State<Shared>, body: String) -> String {
    stub.logins.fetch_add(1, Ordering::SeqCst);
    if *stub.reject_login.lock().unwrap() || !body.contains("user=") {
        // 200, with the failure in the body. This is the whole reason the client
        // decides by content.
        return r#"{"result":false,"error":"wrong credentials"}"#.into();
    }
    *stub.signed_in.lock().unwrap() = true;
    stub.expire_after.store(usize::MAX, Ordering::SeqCst);
    r#"{"result":true}"#.into()
}

/// Whether this request is inside a live session, counting it against the expiry.
fn alive(stub: &Stub) -> bool {
    if !*stub.signed_in.lock().unwrap() {
        return false;
    }
    let left = stub.expire_after.load(Ordering::SeqCst);
    if left == 0 {
        *stub.signed_in.lock().unwrap() = false;
        return false;
    }
    if left != usize::MAX {
        stub.expire_after.store(left - 1, Ordering::SeqCst);
    }
    true
}

async fn get_list(State(stub): State<Shared>) -> axum::response::Response {
    stub.lists.fetch_add(1, Ordering::SeqCst);
    if !alive(&stub) {
        return (axum::http::StatusCode::UNAUTHORIZED, "").into_response();
    }
    // Deliberately as ragged as the real thing. `"N/A"` where a rating should be is
    // what the Share actually sends for a file nobody has rated, and it failed the
    // whole list before this was read leniently; the footprint as a string and the row
    // with no usable `rid` are the same class of thing.
    (
        [("content-type", "application/json")],
        r#"{"result":true,"list_timestamp":1700000000,"list":[
            {"rid":11,"fixture":"Robin Spiider","manufacturer":"Robe Lighting","revision":"1.2",
             "rating":4.5,"modes":[{"name":"Mode 1","dmxfootprint":49}]},
            {"rid":12,"fixture":"Robin MegaPointe","manufacturer":"Robe Lighting","revision":"2.0",
             "rating":"N/A","uploader":"Manuf.",
             "modes":[{"name":"Standard","dmxfootprint":"39"}]},
            {"rid":14,"fixture":"Robe MegaPointe","manufacturer":"Somebody Else","revision":"1.0",
             "rating":4.8,"uploader":"User","modes":[{"name":"Mode 1","dmxfootprint":39}]},
            {"rid":13,"fixture":"Sharpy","manufacturer":"Clay Paky","revision":"1.0","rating":3.1,
             "modes":[{"name":"Basic","dmxfootprint":16}]},
            {"rid":"N/A","fixture":"Nothing anybody can download","manufacturer":"Nobody"}
        ]}"#,
    )
        .into_response()
}

async fn download(State(stub): State<Shared>) -> axum::response::Response {
    stub.downloads.fetch_add(1, Ordering::SeqCst);
    if !alive(&stub) {
        return (axum::http::StatusCode::UNAUTHORIZED, "").into_response();
    }
    // A zip header is all the client checks for, and all a test needs to be.
    (axum::http::StatusCode::OK, axum::body::Bytes::from_static(b"PK\x03\x04 pretend gdtf"))
        .into_response()
}

/// The stub, running, and where to reach it.
async fn a_stub() -> (Shared, String) {
    let stub: Shared = Arc::new(Stub::default());
    let app = Router::new()
        .route("/login.php", post(login))
        .route("/getList.php", get(get_list))
        .route("/downloadFile.php", get(download))
        .with_state(stub.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (stub, format!("http://{addr}"))
}

/// The stub, and a client pointed at it with no disk cache.
async fn a_share() -> (ShareHandle, Shared) {
    let (stub, base) = a_stub().await;
    (ShareHandle::with_base(&base, None), stub)
}

/// A station with a Share login in its preferences, for the length of a test.
fn signed_in() -> crate::infra::preferences::testing::OwnFile {
    let own = crate::infra::preferences::testing::own_file();
    let prefs = crate::infra::preferences::Preferences {
        gdtf_share: Some(ShareCredentials {
            user: "somebody@example.com".into(),
            password: "hunter2".into(),
        }),
        ..Default::default()
    };
    crate::infra::preferences::save(&prefs).unwrap();
    own
}

#[tokio::test]
async fn with_no_login_on_this_console_the_share_says_so_rather_than_asking() {
    let _own = crate::infra::preferences::testing::own_file();
    let (share, stub) = a_share().await;

    let error = share.list(true).await.unwrap_err();
    assert!(matches!(error, ShareError::NoCredentials), "{error}");
    assert_eq!(
        stub.logins.load(Ordering::SeqCst),
        0,
        "and it does not go to somebody else's server to find out",
    );
}

#[tokio::test]
async fn a_login_the_share_refuses_in_a_two_hundred_is_still_a_refusal() {
    let _own = signed_in();
    let (share, stub) = a_share().await;
    *stub.reject_login.lock().unwrap() = true;

    let error = share.list(true).await.unwrap_err();
    assert!(
        matches!(error, ShareError::BadCredentials),
        "the failure is in the body, not the status: {error}",
    );
}

#[tokio::test]
async fn the_list_is_fetched_once_and_searched_locally_after_that() {
    let _own = signed_in();
    let (share, stub) = a_share().await;

    let all = share.list(true).await.unwrap();
    assert_eq!(all.len(), 4, "four usable rows out of five");
    assert_eq!(stub.logins.load(Ordering::SeqCst), 1);

    for _ in 0..5 {
        assert_eq!(share.search("robe lighting", None, 10).await.unwrap().len(), 2);
    }
    assert_eq!(
        stub.lists.load(Ordering::SeqCst),
        1,
        "a search box that fetched tens of megabytes per keystroke would be unusable",
    );
}

#[tokio::test]
async fn a_search_finds_by_either_name_and_answers_best_rated_first() {
    let _own = signed_in();
    let (share, _stub) = a_share().await;
    share.list(true).await.unwrap();

    let by_fixture = share.search("sharpy", None, 10).await.unwrap();
    assert_eq!(by_fixture.len(), 1);
    assert_eq!(by_fixture[0].manufacturer, "Clay Paky");
    assert_eq!(by_fixture[0].modes[0].dmx_footprint, 16);

    let by_maker = share.search("robe lighting", None, 10).await.unwrap();
    assert_eq!(
        by_maker.iter().map(|row| row.fixture.as_str()).collect::<Vec<_>>(),
        vec!["Robin MegaPointe", "Robin Spiider"],
        "the manufacturer's own first, then by rating",
    );

    assert_eq!(share.search("", Some("Clay Paky"), 10).await.unwrap().len(), 1);
    assert_eq!(share.search("", None, 2).await.unwrap().len(), 2, "the limit is honoured");
}

/// What somebody types, against what an uploader typed.
///
/// The Share's name for this fixture is "Robin MegaPointe" and its manufacturer is
/// "Robe Lighting". Every line below is a reasonable thing to type for it, and a
/// single substring match answers nothing for four of them.
#[tokio::test]
async fn a_search_finds_a_fixture_however_the_words_were_spaced() {
    let _own = signed_in();
    let (share, _stub) = a_share().await;

    for typed in [
        "megapointe",
        "mega pointe",
        "MEGA POINTE",
        "robe megapointe",
        "megapointe robe",
        "robin mega pointe",
    ] {
        let hits = share.search(typed, None, 10).await.unwrap();
        assert!(!hits.is_empty(), "{typed:?} found nothing");
        assert_eq!(
            hits[0].fixture, "Robin MegaPointe",
            "{typed:?} did not put the manufacturer's own file first",
        );
    }

    assert!(
        share.search("mega sharpy", None, 10).await.unwrap().is_empty(),
        "every word has to be found, or a two-word query would match half the catalogue",
    );
}

/// Which of seven MegaPointes is the real one.
///
/// The Share answers that itself — `uploader` is `"Manuf."` on exactly one of them —
/// and that is worth more than any ranking this console could invent. A ranking built
/// out of *where* the words matched got it backwards for "robe mega pointe": somebody
/// else's copy has "Robe" in its own name, and the manufacturer's has it only in the
/// manufacturer field.
#[tokio::test]
async fn the_manufacturers_own_file_comes_first_even_when_a_copy_is_better_rated() {
    let _own = signed_in();
    let (share, _stub) = a_share().await;

    let hits = share.search("robe mega pointe", None, 10).await.unwrap();
    assert_eq!(hits.len(), 2, "two files answer to that");
    assert_eq!(hits[0].manufacturer, "Robe Lighting");
    assert!(hits[0].from_manufacturer(), "and it is the one Robe published");
    assert_eq!(hits[0].rating, None, "despite the copy below it being rated 4.8");
    assert_eq!(hits[1].manufacturer, "Somebody Else");
}

/// The row that used to fail the whole catalogue.
///
/// A file nobody has rated has `"rating":"N/A"` — a string where a number belongs —
/// and a strict reader answered "the GDTF Share answered something this console could
/// not read" for the entire list, whatever was being searched for.
#[tokio::test]
async fn a_row_the_share_wrote_ragged_is_read_rather_than_failing_the_list() {
    let _own = signed_in();
    let (share, _stub) = a_share().await;

    let hits = share.search("megapointe", None, 10).await.unwrap();
    assert!(!hits.is_empty(), "the search that used to answer an error");
    let ragged = hits.iter().find(|row| row.manufacturer == "Robe Lighting").unwrap();
    assert_eq!(ragged.rating, None, "\"N/A\" is no rating, not a parse failure");
    assert_eq!(
        ragged.modes[0].dmx_footprint, 39,
        "and a footprint written as text is still a footprint",
    );
}

/// A row with nothing usable in it is left out rather than kept as a broken one.
#[tokio::test]
async fn a_row_with_no_revision_id_is_dropped_because_nothing_could_download_it() {
    let _own = signed_in();
    let (share, _stub) = a_share().await;

    let hits = share.search("nothing anybody", None, 10).await.unwrap();
    assert!(hits.is_empty(), "an Import button on it would have nothing to ask for");
    assert_eq!(share.list(false).await.unwrap().len(), 4, "and the other four are fine");
}

#[tokio::test]
async fn a_session_that_has_gone_idle_is_re_established_once_and_the_request_goes_through() {
    let _own = signed_in();
    let (share, stub) = a_share().await;

    share.list(true).await.unwrap();
    assert_eq!(stub.logins.load(Ordering::SeqCst), 1);

    // Two hours of nothing, as the Share sees it.
    stub.expire_after.store(0, Ordering::SeqCst);

    let bytes = share.download(11).await.unwrap();
    assert!(bytes.starts_with(b"PK"), "the file came back after the re-login");
    assert_eq!(stub.logins.load(Ordering::SeqCst), 2, "logged in again, once");
    assert_eq!(stub.downloads.load(Ordering::SeqCst), 2, "the refused try, then the real one");
}

#[tokio::test]
async fn a_download_answers_the_files_bytes() {
    let _own = signed_in();
    let (share, _stub) = a_share().await;
    assert!(share.download(12).await.unwrap().starts_with(b"PK\x03\x04"), "a .gdtf is a zip");
}

#[tokio::test]
async fn the_status_says_what_a_settings_panel_needs_and_never_the_password() {
    let _own = signed_in();
    let (share, _stub) = a_share().await;

    let before = share.status().await;
    assert_eq!(before["configured"], true);
    assert_eq!(before["user"], "somebody@example.com");
    assert_eq!(before["signedIn"], false, "nothing has been asked of it yet");
    assert!(
        !before.to_string().contains("hunter2"),
        "the password is written down and never read back out: {before}",
    );

    share.list(true).await.unwrap();
    let after = share.status().await;
    assert_eq!(after["signedIn"], true);
    assert_eq!(after["listSize"], 4);
    assert!(after["listAgeSeconds"].as_u64().unwrap() < 5);
}

#[tokio::test]
async fn a_list_kept_on_the_disk_survives_a_restart_without_being_fetched_again() {
    let _own = signed_in();
    let dir = std::env::temp_dir().join(format!("pult-share-{}", uuid::Uuid::new_v4()));
    let cache = dir.join("gdtf-share-list.json");
    let (stub, base) = a_stub().await;

    let first = ShareHandle::with_base(&base, Some(cache.clone()));
    first.list(true).await.unwrap();
    assert_eq!(stub.lists.load(Ordering::SeqCst), 1);
    assert!(cache.exists(), "the list was written down");

    // A second console, or the same one after a restart.
    let second = ShareHandle::with_base(&base, Some(cache.clone()));
    assert_eq!(second.list(false).await.unwrap().len(), 4);
    assert_eq!(
        stub.lists.load(Ordering::SeqCst),
        1,
        "a day-old cache is good enough; nothing about a rig needs yesterday's list",
    );

    // And asking outright fetches it again.
    second.list(true).await.unwrap();
    assert_eq!(stub.lists.load(Ordering::SeqCst), 2);

    let _ = std::fs::remove_dir_all(&dir);
}
