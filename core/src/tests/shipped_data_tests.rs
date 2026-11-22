//! The data the executable carries: every module, theme and page of the
//! source in it byte for byte, written out into a data directory when the
//! stamp there says something else, the editor's folders replaced whole and
//! the others left alone, and written again on request.
//!
//! Design: `doc/architettura-editor.typ, <dati-distribuiti>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, MODULE_KINDS, create_global_env};
    use crate::shipped_data::{FILES, PARTS, STAMP, STAMP_FILE, ensure_in, is_current, restore_in};
    use risp::{Env, LispExp, Parser, eval};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// A directory of the test's own, gone when it is.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("rsedit-shipped-data-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            let mut path = self.0.clone();
            path.extend(name.split('/'));
            path
        }

        /// What the sandbox holds directly, by name, sorted.
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .expect("listing the sandbox")
                .map(|entry| {
                    entry
                        .expect("an entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The bytes the executable carries for NAME.
    fn carried(name: &str) -> &'static [u8] {
        FILES
            .iter()
            .find(|(path, _)| *path == name)
            .unwrap_or_else(|| panic!("{name} is not carried"))
            .1
    }

    /// Asserts that DIR holds every file the executable carries, as it is.
    fn assert_written_out(dir: &Sandbox) {
        for (name, bytes) in FILES {
            let found = std::fs::read(dir.path(name))
                .unwrap_or_else(|why| panic!("{name} is not written out: {why}"));
            assert!(found == *bytes, "{name} is not what is carried");
        }
        assert!(is_current(&dir.0));
    }

    /// The files of DIR named `*.EXTENSION`, by the name they are carried at
    /// under DESTINATION.
    fn sources(dir: &Path, destination: &str, extension: &str) -> BTreeMap<String, Vec<u8>> {
        let mut found = BTreeMap::new();
        for entry in std::fs::read_dir(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()))
        {
            let path = entry.expect("an entry").path();
            if path.is_file() && path.extension().is_some_and(|found| found == extension) {
                let name = path
                    .file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned();
                let bytes = std::fs::read(&path).expect("reading a source");
                found.insert(format!("{destination}/{name}"), bytes);
            }
        }
        found
    }

    // -- what is carried -----------------------------------------------------

    #[test]
    fn every_module_theme_and_page_is_carried_as_it_is_in_the_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut expected = BTreeMap::new();
        for kind in MODULE_KINDS {
            expected.extend(sources(
                &root.join("lisp").join(kind),
                &format!("lisp/{kind}"),
                "lisp",
            ));
        }
        expected.extend(sources(&root.join("../themes"), "themes", "lisp"));
        expected.extend(sources(&root.join("../man"), "man", "txt"));
        expected.insert(
            "man/risp-base-index.txt".into(),
            risp::BASE_FUNCTION_INDEX.as_bytes().to_vec(),
        );
        assert!(!risp::BASE_FUNCTION_INDEX.is_empty());
        let carried: BTreeMap<String, Vec<u8>> = FILES
            .iter()
            .map(|(name, bytes)| (name.to_string(), bytes.to_vec()))
            .collect();
        let missing: Vec<&String> = expected
            .keys()
            .filter(|name| !carried.contains_key(*name))
            .collect();
        let extra: Vec<&String> = carried
            .keys()
            .filter(|name| !expected.contains_key(*name))
            .collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "missing {missing:?}, extra {extra:?}"
        );
        for (name, bytes) in &expected {
            assert!(carried[name] == *bytes, "{name} is not carried as it is");
        }
    }

    #[test]
    fn every_carried_file_is_in_one_of_the_editors_folders() {
        let stray: Vec<&str> = FILES
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| {
                !PARTS
                    .iter()
                    .any(|part| name.starts_with(&format!("{part}/")))
            })
            .collect();
        assert!(stray.is_empty(), "{stray:?}");
        let mut names: Vec<&str> = FILES.iter().map(|(name, _)| *name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), FILES.len(), "a file carried twice");
    }

    // -- writing it out --------------------------------------------------------

    #[test]
    fn an_empty_directory_is_given_every_file_and_the_stamp_once() {
        let dir = Sandbox::new("empty");
        assert!(!is_current(&dir.0));
        assert!(ensure_in(&dir.0).expect("written out"));
        assert_written_out(&dir);
        assert_eq!(
            std::fs::read_to_string(dir.0.join(STAMP_FILE)).expect("the stamp"),
            STAMP
        );
        assert!(
            !ensure_in(&dir.0).expect("nothing to write"),
            "written twice"
        );
        // Nothing left of the writing itself.
        let mut expected: Vec<String> = PARTS.iter().map(|part| part.to_string()).collect();
        expected.push(STAMP_FILE.into());
        expected.sort();
        assert_eq!(dir.names(), expected);
    }

    #[test]
    fn a_missing_directory_is_made() {
        let dir = Sandbox::new("missing");
        let inner = dir.0.join("rsedit");
        assert!(ensure_in(&inner).expect("written out"));
        assert!(is_current(&inner));
    }

    #[test]
    fn another_stamp_replaces_the_editors_folders_whole_and_leaves_the_rest() {
        let dir = Sandbox::new("stale");
        ensure_in(&dir.0).expect("written out");
        // What an older version left: a module it shipped and this one does
        // not, and a module of its own.
        std::fs::write(dir.path("lisp/base/gone.lisp"), "(provide 'gone)").expect("a stray");
        std::fs::write(dir.path("lisp/base/commands.lisp"), ";; older").expect("an old one");
        std::fs::create_dir_all(dir.path("plugins/mine")).expect("a plugin");
        std::fs::write(dir.path("plugins/mine/plugin.lisp"), "(mine)").expect("a plugin");
        std::fs::write(dir.0.join(STAMP_FILE), "rsedit_core 0.0.0 0\n").expect("an old stamp");

        assert!(ensure_in(&dir.0).expect("written again"));
        assert_written_out(&dir);
        assert!(
            !dir.path("lisp/base/gone.lisp").exists(),
            "what it no longer ships stays"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path("plugins/mine/plugin.lisp")).expect("the plugin"),
            "(mine)"
        );
        let mut expected: Vec<String> = PARTS.iter().map(|part| part.to_string()).collect();
        expected.extend([STAMP_FILE.to_string(), "plugins".into()]);
        expected.sort();
        assert_eq!(dir.names(), expected);
    }

    #[test]
    fn no_stamp_is_a_start_stopped_half_way_and_the_data_is_written_again() {
        let dir = Sandbox::new("no-stamp");
        ensure_in(&dir.0).expect("written out");
        std::fs::remove_dir_all(dir.path("themes")).expect("half of it gone");
        std::fs::remove_file(dir.0.join(STAMP_FILE)).expect("no stamp");
        assert!(ensure_in(&dir.0).expect("written again"));
        assert_written_out(&dir);
    }

    #[test]
    fn with_its_stamp_a_damaged_file_stays_until_the_data_is_restored() {
        // Only the stamp is read at a start: a file damaged under a stamp
        // that is right is mended by `--restore-data`, which writes all of it
        // whatever the stamp says.
        let dir = Sandbox::new("restore");
        ensure_in(&dir.0).expect("written out");
        let name = "lisp/base/commands.lisp";
        std::fs::write(dir.path(name), "(damaged").expect("damaging it");
        assert!(!ensure_in(&dir.0).expect("nothing to write"));
        assert_eq!(
            std::fs::read(dir.path(name)).expect("the file"),
            b"(damaged"
        );
        restore_in(&dir.0).expect("restored");
        assert_eq!(
            std::fs::read(dir.path(name)).expect("the file"),
            carried(name)
        );
        assert_written_out(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn what_a_stopped_start_left_behind_is_removed_once_it_is_old() {
        let dir = Sandbox::new("leftovers");
        let old = dir.path(".staging-1-0");
        let young = dir.path(".old-2-0");
        for leftover in [&old, &young] {
            std::fs::create_dir_all(leftover.join("lisp")).expect("a leftover");
        }
        let hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        std::fs::File::open(&old)
            .and_then(|file| file.set_modified(hours_ago))
            .expect("ageing the leftover");
        ensure_in(&dir.0).expect("written out");
        assert!(!old.exists(), "an old leftover stays");
        assert!(young.exists(), "another editor's folder was deleted");
    }

    // -- at the editor's start ---------------------------------------------------

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(src).next().expect("the test must parse");
        eval(&ast, env.clone(), ctx).expect("the test must evaluate")
    }

    fn strings(list: &LispExp<Ctx>) -> Vec<String> {
        list.iter()
            .map(|item| match item {
                LispExp::String(text) => text.to_string(),
                other => panic!("not a string: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn the_editor_writes_its_data_out_and_finds_its_themes_there() {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        let data = crate::paths::shipped_data_dir().expect("the suite's data directory");
        assert!(is_current(&data));
        let themes = strings(&run("theme-path", &env, &ctx));
        assert_eq!(
            themes.last().map(PathBuf::from),
            Some(data.join("themes")),
            "{themes:?}"
        );
    }

    #[test]
    fn a_tests_editor_has_none_of_the_editors_modules_in_lisp_path() {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        let data = crate::paths::shipped_data_dir().expect("the suite's data directory");
        let modules = strings(&run("lisp-path", &env, &ctx));
        assert!(
            modules.iter().all(|dir| !Path::new(dir).starts_with(&data)),
            "{modules:?}"
        );
    }
}
