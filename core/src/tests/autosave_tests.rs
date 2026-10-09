//! Auto-save: keeping a copy of work that has never been written.
//!
//! # What is worth testing here
//!
//! This is the only thing in the editor that helps when the editor *stops
//! without being asked*, so its failures are all discovered at the worst
//! possible moment and never before. Three of them are silent:
//!
//! - A copy that is never written, because the "has this changed" test was
//!   wrong. Looks identical to a working feature until you need it.
//! - A copy written for every buffer every turn, because the test was wrong
//!   the other way. Costs a write every half minute per open file, forever,
//!   and nothing complains.
//! - Two files sharing one copy, because the naming did not account for two
//!   `main.rs` in different directories. Recovers the wrong work.
//!
//! So these check the naming as arithmetic, the decision to write as a
//! decision, and the lifetime of the file at each end.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use crate::modes::autosave::auto_save_path;
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-autosave-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }

        fn sub(&self, name: &str) -> String {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).expect("subdirectory");
            dir.display().to_string()
        }

        /// What the auto-save file beside NAME holds, if there is one.
        fn copy_beside(&self, name: &str) -> Option<String> {
            std::fs::read_to_string(self.0.join(format!("#{name}#"))).ok()
        }

        fn entries(&self, dir: &str) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into()))
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
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

    /// One turn of the auto-saver over every buffer.
    fn sweep(ctx: &Ctx, directory: Option<&str>) {
        ctx.auto_save_turn(0, directory);
    }

    fn text(ctx: &Ctx, name: &str) -> String {
        ctx.with_buffer(name, |buf| buf.text.to_string())
            .unwrap_or_default()
    }

    // ----------------------------------------------------------------
    // Where the copy goes, as arithmetic
    // ----------------------------------------------------------------

    #[test]
    fn the_copy_sits_beside_its_own_file_by_default() {
        assert_eq!(
            auto_save_path("/home/me/notes.txt", None),
            PathBuf::from("/home/me/#notes.txt#")
        );
    }

    #[test]
    fn a_configured_directory_takes_every_copy() {
        assert_eq!(
            auto_save_path("/home/me/notes.txt", Some("/tmp/saves")),
            PathBuf::from("/tmp/saves/#!home!me!notes.txt#")
        );
    }

    #[test]
    fn two_files_of_the_same_name_do_not_share_a_copy() {
        // The case a shared directory exists to break and must not: `src` and
        // `tests` both holding a `main.rs`. Sharing one copy means recovering
        // the wrong work, which is worse than recovering none.
        let one = auto_save_path("/p/src/main.rs", Some("/tmp/saves"));
        let two = auto_save_path("/p/tests/main.rs", Some("/tmp/saves"));
        assert_ne!(one, two);
    }

    #[test]
    fn a_path_containing_a_bang_cannot_collide_with_one_containing_a_separator() {
        // Without doubling the literal `!`, `/a!b/c` and `/a/b/c` both flatten
        // to `!a!b!c` and one buffer's recovery file is quietly another's.
        let bang = auto_save_path("/a!b/c", Some("/tmp"));
        let slash = auto_save_path("/a/b/c", Some("/tmp"));
        assert_ne!(bang, slash);
    }

    #[test]
    fn a_file_at_the_root_still_gets_a_name() {
        assert_eq!(
            auto_save_path("/notes.txt", None),
            PathBuf::from("/#notes.txt#")
        );
    }

    // ----------------------------------------------------------------
    // When a copy is written
    // ----------------------------------------------------------------

    #[test]
    fn unsaved_work_is_written_to_a_copy() {
        let sandbox = Sandbox::new("writes");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);
        run(r#"(insert "UNSAVED ")"#, &env, &ctx);

        sweep(&ctx, None);
        assert_eq!(
            sandbox.copy_beside("a.txt").as_deref(),
            Some("UNSAVED on disk")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "on disk",
            "and the real file is untouched"
        );
    }

    #[test]
    fn a_buffer_with_nothing_unsaved_gets_no_copy() {
        let sandbox = Sandbox::new("clean");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        sweep(&ctx, None);
        assert_eq!(sandbox.copy_beside("a.txt"), None);
    }

    #[test]
    fn an_untouched_buffer_is_not_rewritten_every_turn() {
        // The failure that costs a write every half minute per open file,
        // forever, and that nothing ever complains about. "Modified" stays
        // true until the save, so the version is what has to be compared.
        let sandbox = Sandbox::new("idle");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        run(r#"(insert "X")"#, &env, &ctx);

        sweep(&ctx, None);
        let after_first = ctx.with_buffer(&name, |buf| buf.auto_saved_at);
        // Nothing has changed, so the second turn must find nothing to do.
        assert!(
            ctx.auto_save_for(&name, None).is_none(),
            "a second turn would have written the same bytes again"
        );
        sweep(&ctx, None);
        assert_eq!(ctx.with_buffer(&name, |buf| buf.auto_saved_at), after_first);
    }

    #[test]
    fn typing_again_makes_a_new_copy_due() {
        let sandbox = Sandbox::new("again");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        run(r#"(insert "one ")"#, &env, &ctx);
        sweep(&ctx, None);

        run(r#"(insert "two ")"#, &env, &ctx);
        assert!(ctx.auto_save_for(&name, None).is_some());
        sweep(&ctx, None);
        assert_eq!(
            sandbox.copy_beside("a.txt").as_deref(),
            Some("one two on disk")
        );
    }

    #[test]
    fn a_buffer_with_no_file_is_passed_over() {
        // There would be nowhere to recover it *to*, and a copy of a listing
        // or a backtrace is a copy of a view.
        let sandbox = Sandbox::new("nofile");
        let (ctx, env) = editor();
        run(r#"(insert "notes")"#, &env, &ctx);

        sweep(&ctx, None);
        assert!(sandbox.entries(&sandbox.0.display().to_string()).is_empty());
    }

    #[test]
    fn every_copy_lands_in_the_configured_directory() {
        let sandbox = Sandbox::new("directory");
        let saves = sandbox.sub("saves");
        let one = sandbox.file("one.txt", "1");
        let two = sandbox.file("two.txt", "2");
        let (ctx, env) = editor();
        open(&one, &env, &ctx);
        run(r#"(insert "A")"#, &env, &ctx);
        open(&two, &env, &ctx);
        run(r#"(insert "B")"#, &env, &ctx);

        sweep(&ctx, Some(&saves));
        assert_eq!(sandbox.entries(&saves).len(), 2, "both went there");
        assert_eq!(sandbox.copy_beside("one.txt"), None, "and none beside");
    }

    // ----------------------------------------------------------------
    // When a copy is thrown away
    // ----------------------------------------------------------------

    #[test]
    fn saving_the_real_file_throws_the_copy_away() {
        // Its job is done: what it held is now in the file it stood in for.
        // Left behind, it would offer stale work to a later recovery.
        let sandbox = Sandbox::new("discard");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);
        run(r#"(insert "X")"#, &env, &ctx);
        sweep(&ctx, None);
        assert!(sandbox.copy_beside("a.txt").is_some());

        run("(save-buffer)", &env, &ctx);
        assert_eq!(sandbox.copy_beside("a.txt"), None);
    }

    #[test]
    fn killing_a_clean_buffer_throws_the_copy_away() {
        let sandbox = Sandbox::new("kill-clean");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        run(r#"(insert "X")"#, &env, &ctx);
        sweep(&ctx, None);
        run("(save-buffer)", &env, &ctx);
        // A copy that exists again, with the buffer left clean.
        std::fs::write(sandbox.0.join("#a.txt#"), "leftover").expect("planting one");

        run(&format!(r#"(kill-buffer "{name}")"#), &env, &ctx);
        assert_eq!(sandbox.copy_beside("a.txt"), None);
    }

    #[test]
    fn killing_a_modified_buffer_keeps_the_copy() {
        // `kill-buffer` does not yet ask, so the copy left behind is the only
        // remaining trace of the work. Deleting it here would turn one
        // un-asked keystroke into a permanent loss.
        let sandbox = Sandbox::new("kill-dirty");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        run(r#"(insert "UNSAVED ")"#, &env, &ctx);
        sweep(&ctx, None);

        run(&format!(r#"(kill-buffer "{name}")"#), &env, &ctx);
        assert_eq!(
            sandbox.copy_beside("a.txt").as_deref(),
            Some("UNSAVED on disk"),
            "the work is still recoverable"
        );
    }

    #[test]
    fn writing_the_buffer_elsewhere_throws_away_the_old_copy() {
        // The copy that exists is named after where the buffer *used* to
        // live, so it has to go before the buffer is repointed.
        let sandbox = Sandbox::new("writefile");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);
        run(r#"(insert "X")"#, &env, &ctx);
        sweep(&ctx, None);

        run(
            &format!(r#"(write-file "{}")"#, sandbox.0.join("b.txt").display()),
            &env,
            &ctx,
        );
        assert_eq!(sandbox.copy_beside("a.txt"), None);
    }

    // ----------------------------------------------------------------
    // Getting it back
    // ----------------------------------------------------------------

    #[test]
    fn opening_a_file_that_has_a_copy_says_so() {
        // A recovery copy nobody is told about is a recovery copy nobody
        // uses, and the moment it matters is exactly the moment somebody
        // reopens the file after a crash.
        let sandbox = Sandbox::new("notice");
        let path = sandbox.file("a.txt", "on disk");
        std::fs::write(sandbox.0.join("#a.txt#"), "rescued").expect("planting one");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        assert!(
            ctx.get_echo_message().contains("recover-file"),
            "it points at the way back: {:?}",
            ctx.get_echo_message()
        );
    }

    #[test]
    fn recovering_puts_the_copy_in_the_buffer_and_leaves_the_file_alone() {
        let sandbox = Sandbox::new("recover");
        let path = sandbox.file("a.txt", "on disk");
        std::fs::write(sandbox.0.join("#a.txt#"), "rescued work").expect("planting one");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        run("(recover-file)", &env, &ctx);
        ctx.with_current_buffer_mut(|buf| {
            for c in "yes".chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", &env, &ctx);

        assert_eq!(text(&ctx, &name), "rescued work");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "on disk",
            "recovery does not write anything"
        );
    }

    #[test]
    fn recovered_text_is_marked_modified_so_it_cannot_be_quit_away_silently() {
        // What is in the buffer is not what is in the file. Saying otherwise
        // would let the rescued work be discarded by a quit that asked
        // nothing -- the exact loss this file exists to undo.
        let sandbox = Sandbox::new("recover-dirty");
        let path = sandbox.file("a.txt", "on disk");
        std::fs::write(sandbox.0.join("#a.txt#"), "rescued").expect("planting one");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        run("(recover-file--adopt)", &env, &ctx);
        assert!(ctx.with_current_buffer(|buf| buf.is_modified));
    }

    #[test]
    fn declining_the_recovery_leaves_the_buffer_as_it_was() {
        let sandbox = Sandbox::new("recover-no");
        let path = sandbox.file("a.txt", "on disk");
        std::fs::write(sandbox.0.join("#a.txt#"), "rescued").expect("planting one");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        run("(recover-file)", &env, &ctx);
        ctx.with_current_buffer_mut(|buf| {
            for c in "no".chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(text(&ctx, &name), "on disk");
    }

    #[test]
    fn recovering_with_no_copy_says_so_rather_than_emptying_the_buffer() {
        let sandbox = Sandbox::new("recover-none");
        let path = sandbox.file("a.txt", "on disk");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);

        assert!(run("(recover-file)", &env, &ctx).is_nil());
        assert_eq!(text(&ctx, &name), "on disk");
        assert!(ctx.get_echo_message().contains("No auto-save"));
    }

    #[test]
    fn saving_after_a_recovery_accepts_it_and_clears_the_copy() {
        // The whole round trip: crash, reopen, recover, keep.
        let sandbox = Sandbox::new("roundtrip");
        let path = sandbox.file("a.txt", "on disk");
        std::fs::write(sandbox.0.join("#a.txt#"), "rescued work").expect("planting one");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);

        run("(recover-file--adopt)", &env, &ctx);
        run("(save-buffer)", &env, &ctx);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "rescued work"
        );
        assert_eq!(sandbox.copy_beside("a.txt"), None);
    }
}
