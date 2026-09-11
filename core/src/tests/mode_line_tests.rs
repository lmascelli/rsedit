//! The mode line, and the pending-input display beside the echo area.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::ui::{Face, RenderableWindowView};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
            b.is_modified = false;
        });
        (ctx, env)
    }

    /// The window showing *scratch*.
    fn view(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> RenderableWindowView {
        ctx.snapshot(env, W, H)
            .views
            .into_iter()
            .find(|v| v.buffer_name == "*scratch*")
            .expect("*scratch* must be on screen")
    }

    fn mode_line(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        view(ctx, env).mode_line.expect("a tiled window has one")
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode, modifiers: KeyModifiers) {
        ctx.handle_key_event(KeyEvent { code, modifiers }, env);
    }

    fn ctrl() -> KeyModifiers {
        KeyModifiers {
            ctrl: true,
            ..Default::default()
        }
    }

    // ---------------- what it says ----------------

    #[test]
    fn the_default_format_names_the_buffer_its_mode_and_where_point_is() {
        let (ctx, env) = editor_with("alpha\nbeta\ngamma");
        eval_str("(next-line) (forward-char 2)", &env, &ctx).expect("move");

        let line = mode_line(&ctx, &env);
        assert!(line.contains("*scratch*"), "the buffer: {line:?}");
        assert!(line.contains("fundamental"), "the mode: {line:?}");
        assert!(line.contains("L2"), "the line, counting from one: {line:?}");
        assert!(line.contains("C2"), "the column: {line:?}");
    }

    #[test]
    fn the_modified_flag_follows_the_buffer() {
        let (ctx, env) = editor_with("alpha");
        assert!(mode_line(&ctx, &env).contains("--"), "unmodified");

        eval_str("(self-insert \"x\")", &env, &ctx).expect("edit");
        assert!(
            mode_line(&ctx, &env).contains("**"),
            "an edit should show: {:?}",
            mode_line(&ctx, &env)
        );
    }

    #[test]
    fn every_escape_expands() {
        let (ctx, env) = editor_with("one\ntwo\nthree\nfour");
        eval_str("(setq mode-line-format \"%b|%m|%l|%c|%*|%%\")", &env, &ctx).expect("setq");
        eval_str("(next-line) (forward-char 1)", &env, &ctx).expect("move");

        assert_eq!(mode_line(&ctx, &env), "*scratch*|fundamental|2|1|--|%");
    }

    /// An unknown escape is left as written. A format that silently swallowed
    /// the bit you got wrong would be harder to fix than one that shows it.
    #[test]
    fn an_unknown_escape_is_left_alone() {
        let (ctx, env) = editor_with("alpha");
        eval_str("(setq mode-line-format \"a%qb\")", &env, &ctx).expect("setq");
        assert_eq!(mode_line(&ctx, &env), "a%qb");
    }

    #[test]
    fn the_position_reads_as_emacs_words_it() {
        let (ctx, env) = editor_with("alpha");
        eval_str("(setq mode-line-format \"%p\")", &env, &ctx).expect("setq");
        assert_eq!(mode_line(&ctx, &env), "All", "one line is all of it");

        let (ctx, env) = editor_with("a\nb\nc\nd\ne");
        eval_str("(setq mode-line-format \"%p\")", &env, &ctx).expect("setq");
        assert_eq!(mode_line(&ctx, &env), "Top");
        eval_str("(end-of-buffer)", &env, &ctx).expect("to the end");
        assert_eq!(mode_line(&ctx, &env), "Bot");
        eval_str("(beginning-of-buffer) (next-line) (next-line)", &env, &ctx).expect("middle");
        assert_eq!(mode_line(&ctx, &env), "50%");
    }

    /// A format that is not a string falls back rather than blanking every
    /// status line in the editor.
    #[test]
    fn a_format_that_is_not_a_string_falls_back() {
        let (ctx, env) = editor_with("alpha");
        eval_str("(setq mode-line-format 42)", &env, &ctx).expect("setq");
        assert!(
            mode_line(&ctx, &env).contains("*scratch*"),
            "the default should be used: {:?}",
            mode_line(&ctx, &env)
        );
    }

    // ---------------- where it goes ----------------

    /// The status line takes a row from the window, and the text has to be
    /// laid out against what is left -- otherwise the last line of the buffer
    /// is drawn under the status line and never seen.
    #[test]
    fn the_status_line_takes_a_row_from_the_text() {
        let (ctx, env) = editor_with("alpha");
        let view = view(&ctx, &env);

        assert!(view.mode_line.is_some());
        assert!(
            view.rect.height <= H - 2,
            "the window rect should give up a row to the status line and \
             another to the echo area, got {} of {H}",
            view.rect.height
        );
        assert!(
            view.lines.len() <= view.rect.height,
            "no more text rows than the rect holds"
        );
    }

    /// A window with one row keeps it for text: a status line with nothing
    /// under it says nothing worth the row.
    #[test]
    fn a_window_too_short_for_one_does_without() {
        let (ctx, env) = editor_with("alpha");
        // Two rows of frame: one for the echo area, one for the window.
        let frame = ctx.snapshot(&env, W, 2);
        let view = frame
            .views
            .into_iter()
            .find(|v| v.buffer_name == "*scratch*")
            .expect("*scratch* must be on screen");
        assert_eq!(view.mode_line, None);
        assert_eq!(view.rect.height, 1, "the row goes to the text");
    }

    /// A floating window says what it is on its border, so a status line would
    /// be a second answer to the same question.
    #[test]
    fn a_floating_window_has_no_status_line() {
        let (ctx, env) = editor_with("alpha");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        eval_str("(minibuffer-read \"Find file:\" nil nil nil)", &env, &ctx).expect("prompt");

        let float = ctx
            .snapshot(&env, W, H)
            .views
            .into_iter()
            .find(|v| v.buffer_name == "*Minibuffer*")
            .expect("the minibuffer must be on screen");
        assert_eq!(float.mode_line, None);
        assert!(
            float.title.is_some(),
            "it is labelled on its border instead"
        );
    }

    #[test]
    fn the_focused_window_uses_the_active_face() {
        let (ctx, env) = editor_with("alpha");
        assert!(view(&ctx, &env).is_focused);
        let theme = ctx.snapshot(&env, W, H).theme;
        assert_ne!(
            theme.style(Face::ModeLine),
            theme.style(Face::ModeLineInactive),
            "an unfocused window's status line has to look different, or the \
             two faces are one face with two names"
        );
        assert!(
            theme.style(Face::ModeLine).reverse,
            "the default has to be visible against a background the editor \
             cannot know"
        );
    }

    // ---------------- pending input beside the echo area ----------------

    /// The argument and the sequence appear together, because they compose:
    /// showing only one would misreport what the next key does.
    #[test]
    fn a_prefix_argument_and_a_key_sequence_are_shown_together() {
        let (ctx, env) = editor_with("alpha");
        eval_str(
            "(define-key nil \"C-x m\" 'beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("bind");

        press(&ctx, &env, KeyCode::Char('u'), ctrl());
        press(&ctx, &env, KeyCode::Char('4'), KeyModifiers::default());
        assert_eq!(ctx.snapshot(&env, W, H).pending_input, "C-u 4-");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        assert_eq!(
            ctx.snapshot(&env, W, H).pending_input,
            "C-u 4 C-x-",
            "both halves, in the order they were typed"
        );
    }

    /// Unlike a message, this does not expire -- which is the whole reason it
    /// is not one.
    #[test]
    fn pending_input_outlives_the_echo_timeout() {
        let (ctx, env) = editor_with("alpha");
        eval_str("(setq echo-message-timeout 0.01)", &env, &ctx).expect("a short timeout");
        eval_str(
            "(define-key nil \"C-x m\" 'beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("bind");
        ctx.set_echo_message("a message");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        std::thread::sleep(std::time::Duration::from_millis(120));

        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_message, "", "the message expired");
        assert_eq!(
            frame.pending_input, "C-x-",
            "but the editor is still waiting for a key, and still says so"
        );
    }
}
