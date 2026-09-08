//! Undo and redo: what gets recorded, how it is grouped, and the symmetry
//! between undoing and redoing.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
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

    /// Type a character the way the editor does, through the key handler, so
    /// that the grouping decision in `handle_key_event` is exercised rather
    /// than bypassed.
    fn type_char(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::default(),
            },
            env,
        );
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

    fn alt() -> KeyModifiers {
        KeyModifiers {
            alt: true,
            ..Default::default()
        }
    }

    // ---------------- the basic round trip ----------------

    #[test]
    fn undo_restores_deleted_text_and_redo_removes_it_again() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str("(kill-word)", &env, &ctx).expect("kill");
        assert_eq!(text_of(&ctx), " beta");

        assert_eq!(
            eval_str("(undo)", &env, &ctx).expect("undo"),
            LispExp::symbol("t".into()),
            "undo should report that it did something"
        );
        assert_eq!(
            text_of(&ctx),
            "alpha beta",
            "the killed word should be back"
        );

        eval_str("(redo)", &env, &ctx).expect("redo");
        assert_eq!(text_of(&ctx), " beta", "redo should kill it again");
    }

    #[test]
    fn undo_removes_inserted_text_and_redo_puts_it_back() {
        let (ctx, env) = editor_with("");
        eval_str("(self-insert \"a\") (self-insert \"b\")", &env, &ctx).expect("insert");
        assert_eq!(text_of(&ctx), "ab");

        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(text_of(&ctx), "", "both inserts were one group");

        eval_str("(redo)", &env, &ctx).expect("redo");
        assert_eq!(
            text_of(&ctx),
            "ab",
            "redoing an insertion must restore the exact text, which is only \
             possible because undo captured it on the way out"
        );
    }

    #[test]
    fn undo_and_redo_report_nil_when_there_is_nothing_to_do() {
        let (ctx, env) = editor_with("text");
        assert_eq!(
            eval_str("(undo)", &env, &ctx).expect("undo"),
            LispExp::nil(),
            "a buffer with no history has nothing to undo"
        );
        assert_eq!(
            eval_str("(redo)", &env, &ctx).expect("redo"),
            LispExp::nil()
        );
        assert_eq!(text_of(&ctx), "text", "and nothing should have changed");
    }

    /// The interesting case for a change-based history: undoing a group has to
    /// take its changes apart in the opposite order to the one they went
    /// together in, because each is described against the text the previous
    /// one produced.
    #[test]
    fn a_group_of_several_changes_undoes_in_reverse_order() {
        let (ctx, env) = editor_with("one two three");
        // One Lisp form, so no boundary falls between them: three deletions in
        // a single group, each at a different offset.
        eval_str(
            "(end-of-buffer) (backward-kill-word) (backward-kill-word) (beginning-of-buffer) (kill-word)",
            &env,
            &ctx,
        )
        .expect("kills");
        assert_eq!(text_of(&ctx), " ");

        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(
            text_of(&ctx),
            "one two three",
            "reversing the group must reproduce the original text exactly"
        );
    }

    /// A forward kill is the case that tells the two candidate answers apart:
    /// re-inserting the text naturally leaves point at its *end*, whereas the
    /// place the user was standing when they killed it is its *beginning*. It
    /// is the second that undo has to restore.
    #[test]
    fn undo_moves_point_back_to_where_the_change_was_made() {
        let (ctx, env) = editor_with("alpha beta gamma");
        eval_str("(kill-word)", &env, &ctx).expect("kill");
        assert_eq!(text_of(&ctx), " beta gamma");

        eval_str("(end-of-buffer)", &env, &ctx).expect("wander off");
        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(text_of(&ctx), "alpha beta gamma");
        assert_eq!(
            point_1d(&ctx),
            0,
            "undo should bring point back to where it stood before the edit, \
             not leave it at the far end of the buffer where it had drifted \
             to, nor at the end of the text it just restored"
        );
    }

    // ---------------- grouping ----------------

    #[test]
    fn a_run_of_typing_undoes_as_one_group() {
        let (ctx, env) = editor_with("");
        for c in "hello".chars() {
            type_char(&ctx, &env, c);
        }
        assert_eq!(text_of(&ctx), "hello");

        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(
            text_of(&ctx),
            "",
            "five keystrokes in a row should undo in one step, not five"
        );
    }

    #[test]
    fn a_long_run_of_typing_breaks_into_groups() {
        let (ctx, env) = editor_with("");
        let typed: String = std::iter::repeat_n('x', 25).collect();
        for c in typed.chars() {
            type_char(&ctx, &env, c);
        }
        assert_eq!(text_of(&ctx), typed);

        eval_str("(undo)", &env, &ctx).expect("undo");
        let left = text_of(&ctx).len();
        assert!(
            left > 0 && left < 25,
            "a 25-character run should undo in more than one step but not \
             one step per character; {left} characters left"
        );
    }

    #[test]
    fn a_command_between_two_runs_of_typing_separates_them() {
        let (ctx, env) = editor_with("");
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");

        for c in "ab".chars() {
            type_char(&ctx, &env, c);
        }
        // C-b is not an edit, but it is a different command, so the run of
        // typing before it is finished.
        press(&ctx, &env, KeyCode::Char('b'), ctrl());
        for c in "cd".chars() {
            type_char(&ctx, &env, c);
        }
        assert_eq!(text_of(&ctx), "acdb");

        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(text_of(&ctx), "ab", "the second run should undo on its own");
        eval_str("(undo)", &env, &ctx).expect("undo again");
        assert_eq!(text_of(&ctx), "", "and then the first");
    }

    /// A run of typing is stored as one insertion rather than one per
    /// character, which is only sound while the characters are adjacent. Two
    /// insertions at different places in the same command must stay separate
    /// changes, or undo would delete a span that was never inserted.
    #[test]
    fn insertions_at_different_places_in_one_command_stay_separate() {
        let (ctx, env) = editor_with("xy");
        eval_str(
            "(self-insert \"a\") (end-of-buffer) (self-insert \"b\")",
            &env,
            &ctx,
        )
        .expect("two insertions, one group");
        assert_eq!(text_of(&ctx), "axyb");

        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(
            text_of(&ctx),
            "xy",
            "undo must remove exactly what was inserted, not the span between \
             the two insertions"
        );
    }

    #[test]
    fn undo_boundary_splits_a_single_lisp_form_into_steps() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(self-insert \"a\") (undo-boundary) (self-insert \"b\")",
            &env,
            &ctx,
        )
        .expect("insert with a boundary");
        assert_eq!(text_of(&ctx), "ab");

        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(
            text_of(&ctx),
            "a",
            "the explicit boundary should keep the two inserts apart"
        );
    }

    // ---------------- the redo branch ----------------

    #[test]
    fn a_new_edit_discards_what_could_have_been_redone() {
        let (ctx, env) = editor_with("");
        eval_str("(self-insert \"a\")", &env, &ctx).expect("insert a");
        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(text_of(&ctx), "");

        eval_str("(self-insert \"z\")", &env, &ctx).expect("insert z");
        assert_eq!(
            eval_str("(redo)", &env, &ctx).expect("redo"),
            LispExp::nil(),
            "editing after an undo should abandon the redo branch"
        );
        assert_eq!(text_of(&ctx), "z", "and must not resurrect the old text");
    }

    /// A redone change becomes undoable again -- otherwise redo would be a
    /// one-way door and the history could only ever be walked in one
    /// direction after the first redo.
    #[test]
    fn a_redone_change_can_be_undone_again() {
        let (ctx, env) = editor_with("");
        eval_str("(self-insert \"a\")", &env, &ctx).expect("insert");
        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(text_of(&ctx), "");
        eval_str("(redo)", &env, &ctx).expect("redo");
        assert_eq!(text_of(&ctx), "a");

        assert_eq!(
            eval_str("(undo)", &env, &ctx).expect("undo again"),
            LispExp::symbol("t".into()),
            "the redone change should be back on the undo stack"
        );
        assert_eq!(text_of(&ctx), "");
    }

    // ---------------- the history limit ----------------

    /// The one part of the history that grows with the document rather than
    /// with the number of edits is the text held by deletions, so that is what
    /// the limit bounds -- and the most recent change survives it, so a single
    /// deletion larger than the whole limit is still undoable.
    #[test]
    fn the_limit_drops_old_history_but_keeps_the_most_recent_change() {
        let (ctx, env) = editor_with("alpha beta gamma");
        eval_str("(set-undo-limit 0)", &env, &ctx).expect("set the limit");

        eval_str("(kill-word) (undo-boundary)", &env, &ctx).expect("first kill");
        eval_str("(kill-word) (undo-boundary)", &env, &ctx).expect("second kill");
        let after = text_of(&ctx);

        eval_str("(undo)", &env, &ctx).expect("undo the most recent");
        assert_ne!(
            text_of(&ctx),
            after,
            "the most recent change must survive any limit"
        );
        assert_eq!(
            eval_str("(undo)", &env, &ctx).expect("undo again"),
            LispExp::nil(),
            "the older change should have been dropped to stay under the limit"
        );
    }

    #[test]
    fn undo_and_redo_can_be_repeated_to_walk_the_history() {
        let (ctx, env) = editor_with("");
        for c in ["a", "b", "c"] {
            eval_str(&format!("(self-insert \"{c}\")"), &env, &ctx).expect("insert");
            eval_str("(undo-boundary)", &env, &ctx).expect("boundary");
        }
        let full = text_of(&ctx);

        eval_str("(undo) (undo) (undo)", &env, &ctx).expect("undo everything");
        assert_eq!(text_of(&ctx), "", "three undos should empty the buffer");

        eval_str("(redo) (redo) (redo)", &env, &ctx).expect("redo everything");
        assert_eq!(
            text_of(&ctx),
            full,
            "walking back up the history should reproduce the text exactly"
        );
    }

    // ---------------- coverage of the editing layer ----------------

    /// Every editing primitive is undoable because there is no way into the
    /// text except the recording layer. This asserts that for each of them,
    /// so that a new command added on the old pattern -- straight to
    /// `buf.text` -- is caught here rather than noticed by a user.
    #[test]
    fn every_editing_command_is_undoable() {
        let cases: &[(&str, &str)] = &[
            ("(self-insert \"z\")", "self-insert"),
            ("(insert-newline)", "insert-newline"),
            (
                "(forward-char) (delete-backward-char)",
                "delete-backward-char",
            ),
            ("(delete-char)", "delete-char"),
            ("(kill-line)", "kill-line"),
            ("(kill-whole-line)", "kill-whole-line"),
            ("(kill-word)", "kill-word"),
            ("(end-of-buffer) (backward-kill-word)", "backward-kill-word"),
            ("(kill-paragraph)", "kill-paragraph"),
            (
                "(end-of-buffer) (backward-kill-paragraph)",
                "backward-kill-paragraph",
            ),
            ("(clear-buffer)", "clear-buffer"),
        ];

        for (src, name) in cases {
            let original = "alpha beta\ngamma delta\n\nsecond paragraph\n";
            let (ctx, env) = editor_with(original);
            eval_str(src, &env, &ctx).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let after = text_of(&ctx);
            assert_ne!(after, original, "{name} should have changed the text");

            eval_str("(undo)", &env, &ctx).unwrap_or_else(|e| panic!("{name} undo: {e:?}"));
            assert_eq!(text_of(&ctx), original, "{name} should be undoable");

            eval_str("(redo)", &env, &ctx).unwrap_or_else(|e| panic!("{name} redo: {e:?}"));
            assert_eq!(text_of(&ctx), after, "{name} should be redoable");
        }
    }

    // ---------------- bindings ----------------

    #[test]
    fn the_undo_keys_are_bound() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");

        eval_str("(kill-word)", &env, &ctx).expect("kill");
        assert_eq!(text_of(&ctx), " beta");

        // C-_ is what a terminal actually reports for C-/, so it is the one
        // worth pressing here.
        press(&ctx, &env, KeyCode::Char('_'), ctrl());
        assert_eq!(text_of(&ctx), "alpha beta", "C-_ should undo");

        press(&ctx, &env, KeyCode::Char('_'), alt());
        assert_eq!(text_of(&ctx), " beta", "M-_ should redo");
    }

    /// Replaying history must not be recorded as new history: if it were, the
    /// first undo would push an entry that the second undo would then undo,
    /// and undo would flip between two states forever.
    #[test]
    fn undoing_twice_walks_back_two_steps_rather_than_oscillating() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(self-insert \"a\") (undo-boundary) (self-insert \"b\")",
            &env,
            &ctx,
        )
        .expect("two groups");
        assert_eq!(text_of(&ctx), "ab");

        eval_str("(undo)", &env, &ctx).expect("first undo");
        assert_eq!(text_of(&ctx), "a");
        eval_str("(undo)", &env, &ctx).expect("second undo");
        assert_eq!(
            text_of(&ctx),
            "",
            "the second undo must undo the earlier edit, not re-apply the first undo"
        );
    }
}
