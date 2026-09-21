//! The scan checkpoints: that they are a shortcut and never an answer.
//!
//! # What these are really testing
//!
//! One property, from several directions: **a scan that resumes from a
//! checkpoint reaches the same answer as a scan that started at the top.** A
//! cache over a stateful lexer is worth having only if that holds, and it is
//! the kind of thing that holds for every case anybody thought of and fails on
//! a string that happens to span a checkpoint boundary. So the equivalence is
//! asserted over a whole file at many positions rather than at a chosen few.
//!
//! The rest is the bookkeeping that keeps it true: an edit drops the
//! checkpoints below it and keeps the ones above, and a turn computed against
//! text or a mode that has since moved on is refused rather than stored.
#[cfg(test)]
mod tests {
    use crate::buffer::BufferTrait;
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::buffer::scan::{LINES_PER_CHECKPOINT, ScanCache, checkpoint_line};
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::modes::sexp;
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
            include_str!("../../lisp/indent.lisp"),
            include_str!("../../lisp/rust-mode.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A rust-mode buffer holding `source`, set without going through `insert`.
    fn rust_buffer(name: &str, source: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(r#"(buffer-create "{name}" 'rust-mode) (switch-to-buffer "{name}")"#),
            env,
            ctx,
        );
        let handle = ctx.get_current_buffer();
        let mut buf = handle.write().expect("write lock");
        buf.text = GapBuffer::from(source);
        buf.text.cursor_move(0, 0);
        buf.version += 1;
    }

    /// Run prescan turns until the buffer is fully checkpointed.
    ///
    /// Bounded, because a loop that never ends is how a test reports a bug as a
    /// hang. The background worker is running too and may do some of this; both
    /// go through `record`, which refuses anything out of order, so they cannot
    /// between them write a checkpoint neither computed.
    fn warm(ctx: &Ctx, name: &str) {
        for _ in 0..1000 {
            let Some(turn) = ctx.prescan_turn_for(name) else {
                return;
            };
            let Some(scanned) = ctx.run_prescan(&turn) else {
                return;
            };
            ctx.store_prescan(&turn, scanned);
        }
        panic!("{name} never finished prescanning");
    }

    /// A file long enough to hold several checkpoints, with every construct
    /// that makes the scan stateful crossing lines: a string with brackets in
    /// it, a block comment, a raw string, a character literal.
    fn source() -> String {
        let mut out = String::new();
        out.push_str("fn main() {\n");
        for n in 0..40 {
            out.push_str(&format!(
                "    // block {n}\n    if a[{n}] {{\n        let s = \"a ( b {n}\";\n        let c = '\\'';\n        f(s, c);\n    }}\n"
            ));
            if n % 7 == 0 {
                out.push_str(&format!(
                    "    /* a comment\n       over {n} lines\n       with ) in it */\n"
                ));
            }
            if n % 11 == 0 {
                out.push_str(&format!(
                    "    let raw = r#\"unbalanced ( and \" quotes {n}\n       still inside\n    \"#;\n"
                ));
            }
        }
        out.push_str("}\n");
        out
    }

    // ----------------------------------------------------------------
    // The property everything else exists to keep
    // ----------------------------------------------------------------

    #[test]
    fn a_resumed_scan_answers_exactly_what_a_scan_from_the_top_answers() {
        let (ctx, env) = editor();
        let text = source();
        rust_buffer("big", &text, &env, &ctx);
        warm(&ctx, "big");

        let table = ctx.syntax_table("rust-mode");
        let handle = ctx.get_buffer("big").expect("the buffer is there");
        let buf = handle.read().expect("read lock");
        let mut resumed = 0;
        // A stride that is not a factor of anything, so the positions tried are
        // not all at the same place in a line or a construct.
        for pos in (0..buf.text.len()).step_by(37) {
            let shortcut = buf.scan_resume(pos);
            if shortcut.is_some() {
                resumed += 1;
            }
            assert_eq!(
                sexp::context_at(&buf.text, &table, shortcut, pos),
                sexp::context_at(&buf.text, &table, None, pos),
                "the context at {pos} must not depend on where the scan started"
            );
            assert_eq!(
                sexp::enclosing(&buf.text, &table, shortcut, pos),
                sexp::enclosing(&buf.text, &table, None, pos),
                "the enclosing list at {pos} must not depend on where the scan started"
            );
            assert_eq!(
                sexp::forward(&buf.text, &table, shortcut, pos, 1),
                sexp::forward(&buf.text, &table, None, pos, 1),
                "forward-sexp from {pos} must not depend on where the scan started"
            );
        }
        assert!(
            resumed > 0,
            "the test proves nothing unless some of those scans actually resumed"
        );
    }

    #[test]
    fn a_checkpoint_taken_inside_a_string_resumes_inside_it() {
        // The case a cache over a stateful lexer gets wrong: the state at a
        // line boundary is only interesting when it is not the base state.
        let (ctx, env) = editor();
        let mut text = String::from("fn main() {\n");
        // A string long enough that a checkpoint lands in the middle of it.
        text.push_str("    let s = \"");
        for n in 0..(LINES_PER_CHECKPOINT * 2) {
            text.push_str(&format!("line {n} with ) and }} inside\n"));
        }
        text.push_str("\";\n}\n");
        rust_buffer("str", &text, &env, &ctx);
        warm(&ctx, "str");

        let table = ctx.syntax_table("rust-mode");
        let handle = ctx.get_buffer("str").expect("the buffer is there");
        let buf = handle.read().expect("read lock");
        let inside = buf.text.cursor_2d_to_1d(LINES_PER_CHECKPOINT + 3, 2);
        let found = sexp::context_at(&buf.text, &table, buf.scan_resume(inside), inside);
        assert!(
            found.string_start.is_some(),
            "a position deep inside the string is inside the string"
        );
        assert_eq!(
            found,
            sexp::context_at(&buf.text, &table, None, inside),
            "and the resumed answer is the answer"
        );
        assert_eq!(
            sexp::context_at(
                &buf.text,
                &table,
                buf.scan_resume(buf.text.len()),
                buf.text.len()
            ),
            sexp::context_at(&buf.text, &table, None, buf.text.len()),
            "and the file still balances the same way at the end"
        );
    }

    #[test]
    fn the_checkpoints_actually_move_the_start_of_the_scan() {
        // Without this the equivalence test above could pass by never using a
        // checkpoint at all.
        let (ctx, env) = editor();
        rust_buffer("moved", &source(), &env, &ctx);
        warm(&ctx, "moved");

        let handle = ctx.get_buffer("moved").expect("the buffer is there");
        let buf = handle.read().expect("read lock");
        let end = buf.text.len();
        let resume = buf.scan_resume(end).expect("a warm cache has a checkpoint");
        assert!(
            resume.offset() > 0,
            "a scan asked about the end of the file starts well down it"
        );
        let line = buf.text.cursor_1d_to_2d(resume.offset()).0;
        assert!(
            buf.text.line_count() - line <= LINES_PER_CHECKPOINT + 1,
            "and within one checkpoint interval of the question"
        );
    }

    // ----------------------------------------------------------------
    // What an edit does to it
    // ----------------------------------------------------------------

    #[test]
    fn an_edit_keeps_the_checkpoints_above_it_and_drops_the_rest() {
        let (ctx, env) = editor();
        rust_buffer("edited", &source(), &env, &ctx);
        warm(&ctx, "edited");

        let handle = ctx.get_buffer("edited").expect("the buffer is there");
        let before = handle.read().expect("read lock").scan.checkpoints();
        assert!(
            before >= 3,
            "the fixture needs several checkpoints, got {before}"
        );

        // An edit three checkpoints down.
        let at = {
            let buf = handle.read().expect("read lock");
            buf.text.cursor_2d_to_1d(checkpoint_line(2) + 1, 0)
        };
        run(&format!("(goto-char {at}) (self-insert \"x\")"), &env, &ctx);

        let buf = handle.read().expect("read lock");
        assert_eq!(
            buf.scan.checkpoints(),
            3,
            "the three checkpoints at or above the edited line survive it"
        );
        assert!(
            buf.scan.checkpoints() < before,
            "and the ones below it did not"
        );
    }

    #[test]
    fn an_edit_does_not_leave_a_stale_answer_behind() {
        // The bug this whole arrangement could have: typing a `(` and being
        // told the buffer still balances, because the answer came from before
        // the keystroke.
        let (ctx, env) = editor();
        rust_buffer("stale", &source(), &env, &ctx);
        warm(&ctx, "stale");

        let table = ctx.syntax_table("rust-mode");
        let handle = ctx.get_buffer("stale").expect("the buffer is there");
        let depth_before = {
            let buf = handle.read().expect("read lock");
            let end = buf.text.len();
            sexp::context_at(&buf.text, &table, buf.scan_resume(end), end).depth
        };
        assert_eq!(depth_before, 0, "the fixture balances to begin with");

        // An opener near the top, above every checkpoint but the first.
        let at = {
            let buf = handle.read().expect("read lock");
            buf.text.cursor_2d_to_1d(2, 0)
        };
        run(&format!("(goto-char {at}) (self-insert \"(\")"), &env, &ctx);

        let buf = handle.read().expect("read lock");
        let end = buf.text.len();
        assert_eq!(
            sexp::context_at(&buf.text, &table, buf.scan_resume(end), end).depth,
            1,
            "the new opener is seen, whatever the cache had worked out before it"
        );
    }

    // ----------------------------------------------------------------
    // What the worker refuses to store
    // ----------------------------------------------------------------

    #[test]
    fn a_turn_whose_text_moved_on_is_thrown_away() {
        let (ctx, env) = editor();
        rust_buffer("moved-on", &source(), &env, &ctx);
        let turn = ctx
            .prescan_turn_for("moved-on")
            .expect("a cold buffer has work");
        let scanned = ctx.run_prescan(&turn).expect("the turn runs");
        assert!(
            !scanned.is_empty(),
            "the fixture is long enough to checkpoint"
        );

        // The text changes between the scanning and the storing -- which is the
        // only moment the rule is about, and why the two are separate calls.
        run("(goto-char 0) (self-insert \"(\")", &env, &ctx);
        ctx.store_prescan(&turn, scanned);

        let handle = ctx.get_buffer("moved-on").expect("the buffer is there");
        assert_eq!(
            handle.read().expect("read lock").scan.checkpoints(),
            0,
            "nothing computed from the old text was kept"
        );
    }

    /// Put a buffer into another mode, the way nothing else can yet.
    ///
    /// A buffer's mode is fixed when it is created, so there is no command for
    /// this and no Lisp that reaches it. The guard exists anyway, because the
    /// day there is one the failure would not look like a mode bug: it would
    /// look like `forward-sexp` occasionally landing somewhere strange in one
    /// buffer, from a checkpoint taken under the table of a language the text
    /// is no longer being read as.
    fn remode(ctx: &Ctx, name: &str, mode: &str) {
        let handle = ctx.get_buffer(name).expect("the buffer is there");
        handle.write().expect("write lock").current_mode = mode.to_string();
    }

    #[test]
    fn a_turn_whose_mode_moved_on_is_thrown_away() {
        // A checkpoint is a state reached under one syntax table. Under another
        // it is not a shortcut but a wrong answer with a plausible shape.
        let (ctx, env) = editor();
        run("(make-mode 'other-mode)", &env, &ctx);
        rust_buffer("remoded", &source(), &env, &ctx);
        let turn = ctx
            .prescan_turn_for("remoded")
            .expect("a cold buffer has work");
        let scanned = ctx.run_prescan(&turn).expect("the turn runs");
        assert!(
            !scanned.is_empty(),
            "the fixture is long enough to checkpoint"
        );

        remode(&ctx, "remoded", "other-mode");
        ctx.store_prescan(&turn, scanned);

        let handle = ctx.get_buffer("remoded").expect("the buffer is there");
        assert_eq!(
            handle.read().expect("read lock").scan.checkpoints(),
            0,
            "nothing computed under the old table was kept"
        );
    }

    #[test]
    fn a_cache_from_another_mode_is_not_offered_as_a_shortcut() {
        let (ctx, env) = editor();
        run("(make-mode 'plain-mode)", &env, &ctx);
        rust_buffer("switched", &source(), &env, &ctx);
        warm(&ctx, "switched");
        assert!(
            {
                let handle = ctx.get_buffer("switched").expect("the buffer is there");
                let buf = handle.read().expect("read lock");
                buf.scan_resume(buf.text.len()).is_some()
            },
            "there is a shortcut to lose in the first place"
        );

        remode(&ctx, "switched", "plain-mode");
        let handle = ctx.get_buffer("switched").expect("the buffer is there");
        let buf = handle.read().expect("read lock");
        assert!(
            buf.scan_resume(buf.text.len()).is_none(),
            "the mode changed, so what was worked out under the old one is refused"
        );
    }

    // ----------------------------------------------------------------
    // The cache's own arithmetic
    // ----------------------------------------------------------------

    #[test]
    fn invalidating_keeps_the_checkpoint_entering_the_edited_line() {
        // The asymmetry worth a test of its own: a checkpoint *entering* line L
        // was computed from the text strictly above L, which an edit on L did
        // not touch.
        let (ctx, env) = editor();
        rust_buffer("exact", &source(), &env, &ctx);
        warm(&ctx, "exact");
        let handle = ctx.get_buffer("exact").expect("the buffer is there");
        let at = {
            let buf = handle.read().expect("read lock");
            assert!(buf.scan.checkpoints() >= 2);
            buf.text.cursor_2d_to_1d(checkpoint_line(1), 0)
        };
        run(&format!("(goto-char {at}) (self-insert \"x\")"), &env, &ctx);
        assert_eq!(
            handle.read().expect("read lock").scan.checkpoints(),
            2,
            "the checkpoint entering the edited line is one of the survivors"
        );
    }

    #[test]
    fn a_checkpoint_is_refused_out_of_order() {
        // Each carries on from the one before it, so one written out of order
        // would be a state reached from somewhere nobody can name.
        let mut cache = ScanCache::default();
        cache.reset(1, "rust-mode");
        let text = GapBuffer::from("(a (b c))");
        let table = crate::modes::SyntaxTable::default();
        let scan = sexp::Scan::<GapBuffer>::begin(&text, &table, 0, None, 1);
        assert!(
            cache.record(1, scan.snapshot()).is_err(),
            "index 1 before 0"
        );
        assert!(cache.record(0, scan.snapshot()).is_ok());
        assert_eq!(cache.checkpoints(), 1);
    }

    #[test]
    fn a_cache_from_another_version_offers_nothing() {
        let mut cache = ScanCache::default();
        cache.reset(1, "rust-mode");
        let text = GapBuffer::from("(a (b c))");
        let table = crate::modes::SyntaxTable::default();
        let scan = sexp::Scan::<GapBuffer>::begin(&text, &table, 0, None, 1);
        cache
            .record(0, scan.snapshot())
            .expect("the first checkpoint");
        assert!(cache.resume_for("rust-mode", 1, 1000).is_some());
        assert!(
            cache.resume_for("rust-mode", 2, 1000).is_none(),
            "another version describes other text"
        );
        assert!(
            cache.resume_for("other-mode", 1, 1000).is_none(),
            "another mode reads it with another table"
        );
    }

    #[test]
    fn a_checkpoint_beyond_the_question_is_ignored() {
        // The guard that makes a stale or ill-fitting snapshot cost a full scan
        // rather than a wrong answer.
        let text = GapBuffer::from("(a (b c)) (d)");
        let table = crate::modes::SyntaxTable::default();
        let mut ahead = sexp::Scan::<GapBuffer>::begin(&text, &table, 0, None, 1);
        ahead.run_to(11);
        let snapshot = ahead.snapshot();
        assert!(snapshot.offset() >= 11);
        // Asked about position 3, offered a snapshot from position 11.
        let scan = sexp::Scan::<GapBuffer>::begin(&text, &table, 3, Some(&snapshot), 1);
        assert_eq!(
            scan.offset(),
            0,
            "a snapshot from beyond the question is refused, and the scan starts at the top"
        );
    }
}
