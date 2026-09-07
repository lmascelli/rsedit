//! Regression tests for defects found by inspection and confirmed by probe.
//!
//! Each one reproduces the failure as it actually presented -- a panic, an
//! abort, a wrong character -- rather than testing the shape of the fix.
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

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env must build")
    }

    /// Comparing two closures used to reach `Env`'s `PartialEq`, which was
    /// `unreachable!()`, so this one-line program panicked and took the editor
    /// with it. Closures now compare by identity.
    #[test]
    fn comparing_two_lambdas_does_not_panic() {
        let (ctx, env) = editor();
        assert!(
            eval_str("(equal (lambda (x) x) (lambda (x) x))", &env, &ctx)
                .expect("comparing lambdas must not panic")
                .is_nil(),
            "two separately written lambdas are not the same closure"
        );
        assert_eq!(
            eval_str("(progn (setq f (lambda (x) x)) (equal f f))", &env, &ctx)
                .expect("comparing a lambda with itself"),
            LispExp::t(),
            "a closure must equal itself"
        );
    }

    /// `LispExp`'s derived `PartialEq` recursed once per cons cell, so
    /// comparing a long list overflowed the stack and *aborted* the process --
    /// not a catchable panic. The comparison is now iterative.
    ///
    /// 60,000 elements is well past where the derived version died.
    const BUILD: &str = "(setq l nil) (setq i 0) \
                         (while (< i 60000) (setq l (cons i l)) (setq i (+ i 1)))";

    #[test]
    fn long_list_equal_self() {
        let (ctx, env) = editor();
        assert_eq!(
            eval_str(&format!("{BUILD} (equal l l)"), &env, &ctx).expect("self comparison"),
            LispExp::t()
        );
    }

    #[test]
    fn long_list_equal_alias() {
        let (ctx, env) = editor();
        assert_eq!(
            eval_str(&format!("{BUILD} (setq m l) (equal l m)"), &env, &ctx).expect("alias"),
            LispExp::t()
        );
    }

    #[test]
    fn long_list_equal_distinct() {
        let (ctx, env) = editor();
        let two = format!(
            "{BUILD} (setq k nil) (setq i 0) \
             (while (< i 60000) (setq k (cons i k)) (setq i (+ i 1))) (equal l k)"
        );
        assert_eq!(
            eval_str(&two, &env, &ctx).expect("two distinct long lists"),
            LispExp::t()
        );
    }

    /// Letting go of a long list used to abort the process.
    ///
    /// `Arc<ConsCell>` dropped its `cdr`, which dropped the next cell, one
    /// stack frame per element -- so a list of a few tens of thousands of
    /// elements overflowed the stack when it went out of scope. It died
    /// between 5,000 and 20,000 elements, which is well inside what ordinary
    /// Lisp builds, and the crash landed far from anything that looked like a
    /// cause. `ConsCell`'s `Drop` now unlinks the chain iteratively.
    ///
    /// Built in Rust rather than through the interpreter so the test measures
    /// the drop and nothing else.
    #[test]
    fn dropping_a_long_list_does_not_overflow_the_stack() {
        for n in [20_000usize, 200_000] {
            let mut list: LispExp<Ctx> = LispExp::nil();
            for i in 0..n {
                list = LispExp::cons(LispExp::number(i as f64), list);
            }
            assert_eq!(list.iter().count(), n);
            drop(list);
        }
    }

    /// A shared tail must survive its head being dropped -- the iterative
    /// unlink has to stop at the first cell it does not solely own.
    #[test]
    fn dropping_a_list_leaves_a_shared_tail_intact() {
        let mut tail: LispExp<Ctx> = LispExp::nil();
        for i in 0..1_000 {
            tail = LispExp::cons(LispExp::number(i as f64), tail);
        }
        let head = LispExp::cons(LispExp::symbol("head".into()), tail.clone());

        drop(head);
        assert_eq!(tail.iter().count(), 1_000, "the shared tail was unlinked");
    }

    /// `at()` used `<=` where it needed `<`, so at the gap it returned whatever
    /// the gap contained -- usually a stale copy left by an earlier
    /// `copy_within`. Reading the buffer back character by character produced
    /// "Xbbc" for the text "Xabc".
    #[test]
    fn reading_a_buffer_character_by_character_matches_its_text() {
        let mut buf = GapBuffer::from("abc");
        buf.move_gap(0);
        buf.insert('X');

        let via_at: String = (0..buf.len())
            .map(|i| buf.at(i).expect("in range"))
            .collect();
        assert_eq!(
            via_at,
            buf.to_string(),
            "at() disagreed with the buffer's text"
        );
        assert_eq!(via_at, "Xabc");

        // And across a mixture of edits and gap positions.
        let mut buf = GapBuffer::from("hello\nworld");
        for target in [0usize, 5, 11, 3, 8] {
            buf.move_gap(target);
            buf.insert('.');
            let via_at: String = (0..buf.len())
                .map(|i| buf.at(i).expect("in range"))
                .collect();
            assert_eq!(
                via_at,
                buf.to_string(),
                "at() disagreed with the gap at {target}"
            );
        }
    }

    /// `nth` walked with an iterator that yields nothing for a syntax `Form`,
    /// so it quietly returned nil where every sibling raises.
    #[test]
    fn nth_rejects_a_syntax_form_like_its_siblings() {
        let (ctx, env) = editor();
        let err = eval_str(
            "(defmacro m (p) (list 'quote (nth 0 p))) (m (a b))",
            &env,
            &ctx,
        )
        .expect_err("nth on a Form must be a type error, not nil");
        assert!(
            matches!(err, EvalError::WrongArgumentType { .. }),
            "expected WrongArgumentType, got {err:?}"
        );
        // Still correct on real lists, and still nil past the end.
        assert_eq!(
            eval_str("(nth 1 '(a b c))", &env, &ctx).expect("nth on a list"),
            LispExp::symbol("b".into())
        );
        assert!(
            eval_str("(nth 9 '(a b c))", &env, &ctx)
                .expect("nth past the end")
                .is_nil()
        );
    }

    /// A body that exhausted the budget left nothing for its cleanups, so the
    /// first step of the first cleanup failed with `OutOfFuel` too and nothing
    /// was unwound -- exactly when unwinding matters most.
    #[test]
    fn unwind_protect_cleanups_run_after_the_body_runs_out_of_fuel() {
        let (ctx, env) = editor();
        eval_str("(setq cleaned nil)", &env, &ctx).expect("setup");
        ctx.set_fuel_budget(50_000);

        let err = eval_str("(unwind-protect (while t 1) (setq cleaned t))", &env, &ctx)
            .expect_err("the runaway body must still fail");
        assert_eq!(err, EvalError::OutOfFuel, "the body's error must survive");
        assert_eq!(
            env.get_variable("cleaned").expect("cleaned must be bound"),
            LispExp::t(),
            "the cleanup must have run despite the budget being spent"
        );
    }

    /// A failing cleanup used to replace the body's error, making a runaway
    /// loop indistinguishable from a broken cleanup. The body's error wins.
    #[test]
    fn a_failing_cleanup_does_not_hide_why_the_body_failed() {
        let (ctx, env) = editor();
        let err = eval_str(
            "(unwind-protect (signal 'my-error nil) (undefined-function-here))",
            &env,
            &ctx,
        )
        .expect_err("the body failed");
        assert!(
            matches!(err, EvalError::Signal { .. }),
            "expected the body's signal to survive, got {err:?}"
        );

        // When only the cleanup fails, its error is the one worth reporting.
        let err = eval_str("(unwind-protect 1 (undefined-function-here))", &env, &ctx)
            .expect_err("the cleanup failed");
        assert!(
            matches!(err, EvalError::UndefinedFunction(_)),
            "expected the cleanup's error, got {err:?}"
        );
    }

    /// Clearing a buffer deleted one character at a time, moving the gap on
    /// every step. 60,000 characters took close to two minutes.
    #[test]
    fn clearing_a_large_buffer_is_immediate() {
        let mut buf = GapBuffer::from("a line of text\n".repeat(20_000).as_str());
        assert!(buf.len() > 250_000);

        let start = std::time::Instant::now();
        buf.clear();
        let elapsed = start.elapsed();

        assert_eq!(buf.len(), 0);
        assert_eq!(buf.to_string(), "");
        assert_eq!(buf.line_count(), 1, "an empty buffer still has one line");
        assert_eq!(buf.cursor_pos(), (0, 0));
        // Generous: the point is that it is not proportional to the text.
        assert!(
            elapsed < std::time::Duration::from_millis(100),
            "clearing took {elapsed:?} -- it is still unwinding character by character"
        );
        // The buffer is usable afterwards.
        buf.insert('x');
        assert_eq!(buf.to_string(), "x");
    }
}
