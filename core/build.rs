use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let crate_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .expect("Cargo must set CARGO_MANIFEST_DIR"),
    );

    let out_dir = PathBuf::from(
        env::var_os("OUT_DIR")
            .expect("Cargo must set OUT_DIR"),
    );

    // <target-dir>[/<target-triple>]/<profile>/build/<package-hash>/out
    //                                      ^ three ancestors up
    let binary_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("Unexpected Cargo OUT_DIR layout");

    let data_dir = binary_dir.join("data");

    copy_files(
        &crate_dir.join("lisp"),
        &data_dir.join("lisp"),
        "lisp",
    );

    for (relative_source, kind, extension) in [
        ("../themes", "themes", "lisp"),
        ("../man", "man", "txt"),
    ] {
        let source = crate_dir.join(relative_source);

        if source.is_dir() {
            copy_files(&source, &data_dir.join(kind), extension);
        }
    }
}

/// Copies the files of SOURCE named `*.EXTENSION` into DESTINATION, creating
/// it and its parents.
fn copy_files(source: &Path, destination: &Path, extension: &str) {
    println!("cargo::rerun-if-changed={}", source.display());
    fs::create_dir_all(destination)
        .unwrap_or_else(|why| panic!("cannot create {}: {why}", destination.display()));
    let entries = fs::read_dir(source)
        .unwrap_or_else(|why| panic!("cannot read {}: {why}", source.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|why| panic!("cannot read {}: {why}", source.display()))
            .path();
        if path.is_file() && path.extension().is_some_and(|found| found == extension) {
            let name = path.file_name().expect("a file has a name");
            fs::copy(&path, destination.join(name))
                .unwrap_or_else(|why| panic!("cannot copy {}: {why}", path.display()));
        }
    }
}
