//! Keyboard macros: recording keys, and pressing them again.
//!
//! # What is worth testing here
//!
//! That a replay really is the keys being pressed. That is the whole claim,
//! and the ways it can quietly not be true are the interesting ones:
//!
//! - A key that runs no command. `C-u` and the digits after it are taken by
//!   the prefix-argument reader; `zap-to-char` reads its character through a
//!   key capture. A recorder watching *commands* would lose both, and the
//!   second replays as a wait for a character that never arrives.
//! - The keys that ended the recording. They are pressed while recording, so
//!   without care every macro ends by stopping a recording that is not
//!   running.
//! - Replaying while recording. A macro that calls a macro must record the
//!   call, not what the call expands to, or recording one that calls another
//!   doubles its body.
//!
//! And the two things nobody would look at until much later: that a replay is
//! one undo step per repetition, and that a replay stops at the first key that
//! failed rather than pressing the rest into a state the recording never saw.
#[cfg(test)]
mod tests {
    use crate::buffer::BufferTrait;
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        // The bindings live in a `.lisp` file and the commands in Rust, so a
        // test that presses keys has to load the file the keys are in.
        // Included rather than loaded by name: `eval-file` reads the installed
        // data directory, which the tests deliberately do not have.
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/common-keymaps.lisp"),
        ] {
            let ast = Parser::new(&format!("(progn {source})"))
                .next()
                .expect("the shipped lisp must parse");
            eval(&ast, env.clone(), &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn with_text(body: &str, ctx: &Ctx) {
        ctx.with_current_buffer_mut(|buf| {
            buf.text = GapBuffer::from(body);
            buf.text.cursor_move(0, 0);
        });
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|buf| buf.text.to_string())
    }

    /// Press a sequence written the way `define-key` writes one.
    ///
    /// Through `handle_key_event`, which is the whole point: a macro test that
    /// called the commands directly would never see a key the recorder has to
    /// notice before any command runs.
    fn press(keys: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for key in parse(keys) {
            ctx.handle_key_event(key, env);
        }
    }

    fn parse(keys: &str) -> Vec<KeyEvent> {
        keys.split_whitespace()
            .map(|part| match part {
                "<ret>" => KeyEvent::new(KeyCode::Enter),
                "<space>" => KeyEvent::new(KeyCode::Char(' ')),
                other => {
                    let (ctrl, rest) = match other.strip_prefix("C-") {
                        Some(rest) => (true, rest),
                        None => (false, other),
                    };
                    let (alt, rest) = match rest.strip_prefix("M-") {
                        Some(rest) => (true, rest),
                        None => (false, rest),
                    };
                    let mut chars = rest.chars();
                    let ch = chars.next().expect("a key name");
                    assert!(chars.next().is_none(), "{other} is not one key");
                    KeyEvent {
                        code: KeyCode::Char(ch),
                        modifiers: KeyModifiers {
                            ctrl,
                            alt,
                            ..Default::default()
                        },
                    }
                }
            })
            .collect()
    }

    fn recorded(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        match run("(kbd-macro-keys)", env, ctx) {
            LispExp::String(s) => (*s).to_string(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    // ----------------------------------------------------------------
    // Recording
    // ----------------------------------------------------------------

    #[test]
    fn what_was_pressed_is_what_was_recorded() {
        let (ctx, env) = editor();
        with_text("hello", &ctx);
        press("C-x ( C-f C-f C-x )", &env, &ctx);
        assert_eq!(
            recorded(&ctx, &env),
            "C-f C-f",
            "the keys that started and stopped it are not part of it"
        );
    }

    #[test]
    fn recording_does_what_the_keys_do() {
        // Not a rehearsal. The keys are recorded *and* obeyed, which is what
        // makes a macro something you record by doing the thing once.
        let (ctx, env) = editor();
        with_text("hello", &ctx);
        press("C-x ( C-f C-f C-x )", &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            2,
            "point moved while it was being recorded"
        );
    }

    #[test]
    fn an_empty_recording_is_not_kept() {
        let (ctx, env) = editor();
        press("C-x ( C-x )", &env, &ctx);
        assert!(
            run("(kbd-macro-keys)", &env, &ctx).is_nil(),
            "there would be nothing to replay, and `C-x e' doing nothing \
             silently is worse than being told"
        );
    }

    #[test]
    fn starting_a_recording_while_recording_is_refused() {
        let (ctx, env) = editor();
        with_text("hello", &ctx);
        press("C-x ( C-f", &env, &ctx);
        assert!(run("(kmacro-start-macro)", &env, &ctx).is_nil());
        press("C-f C-x )", &env, &ctx);
        assert_eq!(
            recorded(&ctx, &env),
            "C-f C-f",
            "the refusal left the recording that was already running alone"
        );
    }

    #[test]
    fn abandoning_a_recording_keeps_the_previous_one() {
        let (ctx, env) = editor();
        with_text("hello world", &ctx);
        press("C-x ( C-f C-x )", &env, &ctx);
        press("C-x ( C-f C-f", &env, &ctx);
        assert!(!run("(kmacro-cancel-macro)", &env, &ctx).is_nil());
        assert_eq!(recorded(&ctx, &env), "C-f", "the one before it is still there");
    }

    #[test]
    fn a_key_the_prefix_argument_reader_takes_is_recorded() {
        // `C-u` and the digits after it never reach a keymap and run no
        // command, so a recorder watching commands would replay `C-u 3 C-f` as
        // a bare `C-f`.
        let (ctx, env) = editor();
        with_text("abcdefghij", &ctx);
        press("C-x ( C-u 3 C-f C-x )", &env, &ctx);
        assert_eq!(recorded(&ctx, &env), "C-u 3 C-f");
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            3,
            "and it meant three while being recorded"
        );
        run("(goto-char 0)", &env, &ctx);
        run("(kmacro-call-macro)", &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            3,
            "and three again when replayed"
        );
    }

    #[test]
    fn a_character_read_by_a_command_is_recorded() {
        // `M-z x' -- `zap-to-char' arms a key capture and the `x' is taken by
        // it, without a second command running. This is the case that decided
        // keys over commands: a recorder watching `call-interactively' sees
        // `(zap-to-char)' and replays it as a wait for a character that is
        // never coming.
        let (ctx, env) = editor();
        with_text("hello world", &ctx);
        press("C-x ( M-z o C-x )", &env, &ctx);
        assert_eq!(recorded(&ctx, &env), "M-z o");
        // Through the `o', not up to it, which is what `zap-to-char' means.
        assert_eq!(text(&ctx), " world", "and it zapped while recording");

        with_text("hello world", &ctx);
        run("(goto-char 0)", &env, &ctx);
        run("(kmacro-call-macro)", &env, &ctx);
        assert_eq!(text(&ctx), " world", "and zaps again when replayed");
    }

    // ----------------------------------------------------------------
    // Replaying
    // ----------------------------------------------------------------

    #[test]
    fn a_replay_presses_the_keys_again() {
        let (ctx, env) = editor();
        with_text("one\ntwo\nthree\nfour", &ctx);
        press("C-x ( C-a C-k C-k C-x )", &env, &ctx);
        assert_eq!(text(&ctx), "two\nthree\nfour");
        run("(kmacro-call-macro)", &env, &ctx);
        assert_eq!(text(&ctx), "three\nfour");
    }

    #[test]
    fn a_count_repeats_it() {
        let (ctx, env) = editor();
        with_text("one\ntwo\nthree\nfour\nfive", &ctx);
        press("C-x ( C-a C-k C-k C-x )", &env, &ctx);
        assert_eq!(
            run("(kmacro-call-macro 3)", &env, &ctx),
            LispExp::number(3.0)
        );
        assert_eq!(text(&ctx), "five");
    }

    #[test]
    fn calling_with_no_macro_recorded_says_so() {
        let (ctx, env) = editor();
        assert!(run("(kmacro-call-macro)", &env, &ctx).is_nil());
    }

    #[test]
    fn one_key_both_ends_and_calls() {
        // `C-x e' with a recording still running: wanting to run the macro is
        // why you stopped recording, so one key does both.
        let (ctx, env) = editor();
        with_text("one\ntwo\nthree", &ctx);
        press("C-x ( C-a C-k C-k", &env, &ctx);
        assert_eq!(text(&ctx), "two\nthree");
        press("C-x e", &env, &ctx);
        assert_eq!(text(&ctx), "three");
        assert_eq!(recorded(&ctx, &env), "C-a C-k C-k");
    }

    #[test]
    fn replayed_keys_are_not_recorded_again() {
        // A macro that calls a macro records the *call*. Recording what the
        // call expands to would double the body, and double it again at a
        // third level.
        //
        // Through a *named* macro, which is the only sane way to call one from
        // inside another with a single last-macro slot: `C-x e' inside a
        // recording ends that recording, and a key bound to
        // `kmacro-call-macro' would call whatever the last macro is by then,
        // which is the one being recorded.
        let (ctx, env) = editor();
        with_text("one\ntwo\nthree", &ctx);
        run(r#"(define-kbd-macro 'kill-a-line "C-a C-k C-k")"#, &env, &ctx);
        run(r#"(define-key nil "C-c k" 'kill-a-line)"#, &env, &ctx);

        press("C-x ( C-c k C-x )", &env, &ctx);
        assert_eq!(
            recorded(&ctx, &env),
            "C-c k",
            "two keys, not the three the call expanded to"
        );
        assert_eq!(text(&ctx), "two\nthree", "and the call did its work");
    }

    #[test]
    fn a_macro_that_calls_itself_is_refused_rather_than_crashing() {
        // The first version of this test aborted the process. Each level of
        // replay is a real call through the key handler, so a self-calling
        // macro grows the *stack*, and the stack runs out while the execution
        // budget -- which was what the docstring claimed would catch it -- is
        // barely touched.
        let (ctx, env) = editor();
        with_text("abcdefghijklmnop", &ctx);
        // A key that calls the last macro without ending a recording, so the
        // macro can contain a call to itself.
        run(r#"(define-key nil "C-c c" 'kmacro-call-macro)"#, &env, &ctx);
        press("C-x ( C-f C-c c C-x )", &env, &ctx);
        assert_eq!(recorded(&ctx, &env), "C-f C-c c");
        // Returns rather than aborting, and leaves the editor usable.
        run("(kmacro-call-macro)", &env, &ctx);
        with_text("hello", &ctx);
        run("(goto-char 0) (forward-char)", &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            1,
            "the editor still works afterwards"
        );
    }

    #[test]
    fn the_key_that_ends_and_calls_is_not_part_of_the_macro() {
        // `C-x e' ends a recording as much as `C-x )' does, so its keys are as
        // much not part of one. Leaving it out of the exclusion list made a
        // recording ended that way contain only `C-x e', which then replayed
        // itself for ever.
        let (ctx, env) = editor();
        with_text("one\ntwo\nthree", &ctx);
        press("C-x ( C-a C-k C-k", &env, &ctx);
        press("C-x e", &env, &ctx);
        assert_eq!(recorded(&ctx, &env), "C-a C-k C-k");
    }

    // ----------------------------------------------------------------
    // Undo
    // ----------------------------------------------------------------

    #[test]
    fn a_replay_is_one_undo_step() {
        // Nobody who ran a fifty-command macro meant to undo a fiftieth of
        // it. Each repetition is one step, so a run of twenty can still be
        // walked back a repetition at a time.
        let (ctx, env) = editor();
        with_text("one\ntwo\nthree\nfour", &ctx);
        press("C-x ( C-a C-k C-k C-x )", &env, &ctx);
        let after_recording = text(&ctx);
        run("(kmacro-call-macro)", &env, &ctx);
        assert_eq!(text(&ctx), "three\nfour");
        run("(undo)", &env, &ctx);
        assert_eq!(
            text(&ctx),
            after_recording,
            "one undo takes back the whole replay, not one of its kills"
        );
    }

    #[test]
    fn each_repetition_is_its_own_undo_step() {
        let (ctx, env) = editor();
        with_text("one\ntwo\nthree\nfour\nfive", &ctx);
        press("C-x ( C-a C-k C-k C-x )", &env, &ctx);
        run("(kmacro-call-macro 3)", &env, &ctx);
        assert_eq!(text(&ctx), "five");
        run("(undo)", &env, &ctx);
        assert_eq!(text(&ctx), "four\nfive", "one repetition back");
        run("(undo)", &env, &ctx);
        assert_eq!(text(&ctx), "three\nfour\nfive", "and another");
    }

    // ----------------------------------------------------------------
    // Stopping
    // ----------------------------------------------------------------

    #[test]
    fn a_replay_stops_at_the_first_key_that_failed() {
        // Every key after a failure is pressed in a state the recording never
        // saw, which is how a macro does damage rather than merely not
        // working.
        let (ctx, env) = editor();
        with_text("abc", &ctx);
        run(r#"(defcommand boom () nil "Fails." (no-such-function))"#, &env, &ctx);
        run(r#"(define-key nil "C-c b" 'boom)"#, &env, &ctx);
        press("C-x ( C-f C-c b C-f C-x )", &env, &ctx);
        run("(goto-char 0)", &env, &ctx);
        assert_eq!(
            run("(kmacro-call-macro 5)", &env, &ctx),
            LispExp::number(0.0),
            "not one repetition finished"
        );
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            1,
            "it moved once, hit the failure, and stopped before the second move"
        );
    }

    // ----------------------------------------------------------------
    // Naming, and outliving the session
    // ----------------------------------------------------------------

    #[test]
    fn a_macro_prints_in_the_syntax_that_defines_one() {
        let (ctx, env) = editor();
        with_text("one\ntwo", &ctx);
        press("C-x ( C-a C-k C-x )", &env, &ctx);
        let spelling = recorded(&ctx, &env);
        assert_eq!(spelling, "C-a C-k");
        // The round trip: what it prints is what `define-kbd-macro` takes.
        with_text("one\ntwo", &ctx);
        run(
            &format!(r#"(define-kbd-macro 'tidy "{spelling}") (tidy)"#),
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "\ntwo");
    }

    #[test]
    fn a_named_macro_is_an_ordinary_command() {
        let (ctx, env) = editor();
        run(r#"(define-kbd-macro 'tidy-line "C-a C-k")"#, &env, &ctx);
        assert!(
            run("(member \"tidy-line\" (all-commands))", &env, &ctx).is_truthy(),
            "M-x finds it"
        );
        assert!(
            !run("(function-doc 'tidy-line)", &env, &ctx).is_nil(),
            "and describe-function has something to say about it"
        );
        run(r#"(define-key nil "C-c t" 'tidy-line)"#, &env, &ctx);
        with_text("one\ntwo", &ctx);
        press("C-c t", &env, &ctx);
        assert_eq!(text(&ctx), "\ntwo", "and a key runs it");
    }

    #[test]
    fn a_nonsense_spelling_is_refused_where_it_is_written() {
        let (ctx, env) = editor();
        let ast = Parser::new(r#"(define-kbd-macro 'bad "C-x not-a-key")"#)
            .next()
            .expect("parses");
        assert!(
            eval(&ast, env.clone(), &ctx).is_err(),
            "rather than becoming a command that does nothing"
        );
    }

    // ----------------------------------------------------------------
    // The counter
    // ----------------------------------------------------------------

    #[test]
    fn the_counter_numbers_the_repetitions() {
        let (ctx, env) = editor();
        with_text("a\nb\nc\n", &ctx);
        // Pressed, not called: a recording records keys, so a counter inserted
        // by evaluating the primitive would not be in the macro at all.
        press("C-x ( C-x C-k C-i C-n C-a C-x )", &env, &ctx);
        run("(kmacro-call-macro 2)", &env, &ctx);
        assert_eq!(text(&ctx), "0a\n1b\n2c\n");
    }

    #[test]
    fn a_width_pads_with_zeros() {
        // What makes a numbered list sort the way it reads.
        let (ctx, env) = editor();
        with_text("", &ctx);
        run("(kmacro-set-counter 7)", &env, &ctx);
        assert_eq!(
            run("(kmacro-insert-counter 3)", &env, &ctx),
            LispExp::string("007".into())
        );
        assert_eq!(text(&ctx), "007");
    }

    #[test]
    fn starting_a_recording_sets_the_counter_to_zero() {
        let (ctx, env) = editor();
        run("(kmacro-set-counter 42)", &env, &ctx);
        press("C-x (", &env, &ctx);
        assert_eq!(run("(kmacro-counter)", &env, &ctx), LispExp::number(0.0));
    }

    #[test]
    fn the_counter_advances_when_it_is_inserted_not_once_per_repetition() {
        // Which is what you want: a macro that puts the number in twice gets
        // two different numbers.
        let (ctx, env) = editor();
        with_text("", &ctx);
        run("(kmacro-set-counter 0)", &env, &ctx);
        run("(kmacro-insert-counter) (insert \"-\") (kmacro-insert-counter)", &env, &ctx);
        assert_eq!(text(&ctx), "0-1");
    }

    // ----------------------------------------------------------------
    // Saying so
    // ----------------------------------------------------------------

    #[test]
    fn the_editor_says_while_a_recording_is_running() {
        // Recording changes nothing visible on its own -- the keys do what
        // they always did -- so without something to ask, the only way to tell
        // is to remember.
        let (ctx, env) = editor();
        with_text("hello", &ctx);
        assert!(run("(kmacro-recording-p)", &env, &ctx).is_nil());
        press("C-x ( C-f C-f", &env, &ctx);
        assert_eq!(
            run("(kmacro-recording-p)", &env, &ctx),
            LispExp::number(2.0),
            "two keys so far"
        );
        press("C-x )", &env, &ctx);
        assert!(run("(kmacro-recording-p)", &env, &ctx).is_nil());
    }

}
