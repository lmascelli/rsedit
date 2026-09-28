//! Which directory an installation's configuration comes from.
//!
//! # Why these test a function that takes its inputs
//!
//! The thing worth pinning down is the *precedence*: which of three possible
//! answers wins, and what is joined onto each. Checking that by setting
//! environment variables would mean writing to a process-wide table while a
//! threaded test runner has other tests reading it -- which is how the
//! `.config/` in a sandbox listing got there in the first place. So the
//! reading of the environment is one thin function that is not tested, and the
//! deciding is a pure one that is.
#[cfg(test)]
mod tests {
    use crate::editor::boot::choose_config_dir;
    use std::path::PathBuf;

    fn path(text: &str) -> Option<PathBuf> {
        Some(PathBuf::from(text))
    }

    fn chosen(
        rsedit_dir: Option<PathBuf>,
        xdg: Option<PathBuf>,
        platform: Option<PathBuf>,
    ) -> Option<String> {
        choose_config_dir(rsedit_dir, xdg, platform).map(|dir| dir.display().to_string())
    }

    #[test]
    fn the_explicit_variable_wins_over_everything() {
        // The point of having it: one answer that means the same thing on
        // every platform, for a portable install, a second configuration kept
        // beside the first, or a test suite that must not touch the person's
        // own files.
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
        // No `rsedit` joined onto it. It names the directory holding
        // `init.lisp`, not a parent of application directories -- which is
        // what makes it usable for a directory that is not laid out like a
        // configuration home at all.
        assert_eq!(
            chosen(path("/tmp/scratch"), None, None),
            Some("/tmp/scratch".to_string())
        );
    }

    #[test]
    fn xdg_is_a_parent_and_gets_the_application_name_joined_on() {
        // `XDG_CONFIG_HOME` is defined as the directory that *contains* each
        // application's own, so `rsedit` belongs on the end. Treating it as
        // the application directory would put `init.lisp` beside every other
        // program's folder.
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
        // A machine with no home directory variable set has nowhere to keep a
        // configuration, and the caller reads that as "there is none" rather
        // than as somewhere to create one. Without this the editor would build
        // a relative path out of nothing and write into the working directory.
        assert_eq!(chosen(None, None, None), None);
    }

    #[test]
    fn a_missing_middle_step_falls_past_it_rather_than_stopping() {
        assert_eq!(
            chosen(None, None, path("/fallback")),
            Some("/fallback".to_string())
        );
    }
}
