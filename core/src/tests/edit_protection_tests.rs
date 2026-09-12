//! `insert`, and the read-only flag the two editing doors enforce.
//!
//! These sit apart from `dired_tests` on purpose. A read-only buffer is not a
//! dired feature -- it is a property of buffers that any view can ask for --
//! and the tests that say so should not have to open a directory listing to do
//! it.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn setup() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn text(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run("(buffer-string)", env, ctx) {
            LispExp::String(s) => (*s).clone(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    // ---------------- insert ----------------

    #[test]
    fn insert_puts_a_whole_string_in_at_point() {
        let (ctx, env) = setup();
        run(r#"(insert "hello")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "hello");
    }

    #[test]
    fn insert_takes_several_arguments_in_order() {
        let (ctx, env) = setup();
        run(r#"(insert "a" "b" "c")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "abc");
    }

    /// A number is formatted rather than refused, so a listing can put a count
    /// in a line without converting it first -- and it is formatted the way
    /// `format`'s `%s` would, because two conversions would let the echo area
    /// and a buffer disagree about the same number.
    #[test]
    fn insert_formats_what_is_not_a_string_the_way_format_does() {
        let (ctx, env) = setup();
        run(r#"(insert "n=" 42 " " nil)"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "n=42 nil");
    }

    #[test]
    fn insert_can_put_in_a_newline() {
        let (ctx, env) = setup();
        run(r#"(insert "a\nb")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "a\nb");
    }

    /// The reason `insert` exists rather than a loop over `self-insert`: one
    /// call is one edit, so it is one undo step however long the text is.
    #[test]
    fn a_whole_insert_undoes_in_one_step() {
        let (ctx, env) = setup();
        run(r#"(insert "a long line of text")"#, &env, &ctx);
        run("(undo)", &env, &ctx);
        assert_eq!(text(&env, &ctx), "");
    }

    #[test]
    fn several_arguments_are_still_one_undo_step() {
        let (ctx, env) = setup();
        run(r#"(insert "one" "two" "three")"#, &env, &ctx);
        run("(undo)", &env, &ctx);
        assert_eq!(text(&env, &ctx), "");
    }

    #[test]
    fn insert_at_point_goes_where_point_is_not_at_the_end() {
        let (ctx, env) = setup();
        run(r#"(insert "ac")"#, &env, &ctx);
        run("(goto-char 1)", &env, &ctx);
        run(r#"(insert "b")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "abc");
    }

    #[test]
    fn inserting_nothing_is_not_an_error() {
        let (ctx, env) = setup();
        run(r#"(insert "")"#, &env, &ctx);
        run("(insert)", &env, &ctx);
        assert_eq!(text(&env, &ctx), "");
    }

    // ---------------- read-only ----------------

    #[test]
    fn a_buffer_is_writable_until_it_is_told_otherwise() {
        let (ctx, env) = setup();
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::nil());
        run("(set-buffer-read-only t)", &env, &ctx);
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::t());
        run("(set-buffer-read-only nil)", &env, &ctx);
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::nil());
    }

    #[test]
    fn a_read_only_buffer_refuses_an_insert_and_says_so() {
        let (ctx, env) = setup();
        run(r#"(insert "kept")"#, &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);

        run(r#"(insert "added")"#, &env, &ctx);

        assert_eq!(text(&env, &ctx), "kept");
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
    }

    /// Every door, not just the one a test happened to pick: the flag is
    /// checked in `insert_text` and `delete_range`, and every editing command
    /// in the editor goes through one of them.
    #[test]
    fn a_read_only_buffer_refuses_every_editing_command() {
        let (ctx, env) = setup();
        run(r#"(insert "first line\nsecond line")"#, &env, &ctx);
        run("(beginning-of-buffer)", &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);
        let before = text(&env, &ctx);

        for command in [
            r#"(self-insert "x")"#,
            "(insert-newline)",
            "(delete-char)",
            "(kill-line)",
            "(kill-whole-line)",
            "(kill-word)",
            "(kill-paragraph)",
            "(clear-buffer)",
            "(end-of-buffer)(delete-backward-char)",
        ] {
            run(command, &env, &ctx);
            assert_eq!(
                text(&env, &ctx),
                before,
                "{command} should have changed nothing"
            );
        }
    }

    /// Reading is the point of a read-only buffer, so moving around one has to
    /// work -- the flag is on the text, not on the cursor.
    #[test]
    fn point_moves_freely_in_a_read_only_buffer() {
        let (ctx, env) = setup();
        run(r#"(insert "first\nsecond\nthird")"#, &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);

        run("(beginning-of-buffer)", &env, &ctx);
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(1.0)
        );
        run("(next-line)(next-line)", &env, &ctx);
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(3.0)
        );
        run("(end-of-line)", &env, &ctx);
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("third".into())
        );
    }

    /// A kill that reported the text but left it in the buffer would put a
    /// copy on the ring, and the next `yank` into a writable buffer would
    /// duplicate text that was never cut.
    #[test]
    fn a_refused_kill_puts_nothing_on_the_kill_ring() {
        let (ctx, env) = setup();
        run(r#"(insert "protected")"#, &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);
        let before = run("(kill-ring-length)", &env, &ctx);

        run("(beginning-of-buffer)(kill-line)", &env, &ctx);

        assert_eq!(run("(kill-ring-length)", &env, &ctx), before);
        assert_eq!(text(&env, &ctx), "protected");
    }

    #[test]
    fn a_refused_region_kill_puts_nothing_on_the_kill_ring() {
        let (ctx, env) = setup();
        run(r#"(insert "protected")"#, &env, &ctx);
        run("(beginning-of-buffer)(set-mark)(end-of-buffer)", &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);
        let before = run("(kill-ring-length)", &env, &ctx);

        run("(kill-region)", &env, &ctx);

        assert_eq!(run("(kill-ring-length)", &env, &ctx), before);
        assert_eq!(text(&env, &ctx), "protected");
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
    }

    /// A yank into a read-only buffer must not claim to have happened: the
    /// yank position it would record is where the *next* `yank-pop` replaces
    /// text, and recording one for text that was never inserted would have
    /// `M-y` delete something else.
    #[test]
    fn a_refused_yank_reports_and_records_nothing() {
        let (ctx, env) = setup();
        run(r#"(kill-new "from the ring")"#, &env, &ctx);
        run(r#"(insert "protected")"#, &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);

        run("(yank)", &env, &ctx);

        assert_eq!(text(&env, &ctx), "protected");
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
        // `yank-pop` has nothing to replace, so it refuses rather than
        // deleting whatever happens to be at the recorded position.
        assert!(eval_str("(yank-pop)", &env, &ctx).is_err());
    }

    /// The flag is per buffer, not per editor: a protected view and an
    /// ordinary file are open at the same time, and typing has to work in one
    /// of them.
    #[test]
    fn read_only_is_a_property_of_one_buffer() {
        let (ctx, env) = setup();
        run("(set-buffer-read-only t)", &env, &ctx);
        run(
            r#"(buffer-create "writable")(switch-to-buffer "writable")"#,
            &env,
            &ctx,
        );

        run(r#"(insert "typed")"#, &env, &ctx);

        assert_eq!(text(&env, &ctx), "typed");
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::nil());
        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::t());
    }

    // ---------------- buffer-create's mode ----------------

    #[test]
    fn a_buffer_can_be_created_in_a_named_mode() {
        let (ctx, env) = setup();
        run("(make-mode 'view-mode)", &env, &ctx);
        run(r#"(buffer-create "*view*" 'view-mode)"#, &env, &ctx);

        let buffer = ctx.get_buffer("*view*").expect("the buffer");
        assert_eq!(buffer.read().expect("read").current_mode, "view-mode");
    }

    #[test]
    fn a_buffer_created_without_a_mode_is_in_fundamental() {
        let (ctx, env) = setup();
        run(r#"(buffer-create "*plain*")"#, &env, &ctx);

        let buffer = ctx.get_buffer("*plain*").expect("the buffer");
        assert_eq!(buffer.read().expect("read").current_mode, "fundamental");
    }

    /// Re-creating a buffer is how a view is refreshed, and a refresh that
    /// reset the mode would take the keymap away from the very buffer being
    /// refreshed.
    #[test]
    fn an_existing_buffer_keeps_the_mode_it_has() {
        let (ctx, env) = setup();
        run("(make-mode 'view-mode)(make-mode 'other-mode)", &env, &ctx);
        run(r#"(buffer-create "*view*" 'view-mode)"#, &env, &ctx);

        run(r#"(buffer-create "*view*" 'other-mode)"#, &env, &ctx);

        let buffer = ctx.get_buffer("*view*").expect("the buffer");
        assert_eq!(buffer.read().expect("read").current_mode, "view-mode");
    }

    // ---------------- reading a line ----------------

    #[test]
    fn the_current_line_is_the_one_point_is_on_without_its_newline() {
        let (ctx, env) = setup();
        run(r#"(insert "first\nsecond\nthird")"#, &env, &ctx);
        run("(beginning-of-buffer)(next-line)", &env, &ctx);

        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("second".into())
        );
    }

    #[test]
    fn the_current_line_of_an_empty_buffer_is_empty() {
        let (ctx, env) = setup();
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("".into())
        );
    }

    /// The numbering matches `goto-line`'s, which is what makes "remember
    /// where I was, rebuild, go back" a two-line idiom rather than an
    /// off-by-one hunt.
    #[test]
    fn the_line_number_counts_from_one_and_round_trips_through_goto_line() {
        let (ctx, env) = setup();
        run(r#"(insert "a\nb\nc\nd")"#, &env, &ctx);
        run("(beginning-of-buffer)", &env, &ctx);
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(1.0)
        );

        run("(goto-line 3)", &env, &ctx);
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(3.0)
        );
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("c".into())
        );
    }
}
