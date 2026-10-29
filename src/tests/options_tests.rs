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
