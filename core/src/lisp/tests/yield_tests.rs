//! `(yield)`: stopping in the middle of a form, and coming back to it.
//!
//! # What is worth testing here
//!
//! A suspension is a claim that the program will carry on *exactly* where it
//! stopped, and every way of getting that subtly wrong still looks like it
//! works from the outside. A loop that resumes at the top of its body instead
//! of the middle runs the first half twice. A `let` whose environment is not
//! the one captured resumes with the right code and the wrong variables. A
//! frame list built outermost-first escapes its own loop on the second yield
//! rather than the first -- which passes any test that yields once.
//!
//! So these are mostly about *order* and *identity*: what ran, how many times,
//! in which scope. The log is a list because a set would hide all three.
//!
//! The other half is the boundary. `(yield)` is refused wherever the evaluator
//! cannot record a way back, and a refusal is only trustworthy if it also
//! happens *before* the yield's own argument runs -- otherwise a program that
//! yields somewhere impossible is left half-done.
#[cfg(test)]
mod tests {
    use crate::lisp::base_env::setup_base_env;
    use crate::lisp::{Env, EvalError, LispContext, LispExp, Parser, eval};
    use std::sync::{Arc, RwLock};

    /// A host that remembers, in order, everything the program logged.
    #[derive(Clone, Debug)]
    struct LogCtx {
        lines: Arc<RwLock<Vec<String>>>,
    }

    impl LogCtx {
        fn new() -> Self {
            Self {
                lines: Arc::new(RwLock::new(Vec::new())),
            }
        }

        fn lines(&self) -> Vec<String> {
            self.lines.read().expect("log").clone()
        }
    }

    impl LispContext for LogCtx {
        fn consume_fuel(&self, _amount: u32) -> Result<(), EvalError<LogCtx>> {
            Ok(())
        }
        fn log_diagnostic(&self, msg: &str) {
            self.lines.write().expect("log").push(msg.to_string());
        }
    }

    impl PartialEq for LogCtx {
        fn eq(&self, other: &Self) -> bool {
            Arc::ptr_eq(&self.lines, &other.lines)
        }
    }

    /// `(log TEXT)` -- append to the host's list, and nothing else.
    ///
    /// The editor's own `log` is not in the base environment, and these tests
    /// want the plain interpreter rather than the editor. A primitive of two
    /// lines keeps them there.
    fn log_primitive(
        args: &[LispExp<LogCtx>],
        _env: Arc<Env<LogCtx>>,
        ctx: &LogCtx,
    ) -> Result<LispExp<LogCtx>, EvalError<LogCtx>> {
        if let Some(LispExp::String(text)) = args.first() {
            ctx.log_diagnostic(text);
        }
        Ok(LispExp::nil())
    }

    fn setup() -> (Arc<Env<LogCtx>>, LogCtx) {
        let env = Env::new_root();
        setup_base_env(env.clone());
        env.set_function("log".into(), LispExp::primitive(log_primitive, None));
        (env, LogCtx::new())
    }

    fn run(script: &str, env: &Arc<Env<LogCtx>>, ctx: &LogCtx) -> LispExp<LogCtx> {
        try_run(script, env, ctx).unwrap_or_else(|why| panic!("evaluating {script}: {why:?}"))
    }

    fn try_run(
        script: &str,
        env: &Arc<Env<LogCtx>>,
        ctx: &LogCtx,
    ) -> Result<LispExp<LogCtx>, EvalError<LogCtx>> {
        let wrapped = format!("(progn {script})");
        let exp = Parser::new(&wrapped).next().expect("the test must parse");
        eval(&exp, env.clone(), ctx)
    }

    fn number(value: f64) -> LispExp<LogCtx> {
        LispExp::Number(value)
    }

    // ----------------------------------------------------------------
    // Stopping inside a form
    // ----------------------------------------------------------------

    #[test]
    fn a_fiber_stops_at_the_yield_and_carries_on_from_there() {
        // The whole claim, in one test: the half of the body before the yield
        // runs on the first resume and the half after it runs on the second,
        // and neither runs twice.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (log "a") (yield 1) (log "b") 2)))"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(resume f)", &env, &ctx), number(1.0));
        assert_eq!(ctx.lines(), vec!["a"]);
        assert_eq!(run("(resume f)", &env, &ctx), number(2.0));
        assert_eq!(ctx.lines(), vec!["a", "b"]);
    }

    #[test]
    fn a_bare_yield_hands_back_nil() {
        let (env, ctx) = setup();
        run(r#"(setq f (fiber (progn (yield) 7)))"#, &env, &ctx);
        assert!(run("(resume f)", &env, &ctx).is_nil());
        assert_eq!(run("(resume f)", &env, &ctx), number(7.0));
    }

    #[test]
    fn the_fiber_is_not_done_while_it_is_merely_suspended() {
        // The distinction the `pending` field exists to make. A fiber halfway
        // through its last form has an empty body and is *not* finished, and
        // a `resume` that read only the body would retire it mid-sentence.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (yield 1) (log "after") 2)))"#,
            &env,
            &ctx,
        );
        run("(resume f)", &env, &ctx);
        assert_eq!(run("(resume f)", &env, &ctx), number(2.0));
        assert_eq!(ctx.lines(), vec!["after"]);
        assert!(
            run("(resume f)", &env, &ctx).is_nil(),
            "and now it really is finished"
        );
    }

    // ----------------------------------------------------------------
    // Loops, which is the shape a worker is written in
    // ----------------------------------------------------------------

    #[test]
    fn a_loop_resumes_in_the_middle_of_the_iteration_it_stopped_in() {
        // The failure this catches is resuming at the top of the body: "one"
        // would then be logged twice per turn and "two" never at all on the
        // first.
        let (env, ctx) = setup();
        run(
            r#"(setq n 0)
               (setq f (fiber (while (< n 3)
                                (log "one")
                                (yield n)
                                (log "two")
                                (setq n (+ n 1)))))"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(resume f)", &env, &ctx), number(0.0));
        assert_eq!(ctx.lines(), vec!["one"]);
        assert_eq!(run("(resume f)", &env, &ctx), number(1.0));
        assert_eq!(ctx.lines(), vec!["one", "two", "one"]);
        assert_eq!(run("(resume f)", &env, &ctx), number(2.0));
        assert_eq!(ctx.lines(), vec!["one", "two", "one", "two", "one"]);
    }

    #[test]
    fn a_loop_that_yields_every_turn_stays_inside_its_loop() {
        // The second yield is the one that matters. Resuming runs the frames
        // that were recorded and then has to put back the ones it had not
        // reached; a driver that dropped them would escape the loop after one
        // turn and the fiber would finish early.
        let (env, ctx) = setup();
        run(
            r#"(setq n 0)
               (setq f (fiber (while t (setq n (+ n 1)) (yield n))))"#,
            &env,
            &ctx,
        );
        for expected in 1..=5 {
            assert_eq!(run("(resume f)", &env, &ctx), number(expected as f64));
        }
    }

    #[test]
    fn a_loop_runs_to_the_end_once_nothing_stops_it() {
        let (env, ctx) = setup();
        run(
            r#"(setq n 0)
               (setq f (fiber (while (< n 3) (yield n) (setq n (+ n 1)))))"#,
            &env,
            &ctx,
        );
        run("(resume f)", &env, &ctx);
        run("(resume f)", &env, &ctx);
        run("(resume f)", &env, &ctx);
        // The fourth resume finishes the last iteration, finds the condition
        // false and falls out of the loop.
        run("(resume f)", &env, &ctx);
        assert_eq!(run("n", &env, &ctx), number(3.0));
        assert!(run("(resume f)", &env, &ctx).is_nil());
    }

    // ----------------------------------------------------------------
    // Scope
    // ----------------------------------------------------------------

    #[test]
    fn a_let_binding_survives_the_suspension_it_was_made_before() {
        // The suspension carries the environment, not just the code. Carrying
        // the code alone would resume with the right statements reading the
        // wrong variables -- and it would look right whenever the names
        // happened not to collide.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (let ((x 10)) (yield) (setq x (+ x 5)) x)))"#,
            &env,
            &ctx,
        );
        run("(resume f)", &env, &ctx);
        assert_eq!(run("(resume f)", &env, &ctx), number(15.0));
    }

    #[test]
    fn a_shadowed_binding_is_still_the_inner_one_after_resuming() {
        let (env, ctx) = setup();
        run(
            r#"(setq x 1)
               (setq f (fiber (let ((x 99)) (yield) x)))"#,
            &env,
            &ctx,
        );
        run("(resume f)", &env, &ctx);
        assert_eq!(run("(resume f)", &env, &ctx), number(99.0));
        assert_eq!(
            run("x", &env, &ctx),
            number(1.0),
            "the outer one is untouched"
        );
    }

    #[test]
    fn a_yield_inside_a_called_function_suspends_the_call() {
        // A worker is written as a function, not as a literal body, so this is
        // the ordinary case rather than an exotic one.
        let (env, ctx) = setup();
        run(
            r#"(defun step-twice () (log "first") (yield 'half) (log "second"))
               (setq f (fiber (progn (step-twice) 'whole)))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(resume f)", &env, &ctx),
            LispExp::symbol("half".into())
        );
        assert_eq!(ctx.lines(), vec!["first"]);
        assert_eq!(
            run("(resume f)", &env, &ctx),
            LispExp::symbol("whole".into())
        );
        assert_eq!(ctx.lines(), vec!["first", "second"]);
    }

    #[test]
    fn nested_blocks_resume_from_the_inside_out() {
        // Three blocks deep, each with something left to do. If the frames
        // were replayed outermost first the log would come out backwards.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn
                                (let ((x 1))
                                  (progn (yield) (log "inner"))
                                  (log "middle"))
                                (log "outer"))))"#,
            &env,
            &ctx,
        );
        run("(resume f)", &env, &ctx);
        assert_eq!(ctx.lines(), Vec::<String>::new());
        run("(resume f)", &env, &ctx);
        assert_eq!(ctx.lines(), vec!["inner", "middle", "outer"]);
    }

    #[test]
    fn a_yield_in_tail_position_is_allowed() {
        // `(when ready (yield))` is how a worker says "nothing to do just
        // now", so tail position has to work. It does because a tail call has
        // nothing left of the body after it, so the body's own record serves.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (when t (yield 'parked)) (log "resumed"))))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(resume f)", &env, &ctx),
            LispExp::symbol("parked".into())
        );
        run("(resume f)", &env, &ctx);
        assert_eq!(ctx.lines(), vec!["resumed"]);
    }

    // ----------------------------------------------------------------
    // The boundary
    // ----------------------------------------------------------------

    #[test]
    fn a_yield_in_an_argument_is_refused() {
        // Coming back here would mean delivering a value into a call that was
        // half-built and is now gone.
        let (env, ctx) = setup();
        run(r#"(setq f (fiber (progn (+ 1 (yield)))))"#, &env, &ctx);
        assert_eq!(
            try_run("(resume f)", &env, &ctx).unwrap_err(),
            EvalError::YieldNotAllowed
        );
    }

    #[test]
    fn a_yield_in_a_condition_is_refused() {
        let (env, ctx) = setup();
        run(r#"(setq f (fiber (progn (if (yield) 1 2))))"#, &env, &ctx);
        assert_eq!(
            try_run("(resume f)", &env, &ctx).unwrap_err(),
            EvalError::YieldNotAllowed
        );
    }

    #[test]
    fn a_yield_inside_a_protected_form_is_refused() {
        // Suspending through an `unwind-protect` would have to choose between
        // running the cleanup for a form that has not finished and skipping it
        // for one that might never resume. Refusing is the honest third
        // answer.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (unwind-protect (yield) (log "cleanup")))))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            try_run("(resume f)", &env, &ctx).unwrap_err(),
            EvalError::YieldNotAllowed
        );
    }

    #[test]
    fn a_yield_inside_a_condition_case_is_refused() {
        // Refused as an ordinary error, which is what lets the enclosing
        // `condition-case` see it. The failure this rules out is the quiet
        // one: a fiber suspending inside a form whose handler is supposed to
        // be guarding it, so the handler is still installed on a stack that no
        // longer exists.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (condition-case e (yield) (error (log "handled"))))))"#,
            &env,
            &ctx,
        );
        run("(resume f)", &env, &ctx);
        assert_eq!(ctx.lines(), vec!["handled"]);
        assert!(
            run("(resume f)", &env, &ctx).is_nil(),
            "nothing was left suspended"
        );
    }

    #[test]
    fn a_yield_inside_a_dolist_is_refused() {
        // `dolist` keeps its position in Rust rather than in a frame, so there
        // is nothing to come back to. A worker that wants to iterate across
        // turns writes a `while`.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (dolist (x '(1 2 3)) (yield x)))))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            try_run("(resume f)", &env, &ctx).unwrap_err(),
            EvalError::YieldNotAllowed
        );
    }

    #[test]
    fn a_yield_from_inside_a_primitive_callback_is_refused() {
        // `mapcar` is part-way through its own Rust loop, holding a vector of
        // results nothing could hand back to it.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (mapcar (lambda (x) (yield x)) '(1 2)))))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            try_run("(resume f)", &env, &ctx).unwrap_err(),
            EvalError::YieldNotAllowed
        );
    }

    #[test]
    fn a_yield_outside_a_fiber_is_refused() {
        // Nothing granted permission, so there is no `resume` waiting to
        // catch it.
        let (env, ctx) = setup();
        assert_eq!(
            try_run("(yield 1)", &env, &ctx).unwrap_err(),
            EvalError::YieldNotAllowed
        );
    }

    #[test]
    fn a_refused_yield_does_not_run_its_own_argument_first() {
        // Refusing after the argument had run would leave the program
        // half-done in the one case where it cannot be told so.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (+ 1 (yield (log "side-effect"))))))"#,
            &env,
            &ctx,
        );
        assert!(try_run("(resume f)", &env, &ctx).is_err());
        assert_eq!(ctx.lines(), Vec::<String>::new());
    }

    // ----------------------------------------------------------------
    // What the old fibers did, unchanged
    // ----------------------------------------------------------------

    #[test]
    fn a_fiber_that_never_yields_runs_to_the_end_in_one_resume() {
        // A fiber's body is a `progn`, and `yield` is the only thing that
        // stops it.
        //
        // # What this replaced
        //
        // The body used to run one top-level form per resume, which was a
        // second way of suspending -- invisible in the source, and redundant
        // the moment `yield` existed. A body of three forms suspended twice
        // with no yield anywhere in sight, and explaining what a fiber does
        // meant explaining two rules that did the same job. Anything that
        // wants the old stepping now writes the yields it meant.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (log "a") (log "b") (log "c")))"#,
            &env,
            &ctx,
        );
        run("(resume f)", &env, &ctx);
        assert_eq!(ctx.lines(), vec!["a", "b", "c"]);
        assert!(run("(resume f)", &env, &ctx).is_nil());
    }

    #[test]
    fn a_fiber_is_worth_the_value_of_its_last_form() {
        // The value has to survive being handed back out through every block
        // the suspension was recorded in. A block with nothing left to run is
        // worth what the blocks inside it came to, not nil -- which is what
        // this answered before the value was carried outwards.
        let (env, ctx) = setup();
        run(r#"(setq f (fiber (progn (yield) 7)))"#, &env, &ctx);
        run("(resume f)", &env, &ctx);
        assert_eq!(run("(resume f)", &env, &ctx), number(7.0));
    }

    #[test]
    fn a_suspended_form_finishes_before_the_rest_of_the_body_runs() {
        // Two yields inside the first form of a two-form body. The third
        // resume finishes that form *and* runs the second, because nothing
        // separates them any more -- which is the point: a fiber stops where
        // it says it stops, and nowhere else.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (yield 1) (yield 2) (log "end of first"))
                              (log "second")))"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(resume f)", &env, &ctx), number(1.0));
        assert_eq!(run("(resume f)", &env, &ctx), number(2.0));
        run("(resume f)", &env, &ctx);
        assert_eq!(ctx.lines(), vec!["end of first", "second"]);
        assert!(run("(resume f)", &env, &ctx).is_nil());
    }

    #[test]
    fn an_error_inside_a_fiber_ends_it() {
        // The position it stopped at is gone, so resuming again would restart
        // it somewhere it had already been.
        let (env, ctx) = setup();
        run(
            r#"(setq f (fiber (progn (yield 1) (undefined-fn) (log "never"))))"#,
            &env,
            &ctx,
        );
        run("(resume f)", &env, &ctx);
        assert_eq!(
            try_run("(resume f)", &env, &ctx).unwrap_err(),
            EvalError::UndefinedFunction("undefined-fn".into())
        );
        assert!(run("(resume f)", &env, &ctx).is_nil());
        assert_eq!(ctx.lines(), Vec::<String>::new());
    }

    #[test]
    fn a_fiber_may_be_resumed_from_inside_lisp_that_the_fiber_itself_started() {
        // The deadlock this guards against: `resume` used to hold the fiber's
        // write lock while evaluating, so any Lisp that reached back to its
        // own fiber hung the thread on a lock nothing could see.
        let (env, ctx) = setup();
        run(
            r#"(setq inner (fiber (progn (log "inner") (yield))))
               (setq outer (fiber (progn (resume inner) (log "outer"))))"#,
            &env,
            &ctx,
        );
        run("(resume outer)", &env, &ctx);
        assert_eq!(ctx.lines(), vec!["inner", "outer"]);
    }
}
