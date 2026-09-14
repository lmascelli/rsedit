//! `HOME` and `USERPROFILE`, set for the duration of one test and put back.
//!
//! # Why this is not two guards in two files
//!
//! The environment is process-wide and the test runner is threaded, so a test
//! that sets `HOME` is writing to something every other test can see. A lock
//! makes that safe -- but only if it is *the same* lock. Two files each with a
//! mutex of their own is the shape of the bug it looks like the fix for: each
//! is certain no other test is inside it, and both are wrong.
//!
//! So there is one guard, here, and every test that leans on the home
//! directory takes it.
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
