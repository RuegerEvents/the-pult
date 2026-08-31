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
    save(&Preferences { history_depth: 2000 }).unwrap();
    assert_eq!(load().history_depth, 2000);
}

/// The directory does not exist until something is saved into it, and a first run
/// should not have to be told twice.
#[test]
fn saving_makes_the_place_to_save_into() {
    let own = own_file();
    assert!(!own.0.exists());
    save(&Preferences { history_depth: 750 }).unwrap();
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
