//! The command line: which files to open, and the options before them.
//!
//! Design: `doc/architettura-editor.typ, <ciclo-vita>`.
use crate::Options;
use std::ffi::OsString;
use std::path::PathBuf;

#[test]
fn cli_keeps_paths_and_handles_option_terminator() {
    let args = ["--gui", "--font", "mono.ttf", "one.rs", "--", "--two.rs"].map(OsString::from);
    let options = Options::parse(args).unwrap();
    assert!(options.gui);
    assert_eq!(options.font, Some(PathBuf::from("mono.ttf")));
    assert_eq!(
        options.files,
        vec![PathBuf::from("one.rs"), PathBuf::from("--two.rs")]
    );
    assert!(Options::parse([OsString::from("--font")]).is_err());
}

#[test]
fn restore_data_is_an_option_and_after_the_terminator_a_file() {
    let options = Options::parse([OsString::from("--restore-data")]).unwrap();
    assert!(options.restore_data);
    assert!(options.files.is_empty());
    let options = Options::parse(["--", "--restore-data"].map(OsString::from)).unwrap();
    assert!(!options.restore_data);
    assert_eq!(options.files, vec![PathBuf::from("--restore-data")]);
}
