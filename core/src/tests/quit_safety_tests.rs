//! Leaving the editor, and the questions asked on the way out.
//!
//! # What is worth testing here
//!
//! Every failure in this file is silent and permanent. A quit that does not
//! ask loses a day's work and says nothing; a question that can be answered by
//! a stray key is the same thing with an extra step; and a confirmation that
//! fires when nothing is modified trains people to type `yes' without reading
//! it, which is how the real one gets answered by reflex.
//!
//! So these check the shape of the interrogation rather than that it happens:
//! which buffers are asked about, what each key does, what *other* keys do,
//! and -- the one that matters most -- that the editor is still running at
//! every point where it should be.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// A directory of this test's own, removed when it goes.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-quit-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        /// A file with CONTENTS, and its path as a string.
        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.0.join(name)).unwrap_or_default()
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

    /// Open PATH and change its text, so it is a modified file buffer.
    fn open_and_dirty(path: &str, text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        let name = match run(&format!(r#"(find-file "{path}")"#), env, ctx) {
            LispExp::String(name) => name.to_string(),
            other => panic!("find-file answered {other:?}"),
        };
        ctx.with_buffer_mut(&name, |buf| {
            for c in text.chars() {
                buf.text.insert(c);
            }
            buf.is_modified = true;
        });
        name
    }

    fn press(ch: char, ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    /// What the one-key question on screen says, or empty when none is up.
    ///
    /// A transient keymap's message, which is a different channel from a
    /// prompt: it reports what is *true right now* rather than something that
    /// happened, so it does not expire.
    fn question(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        ctx.snapshot(env, 80, 24).prompt
    }

    /// What the typed question asks, or empty when none is open.
    ///
    /// `yes-or-no` goes through the minibuffer, so its question is the
    /// floating window's title rather than the transient message above. Two
    /// helpers because they really are two mechanisms, and a test that
    /// confused them would pass while checking nothing.
    fn typed_question(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        ctx.snapshot(env, 80, 24)
            .views
            .iter()
            .filter(|view| view.has_border)
            .find_map(|view| view.title.clone())
            .unwrap_or_default()
    }

    /// Type TEXT into whatever prompt is open and press Return.
    fn answer(text: &str, ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.with_current_buffer_mut(|buf| {
            for c in text.chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", env, ctx);
    }

    // ----------------------------------------------------------------
    // Quitting with nothing at stake
    // ----------------------------------------------------------------

    #[test]
    fn quitting_with_nothing_modified_asks_nothing() {
        // A confirmation that fires when there is nothing to lose is worse
        // than none: it is the one that teaches people to answer without
        // reading.
        let (ctx, env) = editor();
        run("(quit)", &env, &ctx);
        assert!(!ctx.is_running());
        assert_eq!(question(&ctx, &env), "");
    }

    #[test]
    fn a_modified_buffer_with_no_file_does_not_hold_the_quit_up() {
        // Typing in *scratch* marks it modified and there is nowhere to put
        // it, so asking would be a question whose answer cannot help. Without
        // this the editor could never be quit without a prompt.
        let (ctx, env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.text.insert('x');
            buf.is_modified = true;
        });
        run("(quit)", &env, &ctx);
        assert!(!ctx.is_running());
    }

    // ----------------------------------------------------------------
    // The per-buffer offer
    // ----------------------------------------------------------------

    #[test]
    fn quitting_with_a_modified_file_asks_before_it_goes() {
        // The whole point. Before this, `C-x C-c` threw the work away and
        // said nothing.
        let sandbox = Sandbox::new("asks");
        let path = sandbox.file("a.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&path, "NEW ", &env, &ctx);

        run("(quit)", &env, &ctx);
        assert!(ctx.is_running(), "it must not have quit yet");
        assert!(
            question(&ctx, &env).contains("Save"),
            "a question is on screen: {:?}",
            question(&ctx, &env)
        );
    }

    #[test]
    fn y_saves_the_buffer_and_n_leaves_it() {
        let sandbox = Sandbox::new("y-and-n");
        let kept = sandbox.file("kept.txt", "original");
        let left = sandbox.file("left.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&kept, "SAVED ", &env, &ctx);
        open_and_dirty(&left, "UNSAVED ", &env, &ctx);

        run("(save-some-buffers)", &env, &ctx);
        // The order the buffers are offered in is not fixed, so answer each
        // question by what it names rather than by position.
        for _ in 0..2 {
            let asking = question(&ctx, &env);
            if asking.contains("kept.txt") {
                press('y', &ctx, &env);
            } else {
                press('n', &ctx, &env);
            }
        }
        assert_eq!(sandbox.read("kept.txt"), "SAVED original");
        assert_eq!(sandbox.read("left.txt"), "original");
    }

    #[test]
    fn any_other_key_leaves_the_question_standing() {
        // `OnUnbound::Refuse`, and the reason for it: a question that a stray
        // key can answer is a question that gets answered by accident, and
        // one of the two answers here throws work away.
        let sandbox = Sandbox::new("stray");
        let path = sandbox.file("a.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&path, "NEW ", &env, &ctx);

        run("(save-some-buffers)", &env, &ctx);
        let asked = question(&ctx, &env);
        for stray in ['z', 'Y', '1', ' '] {
            press(stray, &ctx, &env);
            assert_eq!(
                question(&ctx, &env),
                asked,
                "{stray:?} answered a question it should not have"
            );
        }
        assert_eq!(sandbox.read("a.txt"), "original", "and nothing was written");
    }

    #[test]
    fn bang_saves_everything_left_without_asking_again() {
        let sandbox = Sandbox::new("bang");
        let one = sandbox.file("one.txt", "original");
        let two = sandbox.file("two.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&one, "A ", &env, &ctx);
        open_and_dirty(&two, "B ", &env, &ctx);

        run("(save-some-buffers)", &env, &ctx);
        press('!', &ctx, &env);
        assert_eq!(question(&ctx, &env), "", "no question is left");
        assert_eq!(sandbox.read("one.txt"), "A original");
        assert_eq!(sandbox.read("two.txt"), "B original");
    }

    #[test]
    fn q_stops_asking_and_saves_nothing_more() {
        let sandbox = Sandbox::new("stop");
        let one = sandbox.file("one.txt", "original");
        let two = sandbox.file("two.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&one, "A ", &env, &ctx);
        open_and_dirty(&two, "B ", &env, &ctx);

        run("(save-some-buffers)", &env, &ctx);
        press('q', &ctx, &env);
        assert_eq!(question(&ctx, &env), "");
        assert_eq!(sandbox.read("one.txt"), "original");
        assert_eq!(sandbox.read("two.txt"), "original");
    }

    // ----------------------------------------------------------------
    // The confirmation that guards the irreversible step
    // ----------------------------------------------------------------

    #[test]
    fn declining_to_save_then_needs_a_typed_yes_to_quit() {
        // Single keys for the offers, a whole word for the act that cannot be
        // taken back. The budget for care is spent where the loss happens.
        let sandbox = Sandbox::new("typed");
        let path = sandbox.file("a.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&path, "NEW ", &env, &ctx);

        run("(quit)", &env, &ctx);
        press('n', &ctx, &env);
        assert!(
            ctx.is_running(),
            "declining to save is not declining to stay"
        );
        assert!(
            typed_question(&ctx, &env).contains("unsaved"),
            "the confirmation is up: {:?}",
            typed_question(&ctx, &env)
        );

        answer("yes", &ctx, &env);
        assert!(!ctx.is_running());
    }

    #[test]
    fn answering_no_to_the_confirmation_keeps_the_editor_running() {
        let sandbox = Sandbox::new("stay");
        let path = sandbox.file("a.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&path, "NEW ", &env, &ctx);

        run("(quit)", &env, &ctx);
        press('n', &ctx, &env);
        answer("no", &ctx, &env);
        assert!(ctx.is_running());
        assert_eq!(sandbox.read("a.txt"), "original", "and nothing was written");
    }

    #[test]
    fn a_half_word_is_refused_and_the_question_asked_again() {
        // The point of spelling it: `y` is not an answer here, and neither is
        // anything else that is not the word. Guessing which was meant is the
        // whole thing being avoided.
        let sandbox = Sandbox::new("halfword");
        let path = sandbox.file("a.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&path, "NEW ", &env, &ctx);

        run("(quit)", &env, &ctx);
        press('n', &ctx, &env);
        answer("y", &ctx, &env);
        assert!(ctx.is_running(), "`y' did not answer it");
        assert!(
            typed_question(&ctx, &env).contains("unsaved"),
            "and it is being asked again: {:?}",
            typed_question(&ctx, &env)
        );

        answer("yes", &ctx, &env);
        assert!(!ctx.is_running());
    }

    #[test]
    fn saving_everything_quits_with_no_confirmation() {
        // Nothing is at stake once it is all on disk, so there is nothing to
        // confirm.
        let sandbox = Sandbox::new("allsaved");
        let path = sandbox.file("a.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&path, "NEW ", &env, &ctx);

        run("(quit)", &env, &ctx);
        press('y', &ctx, &env);
        assert!(!ctx.is_running());
        assert_eq!(sandbox.read("a.txt"), "NEW original");
    }

    #[test]
    fn quit_without_saving_asks_nothing_at_all() {
        let sandbox = Sandbox::new("force");
        let path = sandbox.file("a.txt", "original");
        let (ctx, env) = editor();
        open_and_dirty(&path, "NEW ", &env, &ctx);

        run("(quit-without-saving)", &env, &ctx);
        assert!(!ctx.is_running());
        assert_eq!(sandbox.read("a.txt"), "original");
    }

    // ----------------------------------------------------------------
    // The prompts on their own
    // ----------------------------------------------------------------

    #[test]
    fn y_or_n_runs_the_branch_the_key_names() {
        let (ctx, env) = editor();
        run("(setq answered nil)", &env, &ctx);
        run(
            "(y-or-n \"Well?\" (lambda () (setq answered 'yes)) (lambda () (setq answered 'no)))",
            &env,
            &ctx,
        );
        press('n', &ctx, &env);
        assert_eq!(run("answered", &env, &ctx), LispExp::symbol("no".into()));
        assert_eq!(question(&ctx, &env), "", "and the question is gone");
    }

    #[test]
    fn yes_or_no_treats_cancelling_as_no() {
        // Backing out of a question about something irreversible means not
        // doing it. The other reading -- that an abandoned prompt consents --
        // is the one that loses work.
        let (ctx, env) = editor();
        run("(setq answered nil)", &env, &ctx);
        run(
            "(yes-or-no \"Well?\" (lambda () (setq answered 'yes)) (lambda () (setq answered 'no)))",
            &env,
            &ctx,
        );
        run("(minibuffer-cancel)", &env, &ctx);
        assert_eq!(run("answered", &env, &ctx), LispExp::symbol("no".into()));
    }

    #[test]
    fn a_question_asked_from_inside_an_answer_does_not_disturb_the_first() {
        // The callbacks ride in the form the keymap is bound to, so nothing is
        // stored between asking and answering -- which is what lets one
        // question be asked from inside another's answer. `quit` does exactly
        // this: the confirmation is asked from the last offer's callback.
        let (ctx, env) = editor();
        run("(setq trail nil)", &env, &ctx);
        run(
            r#"(y-or-n "First?"
                 (lambda ()
                   (setq trail (append trail (list 'outer)))
                   (y-or-n "Second?" (lambda () (setq trail (append trail (list 'inner)))))))"#,
            &env,
            &ctx,
        );
        press('y', &ctx, &env);
        assert!(
            question(&ctx, &env).contains("Second"),
            "the second question is up: {:?}",
            question(&ctx, &env)
        );
        press('y', &ctx, &env);
        assert_eq!(
            run("trail", &env, &ctx),
            run("'(outer inner)", &env, &ctx),
            "both answers ran, in order"
        );
    }
}
