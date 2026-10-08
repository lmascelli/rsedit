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
//! Three boundaries are checked, and they are the three that carry weight:
//!
//! - `text/` knows nothing of the editor *or* the interpreter. It is the layer
//!   that makes searching and rectangles testable against a bare
//!   [`crate::buffer::BufferTrait`], and reachable from a program that is not
//!   an editor.
//! - `lisp/` knows nothing of the editor. Its context is a type parameter, which
//!   is what lets the interpreter be used and tested on its own.
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

    /// Every file under `lisp/`.
    const LISP: &[(&str, &str)] = &[
        ("lisp/mod.rs", include_str!("../lisp/mod.rs")),
        ("lisp/context.rs", include_str!("../lisp/context.rs")),
        (
            "lisp/environment.rs",
            include_str!("../lisp/environment.rs"),
        ),
        ("lisp/error.rs", include_str!("../lisp/error.rs")),
        ("lisp/eval.rs", include_str!("../lisp/eval.rs")),
        ("lisp/fuel.rs", include_str!("../lisp/fuel.rs")),
        ("lisp/handshake.rs", include_str!("../lisp/handshake.rs")),
        ("lisp/lispexp.rs", include_str!("../lisp/lispexp.rs")),
        ("lisp/parser.rs", include_str!("../lisp/parser.rs")),
        ("lisp/source_map.rs", include_str!("../lisp/source_map.rs")),
        ("lisp/types.rs", include_str!("../lisp/types.rs")),
        ("lisp/utils.rs", include_str!("../lisp/utils.rs")),
    ];

    /// Every file under `lisp/base/`: the primitives this Lisp has of its own.
    ///
    /// Its own list rather than entries in [`LISP`], because it is its own
    /// directory with its own `mod.rs` to count against -- and because the one
    /// boundary that matters most is here: a *primitive* that reached for an
    /// editor would be the easiest of these mistakes to make, and the hardest to
    /// see.
    const LISP_BASE: &[(&str, &str)] = &[
        ("lisp/base/mod.rs", include_str!("../lisp/base/mod.rs")),
        ("lisp/base/atoms.rs", include_str!("../lisp/base/atoms.rs")),
        (
            "lisp/base/comparisons.rs",
            include_str!("../lisp/base/comparisons.rs"),
        ),
        (
            "lisp/base/fibers.rs",
            include_str!("../lisp/base/fibers.rs"),
        ),
        (
            "lisp/base/functions.rs",
            include_str!("../lisp/base/functions.rs"),
        ),
        (
            "lisp/base/inspect.rs",
            include_str!("../lisp/base/inspect.rs"),
        ),
        ("lisp/base/lists.rs", include_str!("../lisp/base/lists.rs")),
        ("lisp/base/math.rs", include_str!("../lisp/base/math.rs")),
        (
            "lisp/base/predicates.rs",
            include_str!("../lisp/base/predicates.rs"),
        ),
        (
            "lisp/base/strings.rs",
            include_str!("../lisp/base/strings.rs"),
        ),
        (
            "lisp/base/symbols.rs",
            include_str!("../lisp/base/symbols.rs"),
        ),
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
    fn the_interpreter_knows_nothing_of_the_editor() {
        for forbidden in ["EditorState", "crate::editor", "crate::buffer", "crate::ui"] {
            assert_never_names(
                LISP_BASE,
                forbidden,
                "A primitive in `lisp/base/` is one that would mean the same thing in a Lisp \
                 with no editor behind it. One that needs a buffer or a window belongs in \
                 `crate::primitives` -- see lisp/base/mod.rs.",
            );
            assert_never_names(
                LISP,
                forbidden,
                "The interpreter's context is a type parameter `T: LispContext`, and that \
                 is what lets it be built and tested without an editor. A primitive that \
                 needs one is an *editor* primitive and belongs in `primitives/`.",
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
    /// Two kinds of declaration are not counted, and both are directories rather
    /// than files: a `mod tests` -- a directory's own tests are allowed to name
    /// whatever they test against, and `lisp/tests/` builds editors -- and a
    /// sub-directory that has a list of its own, which `lisp/base` does.
    #[test]
    fn every_file_in_each_checked_directory_is_listed() {
        for (files, directory, sub_directories) in [
            (TEXT, "text", &[][..]),
            (LISP, "lisp", &["base"][..]),
            (LISP_BASE, "lisp/base", &[][..]),
            (UI, "ui", &[][..]),
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
}
