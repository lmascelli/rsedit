//! The prefix argument, and the arguments the editor answers itself.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::commands::{ArgSpec, PrefixArg};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");
        // The minibuffer sizes itself against the frame, so a test that opens
        // a prompt needs one.
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn text_of(ctx: &Ctx) -> String {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .to_string()
    }

    fn point_1d(ctx: &Ctx) -> usize {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .cursor_pos_1d()
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

    fn plain() -> KeyModifiers {
        KeyModifiers::default()
    }

    /// Type a run of keys: `C-` prefixes the next character, everything else
    /// is plain. Keeps a five-keystroke sequence readable as one line.
    fn keys(ctx: &Ctx, env: &Arc<Env<Ctx>>, spec: &str) {
        let mut chars = spec.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '^' {
                let next = chars.next().expect("^ must be followed by a key");
                press(ctx, env, KeyCode::Char(next), ctrl());
            } else if c != ' ' {
                press(ctx, env, KeyCode::Char(c), plain());
            }
        }
    }

    // ---------------- building the argument ----------------

    #[test]
    fn c_u_alone_means_four() {
        let (ctx, env) = editor_with("");
        keys(&ctx, &env, "^u");
        assert_eq!(ctx.prefix_argument(), Some(PrefixArg::Raw(1)));
        assert_eq!(ctx.prefix_argument().unwrap().count(), 4);
    }

    #[test]
    fn each_c_u_multiplies_by_four() {
        let (ctx, env) = editor_with("");
        keys(&ctx, &env, "^u^u");
        assert_eq!(ctx.prefix_argument().unwrap().count(), 16);
        keys(&ctx, &env, "^u");
        assert_eq!(ctx.prefix_argument().unwrap().count(), 64);
    }

    #[test]
    fn digits_after_c_u_replace_the_four_rather_than_extending_it() {
        let (ctx, env) = editor_with("");
        keys(&ctx, &env, "^u12");
        assert_eq!(
            ctx.prefix_argument(),
            Some(PrefixArg::Number(12)),
            "C-u 1 2 is twelve, not four then twelve"
        );
    }

    #[test]
    fn a_minus_before_the_digits_makes_the_argument_negative() {
        let (ctx, env) = editor_with("");
        keys(&ctx, &env, "^u-");
        assert_eq!(ctx.prefix_argument().unwrap().count(), -1, "a bare minus");

        let (ctx, env) = editor_with("");
        keys(&ctx, &env, "^u-42");
        assert_eq!(
            ctx.prefix_argument(),
            Some(PrefixArg::Number(-42)),
            "the digits should build a negative number, not minus one then 42"
        );
    }

    /// The obvious bug, and invisible until someone types into a buffer: a
    /// digit with no argument in progress is just a digit.
    #[test]
    fn a_digit_with_no_argument_in_progress_is_typed() {
        let (ctx, env) = editor_with("");
        keys(&ctx, &env, "42");
        assert_eq!(text_of(&ctx), "42");
        assert_eq!(ctx.prefix_argument(), None);
    }

    /// Nothing reaches the buffer while the argument is being built, and the
    /// user is told what they have so far.
    #[test]
    fn building_an_argument_types_nothing_and_is_shown() {
        let (ctx, env) = editor_with("");
        keys(&ctx, &env, "^u4");
        assert_eq!(text_of(&ctx), "", "C-u 4 inserts no text");
        assert_eq!(ctx.get_echo_message(), "C-u 4-");
    }

    // ---------------- spending it ----------------

    #[test]
    fn a_count_repeats_the_command_it_precedes() {
        let (ctx, env) = editor_with("alpha beta gamma");
        keys(&ctx, &env, "^u4^f");
        assert_eq!(point_1d(&ctx), 4, "C-u 4 C-f moves four characters");
    }

    #[test]
    fn without_an_argument_a_counted_command_does_it_once() {
        let (ctx, env) = editor_with("alpha beta");
        keys(&ctx, &env, "^f");
        assert_eq!(point_1d(&ctx), 1);
    }

    #[test]
    fn the_argument_belongs_to_exactly_one_command() {
        let (ctx, env) = editor_with("alpha beta gamma");
        keys(&ctx, &env, "^u4^f");
        assert_eq!(point_1d(&ctx), 4);
        assert_eq!(ctx.prefix_argument(), None, "spent");

        keys(&ctx, &env, "^f");
        assert_eq!(point_1d(&ctx), 5, "the next command moves one, not four");
    }

    /// A command that fails must still consume its argument, or the next one
    /// silently inherits it.
    #[test]
    fn a_failing_command_still_consumes_the_argument() {
        let (ctx, env) = editor_with("alpha");
        eval_str("(define-key nil \"C-t\" 'no-such-command)", &env, &ctx).expect("bind");

        keys(&ctx, &env, "^u4^t");
        assert_eq!(ctx.prefix_argument(), None);
    }

    /// A negative count is currently *floored to zero* by `repeat_count`, so
    /// `C-u - C-f` moves nowhere rather than moving backwards as it would in
    /// Emacs. Pinned here so the limitation is a decision on the record rather
    /// than something nobody noticed: reversing direction on a negative count
    /// means teaching each command what its opposite is, which is a change to
    /// the commands and not to this machinery.
    #[test]
    fn a_negative_count_currently_does_nothing() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str("(forward-char 3)", &env, &ctx).expect("move in a bit");
        assert_eq!(point_1d(&ctx), 3);

        keys(&ctx, &env, "^u-^f");
        assert_eq!(
            point_1d(&ctx),
            3,
            "a negative count is floored to zero, so nothing moves"
        );
        assert_eq!(ctx.prefix_argument(), None, "and it is still consumed");
    }

    /// A prefix argument survives a prefix *key*: the two are independent
    /// pieces of state, which is the reason they are not one enum.
    #[test]
    fn an_argument_survives_a_key_sequence() {
        let (ctx, env) = editor_with("alpha beta gamma");
        eval_str("(define-key nil \"C-x f\" 'forward-char)", &env, &ctx).expect("bind");

        keys(&ctx, &env, "^u4^xf");
        assert_eq!(
            point_1d(&ctx),
            4,
            "C-u 4 C-x f should move four: the argument outlives the sequence"
        );
    }

    /// And a digit inside a key sequence belongs to the sequence, not to the
    /// argument -- the reader stops taking digits as soon as a key it does not
    /// want goes past it.
    #[test]
    fn a_digit_inside_a_key_sequence_is_part_of_the_sequence() {
        let (ctx, env) = editor_with("alpha beta gamma");
        eval_str("(define-key nil \"C-x 4\" 'end-of-buffer)", &env, &ctx).expect("bind");

        keys(&ctx, &env, "^u2^x4");
        assert_eq!(point_1d(&ctx), 16, "C-x 4 ran, so the 4 was not the count");
    }

    // ---------------- p and P ----------------

    #[test]
    fn p_gives_a_count_and_capital_p_gives_what_the_user_asked_for() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(defun show-p (n) (set-echo-message (format \"%s\" n))) \
             (register-command \"show-p\" '(\"p\")) \
             (defun show-raw (n) (set-echo-message (format \"%s\" n))) \
             (register-command \"show-raw\" '(\"P\")) \
             (define-key nil \"C-t\" 'show-p) \
             (define-key nil \"C-y\" 'show-raw)",
            &env,
            &ctx,
        )
        .expect("define two commands");

        keys(&ctx, &env, "^t");
        assert_eq!(ctx.get_echo_message(), "1", "p defaults to one");
        keys(&ctx, &env, "^y");
        assert_eq!(
            ctx.get_echo_message(),
            "nil",
            "P says the user asked for nothing"
        );

        keys(&ctx, &env, "^u^t");
        assert_eq!(ctx.get_echo_message(), "4", "p turns a bare C-u into four");
        keys(&ctx, &env, "^u^y");
        assert_eq!(
            ctx.get_echo_message(),
            "(4)",
            "P keeps a bare C-u distinguishable from the number four"
        );
        keys(&ctx, &env, "^u4^y");
        assert_eq!(ctx.get_echo_message(), "4", "a typed number is a number");
    }

    /// An answered argument opens no prompt at all, which is what makes
    /// `C-u 4 C-f` one keystroke's worth of work rather than a dialogue.
    #[test]
    fn an_answered_argument_never_opens_a_minibuffer() {
        let (ctx, env) = editor_with("alpha beta");
        keys(&ctx, &env, "^u4^f");
        assert!(
            ctx.get_buffer("*Minibuffer*").is_none(),
            "nothing should have been asked"
        );
    }

    /// Answered and prompted arguments interleave in one chain, in the order
    /// the command declared them.
    #[test]
    fn answered_and_prompted_arguments_arrive_in_order() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(defun both (n s) (set-echo-message (format \"%s/%s\" n s))) \
             (register-command \"both\" '(\"p\" \"sWord: \"))",
            &env,
            &ctx,
        )
        .expect("define");

        eval_str("(call-interactively \"both\")", &env, &ctx).expect("start it");
        assert!(
            ctx.get_buffer("*Minibuffer*").is_some(),
            "the string argument still has to be asked for"
        );
        for c in "hi".chars() {
            press(&ctx, &env, KeyCode::Char(c), plain());
        }
        press(&ctx, &env, KeyCode::Enter, plain());
        assert_eq!(ctx.get_echo_message(), "1/hi");
    }

    // ---------------- r ----------------

    #[test]
    fn the_region_spec_hands_a_command_both_ends() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(
            "(defun show-r (a b) (set-echo-message (format \"%s-%s\" a b))) \
             (register-command \"show-r\" '(\"r\")) \
             (set-mark) (forward-word) (call-interactively \"show-r\")",
            &env,
            &ctx,
        )
        .expect("run it");
        assert_eq!(ctx.get_echo_message(), "0-5");
    }

    #[test]
    fn a_region_command_refuses_before_prompting_when_there_is_no_region() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(
            "(defun eat (a b s) nil) (register-command \"eat\" '(\"r\" \"sWith: \"))",
            &env,
            &ctx,
        )
        .expect("define");

        assert!(
            eval_str("(call-interactively \"eat\")", &env, &ctx).is_err(),
            "with no region the command should be refused"
        );
        assert!(
            ctx.get_buffer("*Minibuffer*").is_none(),
            "and refused before asking a question whose answer would be wasted"
        );
    }

    // ---------------- specs ----------------

    #[test]
    fn the_answered_specs_take_no_prompt() {
        for code in ["p", "P", "r"] {
            assert!(ArgSpec::parse(code).is_ok(), "{code} should parse");
            assert!(
                !ArgSpec::parse(code).unwrap().prompts(),
                "{code} must not prompt"
            );
            assert!(
                ArgSpec::parse(&format!("{code}Some prompt: ")).is_err(),
                "{code} with a prompt is a mistake worth reporting"
            );
        }
        assert!(ArgSpec::parse("sFind: ").unwrap().prompts());
    }

    // ---------------- C-g ----------------

    #[test]
    fn keyboard_quit_abandons_a_half_typed_sequence() {
        let (ctx, env) = editor_with("");
        keys(&ctx, &env, "^x");
        assert_eq!(ctx.get_echo_message(), "C-x-");

        keys(&ctx, &env, "^g");
        // The sequence is gone, so the next key starts afresh.
        keys(&ctx, &env, "a");
        assert_eq!(text_of(&ctx), "a");
    }

    #[test]
    fn keyboard_quit_abandons_a_half_built_argument() {
        let (ctx, env) = editor_with("alpha beta");
        keys(&ctx, &env, "^u4");
        keys(&ctx, &env, "^g");
        assert_eq!(ctx.prefix_argument(), None);

        keys(&ctx, &env, "^f");
        assert_eq!(point_1d(&ctx), 1, "the abandoned four is not spent here");
    }

    #[test]
    fn keyboard_quit_ends_the_region() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str("(set-mark) (forward-word)", &env, &ctx).expect("select");
        keys(&ctx, &env, "^g");
        assert_eq!(
            eval_str("(use-region-p)", &env, &ctx).expect("use-region-p"),
            LispExp::nil()
        );
    }
}
