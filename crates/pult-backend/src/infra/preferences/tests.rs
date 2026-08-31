use super::testing::own_file;
use super::*;

#[test]
fn a_console_that_has_never_been_told_uses_the_defaults() {
    let _own = own_file();
    assert_eq!(load(), Preferences::default());
}

#[test]
fn what_is_written_is_what_comes_back() {
    let _own = own_file();
    save(&Preferences { history_depth: 2000, ..Default::default() }).unwrap();
    assert_eq!(load().history_depth, 2000);
}

/// The directory does not exist until something is saved into it, and a first run
/// should not have to be told twice.
#[test]
fn saving_makes_the_place_to_save_into() {
    let own = own_file();
    assert!(!own.0.exists());
    save(&Preferences { history_depth: 750, ..Default::default() }).unwrap();
    assert!(own.0.exists());
}

/// A file somebody edited by hand, badly. The console starts.
#[test]
fn a_file_that_will_not_parse_is_replaced_by_the_defaults() {
    let own = own_file();
    std::fs::create_dir_all(own.0.parent().unwrap()).unwrap();
    std::fs::write(&own.0, "history_depth = \"as much as possible\"").unwrap();

    assert_eq!(load(), Preferences::default());
}

/// A number out of range means the nearest sensible one, not no undo at all.
#[test]
fn a_depth_out_of_range_is_brought_back_inside_it() {
    let own = own_file();
    std::fs::create_dir_all(own.0.parent().unwrap()).unwrap();
    std::fs::write(&own.0, "history_depth = 0").unwrap();
    assert_eq!(load().history_depth, pult_schema::types::show::HISTORY_DEPTH_MIN);

    std::fs::write(&own.0, "history_depth = 4000000").unwrap();
    assert_eq!(load().history_depth, pult_schema::types::show::HISTORY_DEPTH_MAX);
}

/// A file written by an older build, which knew fewer settings than this one.
#[test]
fn a_setting_this_build_does_not_recognise_is_ignored() {
    let own = own_file();
    std::fs::create_dir_all(own.0.parent().unwrap()).unwrap();
    std::fs::write(&own.0, "history_depth = 300\nfavourite_colour = \"amber\"").unwrap();

    assert_eq!(load().history_depth, 300);
}

#[test]
fn a_preferences_file_written_before_plugins_existed_still_loads() {
    let own = testing::own_file();
    std::fs::create_dir_all(own.0.parent().unwrap()).unwrap();
    // What an older console wrote. A missing key must not be a parse failure,
    // or upgrading would lose every setting in the file rather than one.
    std::fs::write(&own.0, "history_depth = 300\n").unwrap();

    let prefs = load();
    assert_eq!(prefs.history_depth, 300);
    assert!(prefs.plugins.is_empty());
    assert!(prefs.plugin_config("anything").is_null(), "and nothing is configured");
}

#[test]
fn per_plugin_settings_survive_a_round_trip() {
    let _own = testing::own_file();
    let mut prefs = Preferences::default();
    prefs.plugins.insert(
        "natural-language-control".into(),
        toml::from_str(r#"model = "llama3""#).unwrap(),
    );
    save(&prefs).unwrap();

    let back = load();
    assert_eq!(
        back.plugin_config("natural-language-control")["model"],
        "llama3",
        "this machine's answer, which never travels with the show",
    );
    assert!(back.plugin_config("command-line").is_null());
}

// ── How long this station keeps what nobody did ───────────────────────────────

#[test]
fn a_console_that_has_never_been_told_keeps_an_hour_of_telemetry() {
    let _own = own_file();
    assert_eq!(load().oplog_retention_minutes, OPLOG_RETENTION_MINUTES_DEFAULT);
}

#[test]
fn a_retention_that_was_written_comes_back() {
    let _own = own_file();
    save(&Preferences { oplog_retention_minutes: 240, ..Default::default() }).unwrap();
    assert_eq!(load().oplog_retention_minutes, 240);
}

/// Getting this wrong costs snapshots rather than correctness, so a nonsense value
/// means the nearest sensible one rather than a console that will not start.
#[test]
fn a_retention_out_of_range_is_brought_back_inside_it() {
    let own = own_file();
    std::fs::create_dir_all(own.0.parent().unwrap()).unwrap();

    std::fs::write(&own.0, "oplog_retention_minutes = 0").unwrap();
    assert_eq!(load().oplog_retention_minutes, OPLOG_RETENTION_MINUTES_MIN);

    std::fs::write(&own.0, "oplog_retention_minutes = 99999999").unwrap();
    assert_eq!(load().oplog_retention_minutes, OPLOG_RETENTION_MINUTES_MAX);
}

#[test]
fn a_retention_that_will_not_parse_leaves_the_console_with_the_default() {
    let own = own_file();
    std::fs::create_dir_all(own.0.parent().unwrap()).unwrap();
    std::fs::write(&own.0, "oplog_retention_minutes = \"forever\"").unwrap();

    assert_eq!(load().oplog_retention_minutes, OPLOG_RETENTION_MINUTES_DEFAULT);
}

/// A file from a build that had never heard of this setting.
#[test]
fn a_file_written_before_the_retention_existed_still_opens() {
    let own = own_file();
    std::fs::create_dir_all(own.0.parent().unwrap()).unwrap();
    std::fs::write(&own.0, "history_depth = 750").unwrap();

    let prefs = load();
    assert_eq!(prefs.history_depth, 750, "what it did say is kept");
    assert_eq!(
        prefs.oplog_retention_minutes, OPLOG_RETENTION_MINUTES_DEFAULT,
        "and what it did not say is the default"
    );
}
