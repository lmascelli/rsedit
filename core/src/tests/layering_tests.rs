//! That the boundaries `lib.rs` describes are boundaries.
//!
//! # Why a test and not a comment
//!
//! The crate's layering was documented in a dozen module headers before it was
//! a directory structure, and documentation is not a constraint: nothing stopped
//! `search.rs` reaching for an `EditorState`, and the only thing that would have
//! noticed is a reader who happened to look. A directory makes the layer
//! visible; this makes it fail.
//!
//! The editor's internal boundaries are checked here. The interpreter now has
//! its own repository, so its dependency boundary is enforced by Cargo instead.
//!
//! - `text/` knows nothing of the editor *or* the interpreter. It is the layer
//!   that makes searching and rectangles testable against a bare
//!   [`crate::buffer::BufferTrait`], and reachable from a program that is not
//!   an editor.
//! - `ui/` knows nothing of `editor/`. It describes what to draw; a renderer is
//!   written against that description, not against this crate's internals.
//! - `buffer/` uses nothing below it in the map but the types it stores: the
//!   faces of `ui/`, and the colouring and scan states of `modes/`.
//! - The map in `lib.rs` names every module the crate declares, and nothing
//!   else.
//!
//! # Why `include_str!` and not a file walk
//!
//! The paths are resolved by the compiler, relative to this file, so the test
//! does not depend on the working directory and a renamed or deleted file is a
//! *compile* error rather than a test that quietly checks nothing. The cost is
//! that a new file in one of these directories has to be added to the list
//! below -- which is the one piece of upkeep here, and is why each list says
//! what it is for.
#[cfg(test)]
mod tests {
    /// Every file under `text/`.
    const TEXT: &[(&str, &str)] = &[
        ("text/mod.rs", include_str!("../text/mod.rs")),
        ("text/kill_ring.rs", include_str!("../text/kill_ring.rs")),
        ("text/rectangle.rs", include_str!("../text/rectangle.rs")),
        ("text/results.rs", include_str!("../text/results.rs")),
        ("text/search.rs", include_str!("../text/search.rs")),
    ];

    /// The crate's root, whose documentation is the map.
    const LIB: &str = include_str!("../lib.rs");

    /// Every file under `buffer/`.
    const BUFFER: &[(&str, &str)] = &[
        ("buffer/mod.rs", include_str!("../buffer/mod.rs")),
        (
            "buffer/buffer_trait.rs",
            include_str!("../buffer/buffer_trait.rs"),
        ),
        ("buffer/disk.rs", include_str!("../buffer/disk.rs")),
        (
            "buffer/gap_buffer.rs",
            include_str!("../buffer/gap_buffer.rs"),
        ),
        ("buffer/mark.rs", include_str!("../buffer/mark.rs")),
        ("buffer/overlay.rs", include_str!("../buffer/overlay.rs")),
        ("buffer/scan.rs", include_str!("../buffer/scan.rs")),
        ("buffer/syntax.rs", include_str!("../buffer/syntax.rs")),
        ("buffer/undo.rs", include_str!("../buffer/undo.rs")),
        (
            "buffer/virtual_text.rs",
            include_str!("../buffer/virtual_text.rs"),
        ),
    ];

    /// What `buffer/` uses from below it in the map, sorted: the types a
    /// buffer stores. The whole of the exception, so that it cannot grow
    /// unnoticed.
    const BUFFER_STORES: &[&str] = &[
        "crate::modes::SyntaxSpan",
        "crate::modes::SyntaxState",
        "crate::modes::SyntaxTable",
        "crate::modes::sexp::Resume",
        "crate::ui::Face",
    ];

    /// Every file under `ui/`.
    const UI: &[(&str, &str)] = &[
        ("ui/mod.rs", include_str!("../ui/mod.rs")),
        ("ui/faces.rs", include_str!("../ui/faces.rs")),
        ("ui/frame.rs", include_str!("../ui/frame.rs")),
        ("ui/layout.rs", include_str!("../ui/layout.rs")),
        ("ui/windows.rs", include_str!("../ui/windows.rs")),
    ];

    /// The lines of SOURCE that are code, with comments and doc comments
    /// dropped.
    ///
    /// A module doc *describing* the boundary -- "this must not reach
    /// `EditorState`" -- would otherwise read as a violation of it, which is the
    /// one way a test like this reliably goes wrong.
    fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
        source
            .lines()
            .enumerate()
            .map(|(n, line)| (n + 1, line.trim()))
            .filter(|(_, line)| !line.starts_with("//"))
    }

    /// Fail if any file in FILES mentions FORBIDDEN in code.
    fn assert_never_names(files: &[(&str, &str)], forbidden: &str, because: &str) {
        for (path, source) in files {
            for (number, line) in code_lines(source) {
                assert!(
                    !line.contains(forbidden),
                    "{path}:{number} names `{forbidden}`:\n    {line}\n\n{because}"
                );
            }
        }
    }

    #[test]
    fn text_knows_nothing_of_the_editor_or_the_interpreter() {
        for forbidden in [
            "EditorState",
            "crate::editor",
            "crate::lisp",
            "risp::",
            "LispExp",
            "Env<",
        ] {
            assert_never_names(
                TEXT,
                forbidden,
                "`text/` is the layer above `BufferTrait` and below everything that \
                 knows what a buffer is for. Something here needing the editor or the \
                 interpreter belongs in `primitives/` or beside its caller -- see \
                 text/mod.rs.",
            );
        }
    }

    /// Returns the paths under `crate::` that LINE names, with a `use` group
    /// one level deep expanded: `crate::modes::{A, B}` names
    /// `crate::modes::A` and `crate::modes::B`.
    fn crate_paths(line: &str) -> Vec<String> {
        let mut paths = Vec::new();
        let mut rest = line;
        while let Some(at) = rest.find("crate::") {
            rest = &rest[at + "crate::".len()..];
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
                .unwrap_or(rest.len());
            let base = match rest[..end].trim_end_matches(':') {
                "" => "crate".to_string(),
                prefix => format!("crate::{prefix}"),
            };
            if rest[..end].ends_with("::") && rest[end..].starts_with('{') {
                let close = rest[end..].find('}').map_or(rest.len(), |i| end + i);
                for item in rest[end + 1..close].split(',') {
                    let item = item.trim();
                    if !item.is_empty() {
                        paths.push(format!("{base}::{item}"));
                    }
                }
                rest = &rest[close..];
            } else {
                paths.push(base);
                rest = &rest[end..];
            }
        }
        paths
    }

    #[test]
    fn the_buffer_uses_only_the_types_it_stores_from_below() {
        let mut named: Vec<String> = BUFFER
            .iter()
            .flat_map(|(_, source)| code_lines(source))
            .flat_map(|(_, line)| crate_paths(line))
            .filter(|path| {
                !["crate::buffer", "crate::work", "crate::paths"]
                    .iter()
                    .any(|above| path == above || path.starts_with(&format!("{above}::")))
            })
            .collect();
        named.sort_unstable();
        named.dedup();
        assert_eq!(
            named, BUFFER_STORES,
            "`buffer/` may use, of what is below it in the map, only the types a \
             buffer stores. A new one is a decision: add it here and to the map in \
             lib.rs, or move the code that needs it below `buffer/`."
        );
    }

    #[test]
    fn the_view_knows_nothing_of_the_editors_state() {
        for forbidden in ["EditorState", "crate::editor", "crate::primitives"] {
            assert_never_names(
                UI,
                forbidden,
                "`ui/` describes what to draw and is handed everything it needs. Reaching \
                 into `editor/` would make a renderer depend on this crate's internals \
                 rather than on a snapshot.",
            );
        }
    }

    /// The lists above are the whole of each directory, and a file left out of
    /// one is a file the boundary is not checked for.
    ///
    /// Counted against the directory as the compiler sees it: `mod.rs` declares
    /// every file in its directory, so the number of `mod`/`pub mod` lines in it
    /// plus the `mod.rs` itself is how many files there are. A new file shows up
    /// here as a count that no longer matches, which is the reminder to add it.
    ///
    /// A sub-directory with a list of its own is counted separately.
    #[test]
    fn every_file_in_each_checked_directory_is_listed() {
        for (files, directory, sub_directories) in [
            (BUFFER, "buffer", &[] as &[&str]),
            (TEXT, "text", &[] as &[&str]),
            (UI, "ui", &[] as &[&str]),
        ] {
            let (_, root) = files
                .iter()
                .find(|(path, _)| path.ends_with("mod.rs"))
                .expect("each list must include its mod.rs");
            let declared = root
                .lines()
                .filter(|line| {
                    let line = line.trim();
                    (line.starts_with("mod ") || line.starts_with("pub mod "))
                        && line.ends_with(';')
                        && line != "mod tests;"
                        && !sub_directories
                            .iter()
                            .any(|sub| line == format!("mod {sub};"))
                })
                .count();
            assert_eq!(
                files.len(),
                declared + 1,
                "{directory}/mod.rs declares {declared} modules, so {} files should be \
                 listed in this test, not {}",
                declared + 1,
                files.len()
            );
        }
    }

    /// Returns the entries of the map in LIB: the first word of each line of
    /// its `text` block that starts one, `work/` or `paths.rs`.
    fn map_entries(lib: &str) -> Vec<&str> {
        lib.lines()
            .filter_map(|line| line.strip_prefix("//!"))
            .skip_while(|line| line.trim() != "```text")
            .skip(1)
            .take_while(|line| line.trim() != "```")
            .filter(|line| line.starts_with("   ") && !line.starts_with("    "))
            .filter_map(|line| line.split_whitespace().next())
            .collect()
    }

    /// Returns the modules LIB declares, but the tests.
    fn declared_modules(lib: &str) -> Vec<&str> {
        lib.lines()
            .filter_map(|line| {
                let line = line.trim();
                let declared = ["pub(crate) mod ", "pub mod ", "mod "]
                    .iter()
                    .find_map(|prefix| line.strip_prefix(prefix))?;
                declared.strip_suffix(';')
            })
            .filter(|name| *name != "tests")
            .collect()
    }

    #[test]
    fn the_map_in_lib_rs_names_every_module_and_nothing_else() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut mapped = Vec::new();
        for entry in map_entries(LIB) {
            let (name, is_there) = match (entry.strip_suffix('/'), entry.strip_suffix(".rs")) {
                (Some(dir), _) => (dir, src.join(dir).join("mod.rs").is_file()),
                (_, Some(file)) => (file, src.join(entry).is_file()),
                _ => panic!("a map entry is a directory/ or a file.rs, not {entry}"),
            };
            assert!(is_there, "the map names {entry}, which is not in core/src");
            mapped.push(name);
        }
        let mut declared = declared_modules(LIB);
        mapped.sort_unstable();
        declared.sort_unstable();
        assert_eq!(
            mapped, declared,
            "the map in lib.rs and its `mod` lines name different modules"
        );
    }
}
