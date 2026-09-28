//! Noticing that a file changed under a buffer, and what is done about it.
//!
//! # What is worth testing here, and how
//!
//! The worker runs on a timer on another thread, which is exactly the shape a
//! test cannot wait on without being a race. So these do not wait: they drive
//! the turn the scheduler would drive, one call at a time, and check what one
//! turn decided.
//!
//! The decisions are asymmetric on purpose, and the asymmetry is the whole
//! feature. A buffer with nothing unsaved is reloaded silently, because that
//! loses nothing and is the pleasant half. A buffer with unsaved edits is
//! never touched by a background job under any circumstances -- it is marked,
//! and the *save* asks. Every test below is about keeping those two apart,
//! including in the window where a buffer becomes modified halfway through a
//! check.
//!
//! Timestamps are set explicitly rather than waited for. A test that wrote a
//! file twice and hoped the clock had moved would pass on a slow filesystem
//! and fail on a fast one.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, FileStamp, OnDisk, disk::compare, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    type Ctx = EditorState<GapBuffer>;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-watch-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }

        /// Write CONTENTS as something *else* would: new bytes, and a
        /// timestamp that is definitely not the one recorded.
        ///
        /// Set rather than waited for, because the granularity of a
        /// filesystem's clock is not something a test should depend on.
        fn rewrite(&self, name: &str, contents: &str) {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("rewriting");
            let later = SystemTime::now() + Duration::from_secs(60);
            let _ = std::fs::File::open(&path).map(|file| file.set_modified(later));
        }

        fn remove(&self, name: &str) {
            let _ = std::fs::remove_file(self.0.join(name));
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn try_run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        try_run(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn open(path: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run(&format!(r#"(find-file "{path}")"#), env, ctx) {
            LispExp::String(name) => name.to_string(),
            other => panic!("find-file answered {other:?}"),
        }
    }

    /// One turn of the watcher, over every buffer.
    fn sweep(ctx: &Ctx) {
        ctx.watch_files_turn(0);
    }

    fn text(ctx: &Ctx, name: &str) -> String {
        ctx.with_buffer(name, |buf| buf.text.to_string())
            .unwrap_or_default()
    }

    fn is_stale(ctx: &Ctx, name: &str) -> bool {
        ctx.with_buffer(name, |buf| buf.stale).unwrap_or(false)
    }

    /// Type INSERT into NAME's buffer the way a person would.
    ///
    /// Through the real editing path rather than by poking `text` and setting
    /// the flag, because the two are not the same: a real edit also bumps the
    /// version, and the version is what the watcher's store step compares. A
    /// helper that skipped it would let a test pass against a guard that does
    /// not work.
    fn dirty(ctx: &Ctx, name: &str, insert: &str, env: &Arc<Env<Ctx>>) {
        assert_eq!(
            ctx.get_current_buffer_name(),
            name,
            "this helper types into the current buffer"
        );
        let before = ctx.with_buffer(name, |buf| buf.version);
        run(&format!(r#"(insert "{insert}")"#), env, ctx);
        assert_ne!(
            ctx.with_buffer(name, |buf| buf.version),
            before,
            "an edit has to bump the version, or half these tests check nothing"
        );
    }

    // ----------------------------------------------------------------
    // The comparison, on its own
    // ----------------------------------------------------------------

    #[test]
    fn nothing_recorded_is_not_the_same_as_changed() {
        // A buffer that was never stamped has not been *seen* to differ from
        // anything. Reading "I do not know" as "it changed" would reload a
        // buffer on the strength of never having looked at it.
        let stamp = FileStamp {
            modified: Some(SystemTime::UNIX_EPOCH),
            len: 3,
        };
        assert_eq!(compare(None, Some(stamp)), OnDisk::Unknown);
        assert_eq!(compare(None, None), OnDisk::Unknown);
    }

    #[test]
    fn a_timestamp_that_went_backwards_still_counts_as_changed() {
        // `git checkout` restores older timestamps, `touch -d` writes whatever
        // it is told, and a corrected clock disagrees with itself. "Newer than
        // mine" is a question about two clocks; "different" is a question the
        // filesystem can answer.
        let recorded = FileStamp {
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000)),
            len: 10,
        };
        let older = FileStamp {
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(500)),
            len: 10,
        };
        assert_eq!(compare(Some(recorded), Some(older)), OnDisk::Changed);
    }

    #[test]
    fn a_same_second_write_is_caught_by_the_length() {
        // The narrow case a timestamp alone misses: a write inside the
        // filesystem's granularity. Recording the length alongside is what
        // shrinks it to "same second *and* same byte count".
        let recorded = FileStamp {
            modified: Some(SystemTime::UNIX_EPOCH),
            len: 10,
        };
        let same_time = FileStamp {
            modified: Some(SystemTime::UNIX_EPOCH),
            len: 11,
        };
        assert_eq!(compare(Some(recorded), Some(same_time)), OnDisk::Changed);
    }

    #[test]
    fn a_file_that_is_gone_says_so_rather_than_saying_changed() {
        let recorded = FileStamp {
            modified: Some(SystemTime::UNIX_EPOCH),
            len: 10,
        };
        assert_eq!(compare(Some(recorded), None), OnDisk::Gone);
    }

    // ----------------------------------------------------------------
    // A clean buffer follows the file
    // ----------------------------------------------------------------

    #[test]
    fn a_clean_buffer_is_reloaded_when_the_file_changes() {
        // The pleasant half: switch branches and the windows are already
        // showing the new text.
        let sandbox = Sandbox::new("reload");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        assert_eq!(text(&ctx, &name), "before");

        sandbox.rewrite("a.txt", "after");
        sweep(&ctx);
        assert_eq!(text(&ctx, &name), "after");
        assert!(!is_stale(&ctx, &name), "nothing to warn about");
    }

    #[test]
    fn an_unchanged_file_is_left_entirely_alone() {
        // The common case, and the one that must cost nothing: a sweep over a
        // file nobody touched must not bump the version, or the highlighter
        // would recolour every open buffer every three seconds.
        let sandbox = Sandbox::new("quiet");
        let path = sandbox.file("a.txt", "steady");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        let before = ctx.with_buffer(&name, |buf| buf.version);

        sweep(&ctx);
        sweep(&ctx);
        assert_eq!(ctx.with_buffer(&name, |buf| buf.version), before);
        assert!(!is_stale(&ctx, &name));
    }

    #[test]
    fn the_editors_own_save_is_not_mistaken_for_somebody_elses_write() {
        // The stamp has to be taken *after* writing, or every save looks like
        // an external change and the editor reports a conflict with itself.
        let sandbox = Sandbox::new("ownsave");
        let path = sandbox.file("a.txt", "one");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        dirty(&ctx, &name, "!", &env);
        run("(save-buffer)", &env, &ctx);
        let after_save = ctx.with_buffer(&name, |buf| buf.version);

        sweep(&ctx);
        assert!(!is_stale(&ctx, &name), "its own write is not a conflict");
        // The version, not the text. Without the stamp being retaken the
        // sweep sees a changed file and *reloads* it -- which lands the same
        // characters in the buffer and looks identical from the outside,
        // while having thrown away the undo history and re-run every
        // highlighter turn for a file nobody else touched.
        assert_eq!(
            ctx.with_buffer(&name, |buf| buf.version),
            after_save,
            "the sweep did something, and there was nothing to do"
        );
        assert_eq!(text(&ctx, &name), "!one");
    }

    #[test]
    fn reloading_forgets_the_undo_history_rather_than_offering_a_state_the_file_never_had() {
        let sandbox = Sandbox::new("undo");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        sandbox.rewrite("a.txt", "after");
        sweep(&ctx);
        run("(undo)", &env, &ctx);
        assert_eq!(
            text(&ctx, &name),
            "after",
            "undo cannot walk back into text that was never edited"
        );
    }

    // ----------------------------------------------------------------
    // A dirty buffer is never touched
    // ----------------------------------------------------------------

    #[test]
    fn a_modified_buffer_is_flagged_and_left_exactly_as_it_is() {
        // The half this whole tier exists for. A background job must never
        // decide to throw away unsaved work.
        let sandbox = Sandbox::new("dirty");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        dirty(&ctx, &name, "MINE ", &env);

        sandbox.rewrite("a.txt", "theirs");
        sweep(&ctx);
        assert_eq!(text(&ctx, &name), "MINE before", "untouched");
        assert!(is_stale(&ctx, &name), "but marked");
    }

    #[test]
    fn a_file_that_disappeared_does_not_empty_the_buffer() {
        // The text in front of the user is now the only copy there is, which
        // makes it more precious rather than less.
        let sandbox = Sandbox::new("gone");
        let path = sandbox.file("a.txt", "only copy");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        sandbox.remove("a.txt");
        sweep(&ctx);
        assert_eq!(text(&ctx, &name), "only copy");
        assert!(is_stale(&ctx, &name));
    }

    #[test]
    fn saving_a_stale_buffer_asks_before_it_overwrites() {
        // The moment the flag is for. Noticing the change did not prompt --
        // this is where it can still be stopped.
        let sandbox = Sandbox::new("conflict");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        dirty(&ctx, &name, "MINE ", &env);
        sandbox.rewrite("a.txt", "theirs");
        sweep(&ctx);

        run("(save-buffer)", &env, &ctx);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "theirs",
            "nothing was written while the question is up"
        );
        assert!(ctx.minibuffer_is_open(), "and a question is up");
    }

    #[test]
    fn confirming_the_overwrite_writes_and_clears_the_flag() {
        let sandbox = Sandbox::new("overwrite");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        dirty(&ctx, &name, "MINE ", &env);
        sandbox.rewrite("a.txt", "theirs");
        sweep(&ctx);

        run("(save-buffer)", &env, &ctx);
        ctx.with_current_buffer_mut(|buf| {
            for c in "yes".chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "MINE before"
        );
        assert!(!is_stale(&ctx, &name));
    }

    #[test]
    fn a_clean_buffer_saves_without_being_asked_anything() {
        let sandbox = Sandbox::new("nofuss");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        dirty(&ctx, &name, "MINE ", &env);

        run("(save-buffer)", &env, &ctx);
        assert!(!ctx.minibuffer_is_open(), "no question");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "MINE before"
        );
    }

    // ----------------------------------------------------------------
    // revert-buffer
    // ----------------------------------------------------------------

    #[test]
    fn reverting_a_clean_buffer_just_reads_the_file() {
        let sandbox = Sandbox::new("revert-clean");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        sandbox.rewrite("a.txt", "after");

        run("(revert-buffer)", &env, &ctx);
        assert!(!ctx.minibuffer_is_open(), "nothing to lose, nothing to ask");
        assert_eq!(text(&ctx, &name), "after");
    }

    #[test]
    fn reverting_a_modified_buffer_asks_first() {
        let sandbox = Sandbox::new("revert-dirty");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        dirty(&ctx, &name, "MINE ", &env);

        run("(revert-buffer)", &env, &ctx);
        assert!(ctx.minibuffer_is_open());
        assert_eq!(text(&ctx, &name), "MINE before", "not yet");

        ctx.with_current_buffer_mut(|buf| {
            for c in "yes".chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(text(&ctx, &name), "before");
    }

    #[test]
    fn declining_the_revert_keeps_the_edits() {
        let sandbox = Sandbox::new("revert-no");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        dirty(&ctx, &name, "MINE ", &env);

        run("(revert-buffer)", &env, &ctx);
        ctx.with_current_buffer_mut(|buf| {
            for c in "no".chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(text(&ctx, &name), "MINE before");
    }

    #[test]
    fn reverting_clears_the_stale_flag() {
        let sandbox = Sandbox::new("revert-clears");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        dirty(&ctx, &name, "MINE ", &env);
        sandbox.rewrite("a.txt", "theirs");
        sweep(&ctx);
        assert!(is_stale(&ctx, &name));

        run("(revert-buffer--reread)", &env, &ctx);
        assert_eq!(text(&ctx, &name), "theirs");
        assert!(!is_stale(&ctx, &name));
    }

    // ----------------------------------------------------------------
    // The window between reading the file and using what was read
    // ----------------------------------------------------------------

    #[test]
    fn a_buffer_typed_into_while_the_file_was_being_read_is_not_reloaded() {
        // The race the whole plan/store split exists to make testable. The
        // check decides to reload a clean buffer, the file is read with no
        // lock held -- and in that window the user types. Storing the result
        // anyway would throw away what they just wrote, from a background
        // thread, with no warning: the exact loss this tier of work is about.
        let sandbox = Sandbox::new("race");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        sandbox.rewrite("a.txt", "theirs");
        // Decided while the buffer is still clean, so the verdict is "reload".
        let check = ctx.file_check_for(&name).expect("a change to act on");
        // ... and then the user types, as they are entitled to.
        dirty(&ctx, &name, "MINE ", &env);
        ctx.store_file_check(check);

        assert_eq!(
            text(&ctx, &name),
            "MINE before",
            "what was typed in the window survived"
        );
        assert!(is_stale(&ctx, &name), "and the conflict is recorded");
    }

    #[test]
    fn a_buffer_edited_and_saved_in_the_window_is_not_reloaded_either() {
        // Same window, but the buffer comes out of it *clean* -- so
        // `is_modified` alone would not catch it. The version is what does:
        // this is no longer the buffer the check looked at.
        let sandbox = Sandbox::new("race-saved");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        sandbox.rewrite("a.txt", "theirs");
        let check = ctx.file_check_for(&name).expect("a change to act on");
        dirty(&ctx, &name, "MINE ", &env);
        run("(save-buffer)", &env, &ctx);
        // The save answered the conflict question, so the buffer is clean and
        // unflagged -- and the stale check is still holding text from before
        // any of that.
        ctx.with_current_buffer_mut(|buf| buf.stale = false);
        ctx.store_file_check(check);

        assert_eq!(text(&ctx, &name), "MINE before");
    }

    #[test]
    fn a_check_that_is_still_true_is_applied() {
        // The other side of the guard: nothing happened in the window, so the
        // reload goes ahead. Without this the two tests above would pass
        // against a store step that never did anything.
        let sandbox = Sandbox::new("race-quiet");
        let path = sandbox.file("a.txt", "before");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        sandbox.rewrite("a.txt", "theirs");
        let check = ctx.file_check_for(&name).expect("a change to act on");
        ctx.store_file_check(check);
        assert_eq!(text(&ctx, &name), "theirs");
    }

    // ----------------------------------------------------------------
    // Scope and switches
    // ----------------------------------------------------------------

    #[test]
    fn a_buffer_visiting_no_file_is_passed_over() {
        let sandbox = Sandbox::new("nofile");
        let _ = sandbox.file("unused.txt", "x");
        let (ctx, _env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.text.insert('x');
            buf.is_modified = true;
        });

        sweep(&ctx);
        assert!(!is_stale(&ctx, "*scratch*"));
    }

    #[test]
    fn a_turn_looks_at_a_bounded_number_of_files() {
        // The bound is what keeps one hung filesystem from putting every open
        // file between the user and their next colour update. The cursor comes
        // back so the next turn carries on where this one stopped.
        let (ctx, _env) = editor();
        let next = ctx.watch_files_turn(0);
        assert!(
            next <= crate::modes::watcher::FILES_PER_TURN,
            "a turn advanced the cursor by more than its batch: {next}"
        );
    }
}
