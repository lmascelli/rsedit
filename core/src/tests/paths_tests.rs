//! The editor's own directories: which place wins, and what asking for one
//! does on the disk. Never the person's own: the precedence is checked through
//! the functions the environment is read into, the disk on directories of each
//! test's own, and `app_dir` itself on the directories that
//! `isolate_config_for_tests` names for this process.
#[cfg(test)]
mod tests {
    use crate::paths::{
        AppDir, DirError, DirOptions, app_dir, choose_dir, ensure_dir, path_from, xdg_parent,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// A directory for one test, gone when the test is.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsedit-app-dir-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("the sandbox");
            Sandbox(dir)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn os(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    fn path(text: &str) -> Option<PathBuf> {
        Some(PathBuf::from(text))
    }

    fn chosen(
        rsedit_dir: Option<PathBuf>,
        xdg: Option<PathBuf>,
        platform: Option<PathBuf>,
    ) -> Option<String> {
        choose_dir(rsedit_dir, xdg, platform).map(|dir| dir.display().to_string())
    }

    // -- which place wins -------------------------------------------------

    #[test]
    fn the_explicit_variable_wins_over_everything() {
        // One answer that means the same on every platform: a portable
        // install, a second configuration, a test suite.
        assert_eq!(
            chosen(
                path("/opt/cfg"),
                path("/home/me/.config"),
                path("/fallback")
            ),
            Some("/opt/cfg".to_string())
        );
    }

    #[test]
    fn the_explicit_variable_is_used_exactly_as_given() {
        // No `rsedit` joined onto it: it names the directory holding
        // `init.lisp`, not a parent of application directories.
        assert_eq!(
            chosen(path("/tmp/scratch"), None, None),
            Some("/tmp/scratch".to_string())
        );
    }

    #[test]
    fn xdg_is_a_parent_and_gets_the_application_name_joined_on() {
        // `XDG_CONFIG_HOME` contains each application's own directory, so
        // `rsedit` goes on the end.
        assert_eq!(
            chosen(None, path("/home/me/.config"), path("/fallback")),
            Some("/home/me/.config/rsedit".to_string())
        );
    }

    #[test]
    fn the_platform_answer_is_used_when_nothing_overrides_it() {
        // Already complete: whoever built it knew whether the platform wanted
        // `.config/rsedit` or `rsedit` under `%APPDATA%`.
        assert_eq!(
            chosen(None, None, path("/home/me/.config/rsedit")),
            Some("/home/me/.config/rsedit".to_string())
        );
    }

    #[test]
    fn nothing_at_all_is_an_answer() {
        // With no home there is nowhere to keep a configuration -- not a
        // relative path made out of nothing, in the working directory.
        assert_eq!(chosen(None, None, None), None);
    }

    #[test]
    fn a_missing_middle_step_falls_past_it_rather_than_stopping() {
        assert_eq!(
            chosen(None, None, path("/fallback")),
            Some("/fallback".to_string())
        );
    }

    #[test]
    fn the_cache_is_chosen_as_the_configuration_is() {
        // The same precedence for every directory: named outright, then the
        // freedesktop parent with the editor's name joined on, then the
        // platform's own place.
        let named = Some(PathBuf::from("/opt/cache"));
        let xdg = Some(PathBuf::from("/home/me/.cache"));
        let platform = Some(PathBuf::from("/home/me/.cache/rsedit"));
        assert_eq!(
            choose_dir(named.clone(), xdg.clone(), platform.clone()),
            named
        );
        assert_eq!(
            choose_dir(None, xdg, platform.clone()),
            Some(PathBuf::from("/home/me/.cache/rsedit"))
        );
        assert_eq!(choose_dir(None, None, platform.clone()), platform);
        assert_eq!(choose_dir(None, None, None), None);
    }

    #[test]
    fn a_variable_set_but_empty_counts_as_not_set() {
        // Otherwise an empty RSEDIT_CONFIG_DIR would name "": the directory
        // the editor was started from.
        assert_eq!(path_from(os("")), None);
        assert_eq!(path_from(None), None);
        assert_eq!(path_from(os("/opt/cfg")), Some(PathBuf::from("/opt/cfg")));
    }

    #[test]
    fn a_relative_freedesktop_variable_is_ignored() {
        // The specification's rule: such a path is invalid, and an
        // implementation ignores it rather than resolving it against
        // wherever it happens to have been started.
        assert_eq!(xdg_parent(os("cache")), None);
        assert_eq!(xdg_parent(os("")), None);
        assert_eq!(
            xdg_parent(os("/home/me/.cache")),
            Some(PathBuf::from("/home/me/.cache"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn the_platforms_own_places_are_under_the_home_directory() {
        use crate::paths::platform_dir;
        use crate::tests::home_guard::guard::HomeAs;
        let _home = HomeAs::directory(std::path::Path::new("/home/someone"));
        assert_eq!(
            platform_dir(AppDir::Config),
            Some(PathBuf::from("/home/someone/.config/rsedit"))
        );
        assert_eq!(
            platform_dir(AppDir::Cache),
            Some(PathBuf::from("/home/someone/.cache/rsedit"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn with_no_home_there_is_no_platform_place() {
        use crate::paths::platform_dir;
        use crate::tests::home_guard::guard::HomeAs;
        let _home = HomeAs::vars(None, None);
        assert_eq!(platform_dir(AppDir::Cache), None);
    }

    // -- what is done on the disk -------------------------------------------

    #[test]
    fn a_directory_that_is_there_is_answered_as_it_is() {
        let sandbox = Sandbox::new("there");
        for create in [false, true] {
            assert_eq!(
                ensure_dir(sandbox.0.clone(), create).ok(),
                Some(sandbox.0.clone())
            );
        }
    }

    #[test]
    fn a_missing_directory_is_made_with_its_parents_when_asked() {
        let sandbox = Sandbox::new("made");
        let deep = sandbox.0.join("one").join("two");
        assert_eq!(ensure_dir(deep.clone(), true).ok(), Some(deep.clone()));
        assert!(deep.is_dir());
    }

    #[test]
    fn a_missing_directory_is_not_found_when_making_it_was_not_asked() {
        let sandbox = Sandbox::new("missing");
        let missing = sandbox.0.join("absent");
        match ensure_dir(missing.clone(), false) {
            Err(DirError::NotFound(path)) => assert_eq!(path, missing),
            other => panic!("expected NotFound, got {other:?}"),
        }
        assert!(!missing.exists(), "and nothing was made");
    }

    #[test]
    fn a_file_where_the_directory_should_be_is_not_a_directory() {
        let sandbox = Sandbox::new("file");
        let file = sandbox.0.join("rsedit");
        std::fs::write(&file, "").unwrap();
        for create in [false, true] {
            match ensure_dir(file.clone(), create) {
                Err(DirError::NotADirectory(path)) => assert_eq!(path, file),
                other => panic!("expected NotADirectory, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_directory_that_cannot_be_made_says_why() {
        // Under a file: no system lets a directory be made there.
        let sandbox = Sandbox::new("unmakeable");
        let file = sandbox.0.join("a-file");
        std::fs::write(&file, "").unwrap();
        let under = file.join("rsedit");
        let Err(why) = ensure_dir(under.clone(), true) else {
            panic!("a directory was made under a file");
        };
        assert!(matches!(&why, DirError::CannotCreate { path, .. } if *path == under));
        assert!(
            std::error::Error::source(&why).is_some(),
            "the system's own reason comes with it"
        );
        let said = why.to_string();
        assert!(
            said.starts_with(&format!("{} could not be created: ", under.display())),
            "{said}"
        );
    }

    #[test]
    fn every_error_says_which_directory() {
        assert_eq!(
            DirError::Undetermined(AppDir::Cache).to_string(),
            "nowhere for the cache directory: nothing in the environment says where it goes"
        );
        assert_eq!(
            DirError::NotFound(PathBuf::from("/x/rsedit")).to_string(),
            "/x/rsedit does not exist"
        );
        assert_eq!(
            DirError::NotADirectory(PathBuf::from("/x/rsedit")).to_string(),
            "/x/rsedit is not a directory"
        );
    }

    // -- app_dir itself, on this process's own directories ------------------

    #[test]
    fn the_directories_named_for_the_suite_are_the_ones_answered() {
        // Through RSEDIT_CONFIG_DIR and RSEDIT_CACHE_DIR, which come first on
        // every system: what a portable install would set too.
        crate::isolate_config_for_tests();
        let pid = std::process::id();
        for (dir, expected) in [
            (AppDir::Config, format!("rsedit-test-config-{pid}")),
            (AppDir::Cache, format!("rsedit-test-cache-{pid}")),
        ] {
            let expected = std::env::temp_dir().join(expected);
            // Whatever another test has already made: asking without making
            // must not make it.
            let there = expected.is_dir();
            let asked = app_dir(DirOptions { dir, create: false });
            assert_eq!(
                expected.is_dir(),
                there,
                "asking made {}",
                expected.display()
            );
            if !there {
                assert!(matches!(asked, Err(DirError::NotFound(_))), "{asked:?}");
            }
            let path = app_dir(DirOptions { dir, create: true }).expect("the suite's own");
            assert_eq!(path, expected);
            assert!(path.is_dir(), "made when it was missing");
            assert_eq!(
                app_dir(DirOptions { dir, create: false }).ok(),
                Some(path),
                "and found once it is there"
            );
        }
    }
}
