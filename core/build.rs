use std::fs;
use std::path::Path;

fn main() {
    let profile_dir = std::env::var("PROFILE").unwrap();
    let manifest_dir = std::env::var("CARGO_WORKSPACE_DIR").unwrap();
    let data_dir = Path::new(&manifest_dir)
        .join("target")
        .join(profile_dir)
        .join("data");

    // What the editor reads at runtime goes beside the binary, because the
    // source tree it was built from may not be there when it runs. Each
    // directory is copied whole: which modules are loaded is init.lisp's
    // business, and a test checks that every module it loads is here.
    copy_files(Path::new("lisp"), &data_dir.join("lisp"), "lisp");

    // The themes and the manual pages are at the project root, outside this
    // crate, so they are copied when they are there. Of `man/`, only the
    // pages: its README.md is for whoever writes one.
    for (source, kind, extension) in [("../themes", "themes", "lisp"), ("../man", "man", "txt")] {
        let source = Path::new(source);
        if source.is_dir() {
            copy_files(source, &data_dir.join(kind), extension);
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
