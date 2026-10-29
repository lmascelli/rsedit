//! `HOME` and `USERPROFILE`, set for the duration of one test and put back,
//! under one lock that every test leaning on the home directory takes: the
//! environment is process-wide and the tests run in threads.
//!
//! Design: `doc/architettura-editor.typ, <directory>`.
#[cfg(test)]
pub(crate) mod guard {
    use std::sync::{Mutex, MutexGuard};

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    /// The home-directory variables, held at chosen values while this lives.
    pub(crate) struct HomeAs {
        home: Option<String>,
        userprofile: Option<String>,
        // Dropped last, after the values above have been put back.
        _guard: MutexGuard<'static, ()>,
    }

    impl HomeAs {
        /// `HOME` pointed at DIR, with `USERPROFILE` cleared so that it cannot
        /// answer instead and make the test pass for the wrong reason.
        pub(crate) fn directory(dir: &std::path::Path) -> Self {
            Self::vars(Some(&dir.to_string_lossy()), None)
        }

        /// Both variables set explicitly -- for the tests that are *about*
        /// which of the two answers.
        pub(crate) fn vars(home: Option<&str>, userprofile: Option<&str>) -> Self {
            let guard = HOME_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let saved = HomeAs {
                home: std::env::var("HOME").ok(),
                userprofile: std::env::var("USERPROFILE").ok(),
                _guard: guard,
            };
            set("HOME", home);
            set("USERPROFILE", userprofile);
            saved
        }
    }

    impl Drop for HomeAs {
        fn drop(&mut self) {
            set("HOME", self.home.as_deref());
            set("USERPROFILE", self.userprofile.as_deref());
        }
    }

    /// SAFETY: every path into this is through `HomeAs`, which holds
    /// `HOME_LOCK` for as long as the value is set -- so no other test is
    /// reading these while one is being written.
    fn set(name: &str, value: Option<&str>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}
