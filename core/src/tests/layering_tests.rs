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
        for (files, directory, sub_directories) in
            [(TEXT, "text", &[] as &[&str]), (UI, "ui", &[] as &[&str])]
        {
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
}
