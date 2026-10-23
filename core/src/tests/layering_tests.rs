//! That the boundaries `lib.rs` describes are boundaries:
//!
//! - `text/` knows nothing of the editor *or* the interpreter, so that it can
//!   be used by a program that is not an editor.
//! - `ui/` knows nothing of `editor/`: a renderer is written against what it
//!   describes, not against this crate's internals.
//! - Each module checked here uses, of what is below it in the map, only what
//!   the map declares: nothing, but for `buffer/` and `managers/` (the types
//!   they store) and `input.rs` (`ELispExp`).
//! - The map in `lib.rs` names every module the crate declares, and nothing
//!   else.
//!
//! The files are read with `include_str!`, so that one renamed or deleted is a
//! compile error rather than a check that quietly checks nothing; a new file
//! has to be added to its list, and a test counts that it was.
#[cfg(test)]
mod tests {
    /// Every file under `text/`.
    const TEXT: &[(&str, &str)] = &[
        ("text/mod.rs", include_str!("../text/mod.rs")),
        ("text/isearch.rs", include_str!("../text/isearch.rs")),
        ("text/kill_ring.rs", include_str!("../text/kill_ring.rs")),
        ("text/rectangle.rs", include_str!("../text/rectangle.rs")),
        ("text/replace.rs", include_str!("../text/replace.rs")),
        ("text/results.rs", include_str!("../text/results.rs")),
        ("text/search.rs", include_str!("../text/search.rs")),
    ];

    /// The crate's root, whose documentation is the map.
    const LIB: &str = include_str!("../lib.rs");

    /// Every file under `work/`.
    const WORK: &[(&str, &str)] = &[
        ("work/mod.rs", include_str!("../work/mod.rs")),
        ("work/clock.rs", include_str!("../work/clock.rs")),
        ("work/core.rs", include_str!("../work/core.rs")),
        ("work/executor.rs", include_str!("../work/executor.rs")),
        ("work/job.rs", include_str!("../work/job.rs")),
        ("work/manager.rs", include_str!("../work/manager.rs")),
        ("work/options.rs", include_str!("../work/options.rs")),
        ("work/registry.rs", include_str!("../work/registry.rs")),
        ("work/report.rs", include_str!("../work/report.rs")),
        ("work/threads.rs", include_str!("../work/threads.rs")),
        ("work/topic.rs", include_str!("../work/topic.rs")),
        ("work/watchdog.rs", include_str!("../work/watchdog.rs")),
    ];

    /// The modules of one file the map orders.
    const PATHS: &[(&str, &str)] = &[("paths.rs", include_str!("../paths.rs"))];
    const INPUT: &[(&str, &str)] = &[("input.rs", include_str!("../input.rs"))];
    const COMMANDS: &[(&str, &str)] = &[("commands.rs", include_str!("../commands.rs"))];

    /// Every file under `managers/`.
    const MANAGERS: &[(&str, &str)] = &[
        ("managers/mod.rs", include_str!("../managers/mod.rs")),
        (
            "managers/buffers.rs",
            include_str!("../managers/buffers.rs"),
        ),
        (
            "managers/commands.rs",
            include_str!("../managers/commands.rs"),
        ),
        (
            "managers/history.rs",
            include_str!("../managers/history.rs"),
        ),
        (
            "managers/kill_yank.rs",
            include_str!("../managers/kill_yank.rs"),
        ),
        ("managers/log.rs", include_str!("../managers/log.rs")),
        ("managers/macros.rs", include_str!("../managers/macros.rs")),
        ("managers/modes.rs", include_str!("../managers/modes.rs")),
        (
            "managers/runtime.rs",
            include_str!("../managers/runtime.rs"),
        ),
        (
            "managers/windows.rs",
            include_str!("../managers/windows.rs"),
        ),
    ];

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

    /// The files of a module, each by its path and with its text.
    type Files = &'static [(&'static str, &'static str)];

    /// Each module checked against the map, its files, and what it uses from
    /// below it in the map, sorted: the exceptions the map declares, and
    /// nothing else, so that they cannot grow unnoticed.
    const FROM_BELOW: &[(&str, Files, &[&str])] = &[
        ("work/", WORK, &[]),
        ("paths.rs", PATHS, &[]),
        (
            "buffer/",
            BUFFER,
            &[
                "crate::modes::SyntaxSpan",
                "crate::modes::SyntaxState",
                "crate::modes::SyntaxTable",
                "crate::modes::sexp::Resume",
                "crate::ui::Face",
            ],
        ),
        ("text/", TEXT, &[]),
        ("input.rs", INPUT, &["crate::ELispExp"]),
        ("commands.rs", COMMANDS, &[]),
        (
            "managers/",
            MANAGERS,
            &[
                "crate::ELispExp",
                "crate::modes::MajorMode",
                "crate::modes::SyntaxTable",
                "crate::ui::Division",
                "crate::ui::Face",
                "crate::ui::FloatingWindow",
                "crate::ui::LayoutNode",
                "crate::ui::Orientation",
                "crate::ui::Rect",
                "crate::ui::Side",
                "crate::ui::SplitPath",
                "crate::ui::Style",
                "crate::ui::Theme",
                "crate::ui::Window",
                "crate::ui::WindowId",
            ],
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

    /// Returns the lines of SOURCE that are code, with comments and doc
    /// comments dropped.
    ///
    /// A module doc *describing* the boundary -- "this must not reach
    /// `EditorState`" -- would otherwise read as a violation of it, which is
    /// the one way a test like this reliably goes wrong.
    fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
        source
            .lines()
            .enumerate()
            .map(|(n, line)| (n + 1, line.trim()))
            .filter(|(_, line)| !line.starts_with("//"))
    }

    /// Fails if any file in FILES mentions FORBIDDEN in code.
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

    /// Returns the paths under `crate::` that the code of SOURCE names, with
    /// `use` groups expanded: `crate::{ELispExp, modes::{A, B}}` names
    /// `crate::ELispExp`, `crate::modes::A` and `crate::modes::B`.
    fn crate_paths(source: &str) -> Vec<String> {
        // One line, so that a group rustfmt spreads over several is read whole.
        let code: Vec<&str> = code_lines(source).map(|(_, line)| line).collect();
        let code = code.join(" ");
        let mut paths = Vec::new();
        for (at, _) in code.match_indices("crate::") {
            expand(&mut paths, "crate", &code[at + "crate::".len()..]);
        }
        paths
    }

    /// Pushes onto PATHS what the path or `use` tree at the start of TEXT
    /// names under BASE.
    fn expand(paths: &mut Vec<String>, base: &str, text: &str) {
        let end = text
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .unwrap_or(text.len());
        let path = match text[..end].trim_end_matches(':') {
            "" => base.to_string(),
            name => format!("{base}::{name}"),
        };
        let Some(group) = text[end..].strip_prefix('{') else {
            return paths.push(path);
        };
        // Each item is a tree under PATH, up to a comma at its own depth or
        // the brace that closes the group.
        let (mut depth, mut start) = (0, 0);
        for (at, c) in group.char_indices() {
            match c {
                '{' => depth += 1,
                '}' if depth > 0 => depth -= 1,
                ',' | '}' if depth == 0 => {
                    let item = group[start..at].trim();
                    if !item.is_empty() {
                        expand(paths, &path, item);
                    }
                    if c == '}' {
                        return;
                    }
                    start = at + 1;
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_use_group_is_read_however_it_is_written() {
        let source = "use crate::{ELispExp, buffer::{A, B as C}};
            use crate::modes::{
                SyntaxSpan,
                // A comment.
                SyntaxState,
            };
            let found: crate::text::search::Match;";
        assert_eq!(
            crate_paths(source),
            [
                "crate::ELispExp",
                "crate::buffer::A",
                "crate::buffer::B",
                "crate::modes::SyntaxSpan",
                "crate::modes::SyntaxState",
                "crate::text::search::Match",
            ]
        );
    }

    #[test]
    fn each_module_uses_from_below_only_what_the_map_declares() {
        let map = map_entries(LIB);
        for (entry, files, declared) in FROM_BELOW {
            let at = map
                .iter()
                .position(|mapped| mapped == entry)
                .unwrap_or_else(|| panic!("{entry} is not in the map"));
            // The module itself and everything above it, as paths.
            let above: Vec<String> = map[..=at]
                .iter()
                .map(|mapped| {
                    let name = mapped.trim_end_matches('/').trim_end_matches(".rs");
                    format!("crate::{name}")
                })
                .collect();
            let mut named: Vec<String> = files
                .iter()
                .flat_map(|(_, source)| crate_paths(source))
                .filter(|path| {
                    !above
                        .iter()
                        .any(|module| path == module || path.starts_with(&format!("{module}::")))
                })
                .collect();
            named.sort_unstable();
            named.dedup();
            assert_eq!(
                named, *declared,
                "{entry} uses, of what is below it in the map, only what the map in \
                 lib.rs declares. A new use is a decision: declare it there and here, \
                 or move the code that needs it below {entry}."
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
    /// Counted against the directory as the compiler sees it: `mod.rs`
    /// declares every file in its directory, so the number of `mod`/`pub mod`
    /// lines in it plus the `mod.rs` itself is how many files there are. A new
    /// file shows up here as a count that no longer matches, which is the
    /// reminder to add it.
    ///
    /// A sub-directory with a list of its own is counted separately.
    #[test]
    fn every_file_in_each_checked_directory_is_listed() {
        for (files, directory, sub_directories) in [
            (WORK, "work", &[] as &[&str]),
            (BUFFER, "buffer", &[] as &[&str]),
            (TEXT, "text", &[] as &[&str]),
            (MANAGERS, "managers", &[] as &[&str]),
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
