//! `electric-pair.lisp`, and the `post-self-insert-hook` it hangs off.
//!
//! The hook is the interesting part. It has to fire for a typed character and
//! *not* for a paste -- otherwise pasted code gets every bracket in it doubled,
//! which is the failure bracketed paste exists to prevent. Several of these
//! tests are about that distinction rather than about pairing.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
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

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/electric-pair.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.get_current_buffer()
            .read()
            .expect("read lock")
            .text
            .to_string()
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.get_current_buffer()
            .read()
            .expect("read lock")
            .text
            .cursor_pos_1d()
    }

    /// Type a character the way a keystroke does: insert it, then run the hook.
    fn type_char(c: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        // Escaped, because one of the characters worth typing here is the
        // quote that would otherwise end the Lisp string it is written in.
        let escaped = c.replace('\\', "\\\\").replace('"', "\\\"");
        run(&format!(r#"(self-insert "{escaped}")"#), env, ctx);
        run("(electric-pair-post-self-insert)", env, ctx);
    }

    #[test]
    fn an_opening_bracket_brings_its_partner() {
        let (ctx, env) = editor();
        type_char("(", &env, &ctx);
        assert_eq!(contents(&ctx), "()");
        // Between them, not after them: the whole point is that you carry on
        // typing the contents.
        assert_eq!(point(&ctx), 1);
    }

    #[test]
    fn every_bracket_the_default_table_pairs_is_paired() {
        for (open, closed) in [("(", "()"), ("[", "[]"), ("{", "{}")] {
            let (ctx, env) = editor();
            type_char(open, &env, &ctx);
            assert_eq!(contents(&ctx), closed, "typing {open}");
        }
    }

    #[test]
    fn the_pairs_come_from_the_modes_own_table_not_from_a_list_here() {
        let (ctx, env) = editor();
        // A mode that pairs something no default table does. If pairing read a
        // list of its own this would not pair at all.
        run(
            r#"(make-mode 'shouty-mode)
               (set-syntax-pairs 'shouty-mode "<>")
               (add-hook 'shouty-mode "post-self-insert-hook"
                         'electric-pair-post-self-insert)
               (buffer-create "shout" 'shouty-mode)
               (switch-to-buffer "shout")"#,
            &env,
            &ctx,
        );
        type_char("<", &env, &ctx);
        assert_eq!(contents(&ctx), "<>");
    }

    #[test]
    fn a_closing_bracket_steps_over_the_one_that_is_there() {
        let (ctx, env) = editor();
        type_char("(", &env, &ctx);
        type_char(")", &env, &ctx);
        // One pair, not `())`. Point past it, ready to keep going.
        assert_eq!(contents(&ctx), "()");
        assert_eq!(point(&ctx), 2);
    }

    #[test]
    fn skipping_can_be_turned_off_on_its_own() {
        let (ctx, env) = editor();
        run("(setq electric-pair-skip-self nil)", &env, &ctx);
        type_char("(", &env, &ctx);
        type_char(")", &env, &ctx);
        // The closer typed is the closer inserted, so the auto-inserted one is
        // still waiting outside it.
        assert_eq!(contents(&ctx), "())");
    }

    #[test]
    fn a_closing_bracket_with_nothing_to_skip_is_just_typed() {
        let (ctx, env) = editor();
        type_char(")", &env, &ctx);
        assert_eq!(contents(&ctx), ")");
        assert_eq!(point(&ctx), 1);
    }

    #[test]
    fn nothing_is_paired_in_front_of_a_word() {
        let (ctx, env) = editor();
        run(r#"(insert "foo") (goto-char 0)"#, &env, &ctx);
        type_char("(", &env, &ctx);
        // `(foo`, not `()foo`: typing a bracket before a word means wrapping
        // it, and a closer in between is never what was meant.
        assert_eq!(contents(&ctx), "(foo");
    }

    #[test]
    fn nothing_is_paired_inside_a_string() {
        let (ctx, env) = editor();
        run(r#"(insert "\"abc\"") (goto-char 2)"#, &env, &ctx);
        type_char("(", &env, &ctx);
        assert_eq!(contents(&ctx), "\"a(bc\"");
    }

    #[test]
    fn a_quote_pairs_and_the_state_read_is_the_one_before_it() {
        let (ctx, env) = editor();
        type_char("\"", &env, &ctx);
        // The subtle one. After the quote is typed, point *is* inside a string
        // -- the one this very quote opened. Reading the state at point would
        // report "in a string" and refuse to pair. It has to be read at the
        // position before the character.
        assert_eq!(contents(&ctx), "\"\"");
        assert_eq!(point(&ctx), 1);
    }

    #[test]
    fn a_quote_closes_the_one_it_is_sitting_on() {
        let (ctx, env) = editor();
        type_char("\"", &env, &ctx);
        type_char("\"", &env, &ctx);
        assert_eq!(contents(&ctx), "\"\"");
        assert_eq!(point(&ctx), 2);
    }

    #[test]
    fn a_mode_can_refuse_quote_pairing_without_refusing_brackets() {
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'lifetimes-mode)
               (put 'lifetimes-mode 'electric-pair-inhibit-quotes t)
               (add-hook 'lifetimes-mode "post-self-insert-hook"
                         'electric-pair-post-self-insert)
               (buffer-create "lt" 'lifetimes-mode)
               (switch-to-buffer "lt")"#,
            &env,
            &ctx,
        );
        type_char("\"", &env, &ctx);
        assert_eq!(contents(&ctx), "\"", "a quote is left alone");
        run(r#"(clear-buffer)"#, &env, &ctx);
        type_char("(", &env, &ctx);
        assert_eq!(contents(&ctx), "()", "brackets still pair");
    }

    #[test]
    fn backspace_takes_the_partner_of_an_empty_pair() {
        let (ctx, env) = editor();
        type_char("(", &env, &ctx);
        run("(electric-pair-delete-backward)", &env, &ctx);
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn backspace_leaves_a_pair_with_something_in_it_alone() {
        let (ctx, env) = editor();
        run(r#"(insert "(ab)") (goto-char 1)"#, &env, &ctx);
        run("(electric-pair-delete-backward)", &env, &ctx);
        // The `(` goes, the `)` stays: deleting a bracket from around text is
        // something people do deliberately.
        assert_eq!(contents(&ctx), "ab)");
    }

    #[test]
    fn forward_delete_takes_the_partner_too() {
        // The mirror of backspace: point before the `(` of an empty pair.
        let (ctx, env) = editor();
        run(r#"(insert "()") (goto-char 0)"#, &env, &ctx);
        run("(electric-pair-delete-forward)", &env, &ctx);
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn forward_delete_from_inside_the_pair_takes_both() {
        let (ctx, env) = editor();
        run(r#"(insert "()") (goto-char 1)"#, &env, &ctx);
        run("(electric-pair-delete-forward)", &env, &ctx);
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn backspace_from_after_the_closer_takes_both() {
        let (ctx, env) = editor();
        run(r#"(insert "()") (goto-char 2)"#, &env, &ctx);
        run("(electric-pair-delete-backward)", &env, &ctx);
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn a_pair_with_only_blanks_in_it_counts_as_empty() {
        for at in [1, 2, 4] {
            let (ctx, env) = editor();
            run(
                &format!(r#"(insert "a(   )b") (goto-char {})"#, at + 1),
                &env,
                &ctx,
            );
            run("(electric-pair-delete-backward)", &env, &ctx);
            assert_eq!(contents(&ctx), "ab", "backspace at {at}");
        }
    }

    #[test]
    fn the_blanks_inside_go_with_the_pair() {
        let (ctx, env) = editor();
        run(r#"(insert "(  )") (goto-char 2)"#, &env, &ctx);
        run("(electric-pair-delete-forward)", &env, &ctx);
        // Not `(  ` or ` )` -- the whole span, so nothing is left to tidy up.
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn a_newline_inside_does_not_make_a_pair_empty() {
        // `{` and `}` on separate lines is a block about to get a body, and
        // collapsing it on one backspace would be startling.
        let (ctx, env) = editor();
        run("(insert \"{\n}\") (goto-char 2)", &env, &ctx);
        run("(electric-pair-delete-backward)", &env, &ctx);
        assert_eq!(contents(&ctx), "{}");
    }

    #[test]
    fn a_pair_with_something_in_it_survives_both_directions() {
        let (ctx, env) = editor();
        run(r#"(insert "(ab)") (goto-char 1)"#, &env, &ctx);
        run("(electric-pair-delete-forward)", &env, &ctx);
        assert_eq!(contents(&ctx), "(b)", "forward delete is ordinary");
    }

    #[test]
    fn forward_delete_at_the_end_of_the_buffer_does_nothing() {
        let (ctx, env) = editor();
        run(r#"(insert "ab") (goto-char 2)"#, &env, &ctx);
        run("(electric-pair-delete-forward)", &env, &ctx);
        assert_eq!(contents(&ctx), "ab");
    }

    // ----------------------------------------------------------------
    // Not unbalancing what is already balanced
    // ----------------------------------------------------------------

    /// A buffer in a mode that pairs braces, with point at OFFSET.
    fn braces_buffer(text: &str, at: usize, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            r#"(make-mode 'braces-mode)
               (set-syntax-pairs 'braces-mode "(){}[]")
               (add-hook 'braces-mode "post-self-insert-hook"
                         'electric-pair-post-self-insert)
               (buffer-create "code" 'braces-mode)
               (switch-to-buffer "code")"#,
            env,
            ctx,
        );
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        run(
            &format!(r#"(insert "{escaped}") (goto-char {at})"#),
            env,
            ctx,
        );
    }

    #[test]
    fn retyping_a_deleted_opener_does_not_add_a_second_closer() {
        // The reported case. Delete the `{` from a function and type it back:
        // the `}` below is still there, so a new one would leave the brace
        // doubled -- and the delete rules would then treat `{}` as an empty
        // pair and take both out together.
        let (ctx, env) = editor();
        braces_buffer("fn a() \n  body\n}", 7, &env, &ctx);
        type_char("{", &env, &ctx);
        assert_eq!(contents(&ctx), "fn a() {\n  body\n}");
    }

    #[test]
    fn an_opener_with_no_closer_ahead_still_pairs() {
        // The other half: the check must not stop pairing working at all.
        let (ctx, env) = editor();
        braces_buffer("fn a() \n  body\n", 7, &env, &ctx);
        type_char("{", &env, &ctx);
        assert_eq!(contents(&ctx), "fn a() {}\n  body\n");
    }

    #[test]
    fn text_after_point_without_a_closer_does_not_count_as_closed() {
        // `up-list` reports an unterminated list as ending at the end of the
        // buffer, so "it moved" is not enough -- what settles it is the
        // character it landed after.
        let (ctx, env) = editor();
        braces_buffer("fn a() \n  body", 7, &env, &ctx);
        type_char("{", &env, &ctx);
        assert_eq!(contents(&ctx), "fn a() {}\n  body");
    }

    #[test]
    fn a_closer_typed_where_the_block_is_already_closed_goes_to_the_end_of_it() {
        // Typing `}` a line above an existing one means "finish this block",
        // and the block is finished.
        let (ctx, env) = editor();
        braces_buffer("{\n  body\n}", 8, &env, &ctx);
        type_char("}", &env, &ctx);
        assert_eq!(contents(&ctx), "{\n  body\n}", "no second closer");
        assert_eq!(point(&ctx), 10, "and point is past the one that was there");
    }

    #[test]
    fn a_closer_with_nothing_closing_the_block_is_typed_normally() {
        let (ctx, env) = editor();
        braces_buffer("{\n  body\n", 9, &env, &ctx);
        type_char("}", &env, &ctx);
        assert_eq!(contents(&ctx), "{\n  body\n}");
    }

    #[test]
    fn the_repaired_brace_can_then_be_deleted_on_its_own() {
        // The second half of the complaint: once a stray `{}` existed, the
        // delete rules saw an empty pair and took both out. With no stray pair
        // made, backspace over the `{` removes just it.
        let (ctx, env) = editor();
        braces_buffer("fn a() \n  body\n}", 7, &env, &ctx);
        type_char("{", &env, &ctx);
        run("(electric-pair-delete-backward)", &env, &ctx);
        assert_eq!(contents(&ctx), "fn a() \n  body\n}");
    }

    #[test]
    fn nested_blocks_still_pair_inside_an_outer_one() {
        // The check asks about the list point is *in*, so an inner brace with
        // no closer of its own still pairs even though the outer one is
        // closed.
        let (ctx, env) = editor();
        braces_buffer("{\n  \n}", 4, &env, &ctx);
        type_char("{", &env, &ctx);
        assert_eq!(contents(&ctx), "{\n  {}\n}");
    }

    #[test]
    fn turning_the_mode_off_stops_all_of_it() {
        let (ctx, env) = editor();
        run("(setq electric-pair-mode nil)", &env, &ctx);
        type_char("(", &env, &ctx);
        assert_eq!(contents(&ctx), "(");
        run(r#"(clear-buffer) (insert "()") (goto-char 1)"#, &env, &ctx);
        run("(electric-pair-delete-backward)", &env, &ctx);
        assert_eq!(contents(&ctx), ")", "backspace is ordinary again");
        run(r#"(clear-buffer) (insert "()") (goto-char 0)"#, &env, &ctx);
        run("(electric-pair-delete-forward)", &env, &ctx);
        assert_eq!(contents(&ctx), ")", "and so is forward delete");
    }
}
