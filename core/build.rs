//! Puts the editor's own data -- its Lisp modules, its themes and its manual
//! pages -- into the executable: a table, in `OUT_DIR/shipped_data.rs`, of
//! every file by the path it is written out at, its bytes taken with
//! `include_bytes!`, and the stamp that says which data this is.
//!
//! Nothing is copied beside the executable: the editor writes the table out
//! into its data directory itself, when what is there is not this data. See
//! `src/shipped_data.rs`.
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let crate_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo must set CARGO_MANIFEST_DIR"),
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR"));

    // (the path it is written out at, `/` between its parts; the file it is
    // read from).
    let mut files: Vec<(String, PathBuf)> = Vec::new();

    // The modules, each kind in a directory of its own: `base`, `features`
    // and `languages`, as they are in `core/lisp`.
    let lisp = crate_dir.join("lisp");
    println!("cargo::rerun-if-changed={}", lisp.display());
    let kinds =
        fs::read_dir(&lisp).unwrap_or_else(|why| panic!("cannot read {}: {why}", lisp.display()));
    for kind in kinds {
        let kind = kind
            .unwrap_or_else(|why| panic!("cannot read {}: {why}", lisp.display()))
            .path();
        if kind.is_dir() {
            let name = kind
                .file_name()
                .expect("a directory has a name")
                .to_str()
                .expect("a kind of module is named in UTF-8");
            collect_files(&kind, &format!("lisp/{name}"), "lisp", &mut files);
        }
    }

    for (relative_source, kind, extension) in
        [("../themes", "themes", "lisp"), ("../man", "man", "txt")]
    {
        let source = crate_dir.join(relative_source);
        if source.is_dir() {
            collect_files(&source, kind, extension, &mut files);
        }
    }

    // The index of the interpreter's own functions, which the interpreter
    // writes: a page like the others once it is a file.
    let index = out_dir.join("risp-base-index.txt");
    fs::write(&index, risp::BASE_FUNCTION_INDEX)
        .unwrap_or_else(|why| panic!("cannot write {}: {why}", index.display()));
    files.push(("man/risp-base-index.txt".into(), index));

    // In one order whatever the file system lists them in, so that the stamp
    // is the same for the same data.
    files.sort();

    let mut table = String::from(
        "/// Every file of the editor's own data, by the path it is written out at\n\
         /// under the data directory, `/` between its parts, sorted.\n\
         pub(crate) static FILES: &[(&str, &[u8])] = &[\n",
    );
    let mut hash = Fnv64::default();
    for (name, path) in &files {
        let bytes =
            fs::read(path).unwrap_or_else(|why| panic!("cannot read {}: {why}", path.display()));
        hash.write(name.as_bytes());
        hash.write(&[0]);
        hash.write(&(bytes.len() as u64).to_le_bytes());
        hash.write(&bytes);
        let source = path
            .to_str()
            .unwrap_or_else(|| panic!("{} is not named in UTF-8", path.display()));
        // `{:?}` writes each string as a Rust literal, escapes and all: a
        // backslash in a Windows path stays a backslash.
        writeln!(table, "    ({name:?}, include_bytes!({source:?})),").expect("a String");
    }
    table.push_str("];\n\n");
    let stamp = format!(
        "rsedit_core {} {:016x}\n",
        env::var("CARGO_PKG_VERSION").expect("Cargo must set CARGO_PKG_VERSION"),
        hash.finish()
    );
    writeln!(
        table,
        "/// What the stamp file says when the data directory holds this data:\n\
         /// the version, and a hash of every file's path and bytes.\n\
         pub(crate) const STAMP: &str = {stamp:?};"
    )
    .expect("a String");

    let generated = out_dir.join("shipped_data.rs");
    fs::write(&generated, table)
        .unwrap_or_else(|why| panic!("cannot write {}: {why}", generated.display()));
}

/// Adds to FILES the files of SOURCE named `*.EXTENSION`, each at DESTINATION
/// followed by its name.
fn collect_files(
    source: &Path,
    destination: &str,
    extension: &str,
    files: &mut Vec<(String, PathBuf)>,
) {
    println!("cargo::rerun-if-changed={}", source.display());
    let entries = fs::read_dir(source)
        .unwrap_or_else(|why| panic!("cannot read {}: {why}", source.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|why| panic!("cannot read {}: {why}", source.display()))
            .path();
        if path.is_file() && path.extension().is_some_and(|found| found == extension) {
            let name = path
                .file_name()
                .expect("a file has a name")
                .to_str()
                .unwrap_or_else(|| panic!("{} is not named in UTF-8", path.display()));
            files.push((format!("{destination}/{name}"), path));
        }
    }
}

/// FNV-1a, 64 bits: enough to tell one build's data from another's, and
/// needing no crate.
struct Fnv64(u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Fnv64(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv64 {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
