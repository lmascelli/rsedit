//! `write-file`: saving somewhere else, and following the buffer there.
//!
//! # What is worth testing here
//!
//! "Save as" is three changes that have to happen together or not at all: the
//! bytes go to the new path, the buffer starts visiting it, and every window
//! showing that buffer follows it to its new name. Miss the third and a window
//! is left naming a buffer the table no longer holds -- the dangling state
//! that has taken this editor down before, and which will not show up in a
//! test that only checks the file's contents.
//!
//! The other half is what happens when it *cannot* work: a path another buffer
//! already has open, and a write that fails. Both must leave the buffer
//! visiting whatever it was visiting before, because the next `C-x C-s` has to
//! go somewhere that still works.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-write-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).display().to_string()
        }

        fn read(&self, name: &str) -> Option<String> {
            std::fs::read_to_string(self.0.join(name)).ok()
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

    fn write_file(path: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        run(&format!(r#"(write-file "{path}")"#), env, ctx)
    }

    /// What the focused window is showing.
    fn shown(ctx: &Ctx) -> String {
        ctx.windows(|windows| {
            windows
                .root()
                .window(windows.focused())
                .map(|window| window.buffer_name.clone())
                .unwrap_or_default()
        })
    }

    fn visiting(ctx: &Ctx, name: &str) -> Option<String> {
        ctx.with_buffer(name, |buf| buf.file_path.clone()).flatten()
    }

    // ----------------------------------------------------------------
    // The three changes that go together
    // ----------------------------------------------------------------

    #[test]
    fn the_text_lands_at_the_new_path() {
        let sandbox = Sandbox::new("lands");
        let old = sandbox.file("old.txt", "contents");
        let (ctx, env) = editor();
        open(&old, &env, &ctx);

        write_file(&sandbox.path("new.txt"), &env, &ctx);
        assert_eq!(sandbox.read("new.txt").as_deref(), Some("contents"));
    }

    #[test]
    fn the_original_file_is_left_alone() {
        // This writes a copy and stops looking at the original. Moving the
        // file would be a different, and much more surprising, command.
        let sandbox = Sandbox::new("original");
        let old = sandbox.file("old.txt", "contents");
        let (ctx, env) = editor();
        open(&old, &env, &ctx);

        write_file(&sandbox.path("new.txt"), &env, &ctx);
        assert_eq!(sandbox.read("old.txt").as_deref(), Some("contents"));
    }

    #[test]
    fn the_buffer_visits_the_new_path_from_then_on() {
        let sandbox = Sandbox::new("visits");
        let old = sandbox.file("old.txt", "one");
        let (ctx, env) = editor();
        open(&old, &env, &ctx);

        write_file(&sandbox.path("new.txt"), &env, &ctx);
        // Type more and save the ordinary way: it has to go to the new file.
        ctx.with_current_buffer_mut(|buf| buf.text.insert('!'));
        run("(save-buffer)", &env, &ctx);
        assert_eq!(sandbox.read("new.txt").as_deref(), Some("!one"));
        assert_eq!(sandbox.read("old.txt").as_deref(), Some("one"));
    }

    #[test]
    fn the_buffer_is_renamed_after_the_new_file() {
        let sandbox = Sandbox::new("renamed");
        let old = sandbox.file("old.txt", "contents");
        let (ctx, env) = editor();
        open(&old, &env, &ctx);

        write_file(&sandbox.path("new.txt"), &env, &ctx);
        assert_eq!(ctx.get_current_buffer_name(), "new.txt");
        assert!(!ctx.has_buffer("old.txt"), "the old name is gone");
        assert_eq!(
            visiting(&ctx, "new.txt").as_deref(),
            Some(&*sandbox.path("new.txt"))
        );
    }

    #[test]
    fn the_window_showing_it_follows_the_rename() {
        // The change that is invisible in a test about file contents, and the
        // one that leaves a window naming a buffer the table no longer holds.
        let sandbox = Sandbox::new("window");
        let old = sandbox.file("old.txt", "contents");
        let (ctx, env) = editor();
        open(&old, &env, &ctx);
        assert_eq!(shown(&ctx), "old.txt");

        write_file(&sandbox.path("new.txt"), &env, &ctx);
        assert_eq!(shown(&ctx), "new.txt");
    }

    #[test]
    fn every_window_showing_it_follows_not_only_the_focused_one() {
        // More than one window may be showing the same buffer, and fixing
        // only the focused one leaves the others exactly as broken.
        let sandbox = Sandbox::new("twowindows");
        let old = sandbox.file("old.txt", "contents");
        let (ctx, env) = editor();
        open(&old, &env, &ctx);
        run("(split-window-below)", &env, &ctx);

        write_file(&sandbox.path("new.txt"), &env, &ctx);
        let names = ctx.windows(|windows| {
            windows
                .root()
                .window_ids()
                .into_iter()
                .filter_map(|id| windows.root().window(id).map(|w| w.buffer_name.clone()))
                .collect::<Vec<_>>()
        });
        assert!(
            names.iter().all(|name| name == "new.txt"),
            "every window followed: {names:?}"
        );
    }

    #[test]
    fn the_buffer_is_no_longer_modified() {
        // It has just been written. Leaving the flag on would make the next
        // quit ask about a buffer that is already safely on disk.
        let sandbox = Sandbox::new("clean");
        let (ctx, env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.text.insert('x');
            buf.is_modified = true;
        });

        write_file(&sandbox.path("fresh.txt"), &env, &ctx);
        assert!(!ctx.with_current_buffer(|buf| buf.is_modified));
    }

    #[test]
    fn a_scratch_buffer_can_be_written_out() {
        // The main use: something typed with nowhere to go, given somewhere.
        let sandbox = Sandbox::new("scratch");
        let (ctx, env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            for c in "notes".chars() {
                buf.text.insert(c);
            }
        });

        write_file(&sandbox.path("notes.txt"), &env, &ctx);
        assert_eq!(sandbox.read("notes.txt").as_deref(), Some("notes"));
        assert_eq!(ctx.get_current_buffer_name(), "notes.txt");
    }

    #[test]
    fn the_mode_is_reconsidered_from_the_new_name() {
        // Saving a scratch buffer as `x.toy` should put it in the mode that
        // name asks for: what decides a mode is the file name, and the file
        // name has just changed.
        //
        // The rule is registered here rather than relying on a shipped
        // language module, because a test binary has no `data/lisp` beside it
        // -- and because what is under test is the reconsidering, not which
        // modes happen to exist.
        let sandbox = Sandbox::new("mode");
        let (ctx, env) = editor();
        run(r#"(add-auto-mode "\\.toy$" 'toy-mode)"#, &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.current_mode.clone()),
            "fundamental-mode"
        );

        write_file(&sandbox.path("x.toy"), &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.current_mode.clone()),
            "toy-mode"
        );
    }

    // ----------------------------------------------------------------
    // Names that are already taken
    // ----------------------------------------------------------------

    #[test]
    fn a_taken_name_is_made_unique_rather_than_clobbering() {
        // Two files called `notes.txt` in different directories, which is the
        // case `find-file`'s `<2>` already exists for. A table keyed by name
        // cannot hold two of them, so the alternative to uniquifying is losing
        // one.
        let sandbox = Sandbox::new("unique");
        let other_dir = sandbox.0.join("sub");
        std::fs::create_dir_all(&other_dir).expect("subdirectory");
        let first = sandbox.file("notes.txt", "first");
        let (ctx, env) = editor();
        open(&first, &env, &ctx);

        // A second buffer, written out to a different directory under the
        // same file name.
        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);
        let elsewhere = other_dir.join("notes.txt").display().to_string();
        write_file(&elsewhere, &env, &ctx);

        assert_eq!(ctx.get_current_buffer_name(), "notes.txt<2>");
        assert!(ctx.has_buffer("notes.txt"), "the first one is untouched");
        assert_eq!(visiting(&ctx, "notes.txt").as_deref(), Some(&*first));
    }

    #[test]
    fn writing_to_a_path_another_buffer_has_open_is_refused() {
        // Two buffers over one file is how one of them silently loses its
        // work: both think they are the truth and the last save wins.
        let sandbox = Sandbox::new("conflict");
        let taken = sandbox.file("taken.txt", "theirs");
        let (ctx, env) = editor();
        open(&taken, &env, &ctx);

        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);
        ctx.with_current_buffer_mut(|buf| buf.text.insert('x'));
        assert!(write_file(&taken, &env, &ctx).is_nil(), "refused");
        assert_eq!(
            sandbox.read("taken.txt").as_deref(),
            Some("theirs"),
            "and nothing was written over it"
        );
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    }

    #[test]
    fn writing_to_the_path_it_already_visits_is_just_a_save() {
        // `write-file` to where you already are is not a conflict with
        // yourself. It saves, and there is nothing to rename.
        let sandbox = Sandbox::new("same");
        let path = sandbox.file("same.txt", "old");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);
        ctx.with_current_buffer_mut(|buf| buf.text.insert('!'));

        assert!(!write_file(&path, &env, &ctx).is_nil());
        assert_eq!(sandbox.read("same.txt").as_deref(), Some("!old"));
        assert_eq!(ctx.get_current_buffer_name(), "same.txt");
    }

    // ----------------------------------------------------------------
    // When it cannot work
    // ----------------------------------------------------------------

    #[test]
    fn a_failed_write_changes_nothing_at_all() {
        // The buffer must still be visiting what it was visiting, or the next
        // `C-x C-s` has nowhere that works to go.
        let sandbox = Sandbox::new("failed");
        let old = sandbox.file("old.txt", "contents");
        let (ctx, env) = editor();
        open(&old, &env, &ctx);
        ctx.with_current_buffer_mut(|buf| buf.is_modified = true);

        let nowhere = sandbox.path("no/such/directory/new.txt");
        assert!(write_file(&nowhere, &env, &ctx).is_nil(), "it failed");
        assert_eq!(ctx.get_current_buffer_name(), "old.txt");
        assert_eq!(visiting(&ctx, "old.txt").as_deref(), Some(&*old));
        assert!(
            ctx.with_current_buffer(|buf| buf.is_modified),
            "and it is still modified, because it still is"
        );
        assert_eq!(shown(&ctx), "old.txt");
    }

    #[test]
    fn write_file_is_a_command_so_it_can_be_run_and_rebound() {
        let (ctx, env) = editor();
        assert_eq!(run("(commandp 'write-file)", &env, &ctx), LispExp::t());
    }
}
