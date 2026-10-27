//! Background workers: what a step is, and what ends one, where every mistake
//! is silent -- a worker that starts over, a replaced one still running, a
//! runaway. The WorkManager is in manual mode: the test takes the steps, with
//! `run_until_idle` or `run_for`, on a clock that moves when told.
//!
//! Design: `doc/architettura-editor.typ, <lavori-lisp>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, TURN_INTERVAL, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::time::Duration;

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env")
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

    fn number(value: f64) -> LispExp<Ctx> {
        LispExp::Number(value)
    }

    /// Take the steps that are due now.
    fn steps_due(ctx: &Ctx) -> usize {
        ctx.work().run_until_idle(ctx)
    }

    /// Let SPAN of the editor's time pass, taking every step that falls due.
    fn wait(ctx: &Ctx, span: Duration) -> usize {
        ctx.work().run_for(ctx, span)
    }

    fn running(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> LispExp<Ctx> {
        run("(running-workers)", env, ctx)
    }

    fn logged(ctx: &Ctx, needle: &str) -> bool {
        ctx.get_logs().iter().any(|line| line.contains(needle))
    }

    // ----------------------------------------------------------------
    // A step
    // ----------------------------------------------------------------

    #[test]
    fn a_step_resumes_the_worker_once_and_no_more() {
        // The bounded-work property the whole mechanism rests on: whatever
        // else a worker's loop would do, one step advances it as far as its
        // next `yield` and stops.
        let (ctx, env) = editor();
        run(
            "(setq counter 0)
             (define-worker 'counter (fiber (while t (setq counter (+ counter 1)) (yield))))",
            &env,
            &ctx,
        );
        assert_eq!(
            run("counter", &env, &ctx),
            number(0.0),
            "nothing runs by itself"
        );
        steps_due(&ctx);
        assert_eq!(
            run("counter", &env, &ctx),
            number(1.0),
            "its first step is at once"
        );
        wait(&ctx, TURN_INTERVAL);
        assert_eq!(
            run("counter", &env, &ctx),
            number(2.0),
            "and a plain (yield) waits a turn"
        );
    }

    #[test]
    fn a_worker_keeps_its_place_between_steps() {
        // Resuming at the top of the loop instead of after the yield would
        // give the same count and a different order, which is why this reads
        // the sequence rather than the total.
        let (ctx, env) = editor();
        run(
            r#"(setq log nil)
               (define-worker 'ordered
                 (fiber (let ((left 2))
                          (while (> left 0)
                            (setq log (append log (list "before")))
                            (yield 'continue)
                            (setq log (append log (list "after")))
                            (setq left (- left 1))))))"#,
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert_eq!(
            run("log", &env, &ctx),
            run(r#"'("before" "after" "before" "after")"#, &env, &ctx),
            "and (yield 'continue) went on at once, three steps in one drive"
        );
        assert!(running(&ctx, &env).is_nil());
    }

    #[test]
    fn a_worker_retires_when_its_program_finishes() {
        let (ctx, env) = editor();
        run(
            "(define-worker 'brief (fiber (progn (yield) 'done)))",
            &env,
            &ctx,
        );
        assert_eq!(running(&ctx, &env), run(r#"'("brief")"#, &env, &ctx));
        steps_due(&ctx);
        wait(&ctx, TURN_INTERVAL);
        assert!(
            running(&ctx, &env).is_nil(),
            "it stops being listed once its program has finished"
        );
        assert!(
            !ctx.work().jobs().iter().any(|job| job.name == "brief"),
            "and it is gone from the WorkManager"
        );
    }

    // ----------------------------------------------------------------
    // The allowance
    // ----------------------------------------------------------------

    #[test]
    fn a_worker_that_never_yields_is_stopped_rather_than_holding_the_thread() {
        // The failure this prevents is the invisible one: without a bound,
        // this loop takes syntax colouring and the prescanner with it for the
        // rest of the session, and nothing anywhere says so.
        let (ctx, env) = editor();
        run(
            "(setq worker-fuel 10000)
             (define-worker 'runaway (fiber (while t (setq x 1))))",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert!(
            running(&ctx, &env).is_nil(),
            "the step ends and the worker with it"
        );
        assert!(
            logged(
                &ctx,
                "[BKG] error: runaway: a step ran past its allowance of 10000 fuel"
            ),
            "and says why: {:?}",
            ctx.get_logs()
        );
    }

    #[test]
    fn a_step_gets_its_whole_allowance_back_every_time() {
        // Each step is set to the allowance rather than topped up towards it,
        // so a worker that spends most of its budget one step is not left
        // short the next. Without that, a busy worker would starve itself
        // after a few steps and then be reported as a runaway.
        let (ctx, env) = editor();
        run(
            "(setq counter 0) (setq worker-fuel 5000)
             (define-worker 'steady
               (fiber (while t
                        (let ((n 0))
                          (while (< n 50) (setq n (+ n 1))))
                        (setq counter (+ counter 1))
                        (yield))))",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        wait(&ctx, TURN_INTERVAL * 19);
        assert_eq!(run("counter", &env, &ctx), number(20.0));
        assert_eq!(running(&ctx, &env), run(r#"'("steady")"#, &env, &ctx));
    }

    #[test]
    fn a_step_leaves_the_fuel_of_the_thread_that_took_it_as_it_was() {
        // In these tests the thread that takes the steps is the test's own,
        // which goes on evaluating Lisp afterwards: a worker's allowance must
        // not become the test's.
        let (ctx, env) = editor();
        run(
            "(define-worker 'light (fiber (while t (yield))))",
            &env,
            &ctx,
        );
        risp::set_remaining(1_234_567);
        steps_due(&ctx);
        assert_eq!(crate::primitives::lisp_job::fuel_left(), 1_234_567);
        risp::set_remaining(risp::DEFAULT_FUEL);
    }

    #[test]
    fn an_error_in_a_worker_stops_it_and_is_reported() {
        let (ctx, env) = editor();
        run(
            "(define-worker 'broken (fiber (progn (undefined-fn))))",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert!(running(&ctx, &env).is_nil());
        assert!(
            logged(&ctx, "[BKG] error: broken: "),
            "the log names the worker: {:?}",
            ctx.get_logs()
        );
    }

    // ----------------------------------------------------------------
    // Replacing one
    // ----------------------------------------------------------------

    #[test]
    fn redefining_a_worker_retires_the_one_it_replaces() {
        // What makes re-evaluating a module safe. Two copies of an indexer
        // running against each other is not an error anywhere -- it is just
        // twice the work and a race over whatever they both write.
        let (ctx, env) = editor();
        let define =
            "(define-worker 'indexer (fiber (while t (setq counter (+ counter 1)) (yield))))";
        run("(setq counter 0)", &env, &ctx);
        run(define, &env, &ctx);
        steps_due(&ctx);
        run(define, &env, &ctx);
        steps_due(&ctx);
        assert_eq!(
            run("counter", &env, &ctx),
            number(2.0),
            "the replaced one took no step after the replacement came"
        );
        wait(&ctx, TURN_INTERVAL);
        assert_eq!(
            run("counter", &env, &ctx),
            number(3.0),
            "and from then on one counts, not two"
        );
        let named = ctx
            .work()
            .jobs()
            .iter()
            .filter(|job| job.name == "indexer")
            .count();
        assert_eq!(named, 1);
    }

    #[test]
    fn a_retired_worker_does_not_take_its_replacement_off_the_list() {
        // The old job hears that it was replaced *after* the new one is live,
        // and its leaving must not clear the name the new one now has.
        let (ctx, env) = editor();
        run(
            "(define-worker 'indexer (fiber (while t (yield))))",
            &env,
            &ctx,
        );
        run(
            "(define-worker 'indexer (fiber (while t (yield))))",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert_eq!(running(&ctx, &env), run(r#"'("indexer")"#, &env, &ctx));
    }

    #[test]
    fn stopping_a_worker_stops_it_before_its_next_step() {
        let (ctx, env) = editor();
        run(
            "(setq counter 0)
             (define-worker 'stoppable (fiber (while t (setq counter (+ counter 1)) (yield))))",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert_eq!(run("(stop-worker 'stoppable)", &env, &ctx), LispExp::t());
        wait(&ctx, TURN_INTERVAL * 5);
        assert_eq!(run("counter", &env, &ctx), number(1.0));
        assert!(
            run("(stop-worker 'stoppable)", &env, &ctx).is_nil(),
            "stopping one that is already stopped says so"
        );
    }

    #[test]
    fn a_worker_stopped_before_its_first_step_never_runs() {
        let (ctx, env) = editor();
        run(
            "(setq counter 0)
             (define-worker 'never (fiber (while t (setq counter (+ counter 1)) (yield))))
             (stop-worker 'never)",
            &env,
            &ctx,
        );
        wait(&ctx, TURN_INTERVAL * 5);
        assert_eq!(run("counter", &env, &ctx), number(0.0));
    }

    // ----------------------------------------------------------------
    // The Lisp door
    // ----------------------------------------------------------------

    #[test]
    fn define_worker_registers_the_name_and_lists_it() {
        let (ctx, env) = editor();
        run("(define-worker 'b (fiber (while t (yield))))", &env, &ctx);
        run("(define-worker 'a (fiber (while t (yield))))", &env, &ctx);
        assert_eq!(
            running(&ctx, &env),
            run(r#"'("a" "b")"#, &env, &ctx),
            "listed at once, and sorted -- and only what Lisp started"
        );
    }

    #[test]
    fn define_worker_refuses_anything_that_is_not_a_fiber() {
        // Refused here, where it is an error the caller sees, rather than a
        // step later on another thread where the only way to report it is a
        // line in a log nobody is reading.
        let (ctx, env) = editor();
        assert!(try_run("(define-worker 'bad (lambda () 1))", &env, &ctx).is_err());
        assert!(running(&ctx, &env).is_nil());
    }

    #[test]
    fn define_worker_takes_a_string_name_as_readily_as_a_symbol() {
        let (ctx, env) = editor();
        run(
            r#"(define-worker "quoted" (fiber (while t (yield))))"#,
            &env,
            &ctx,
        );
        assert_eq!(running(&ctx, &env), run(r#"'("quoted")"#, &env, &ctx));
    }

    #[test]
    fn a_worker_cannot_open_a_prompt() {
        // A minibuffer put up by a worker is a window nobody is typing into,
        // over a command nobody started. Refused where it can still be
        // reported rather than half-working somewhere baffling.
        let (ctx, env) = editor();
        run(
            r#"(define-worker 'asker
                 (fiber (progn (minibuffer-read "Why?:" nil nil nil) (yield))))"#,
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert!(
            running(&ctx, &env).is_nil(),
            "the step fails and the worker with it"
        );
        assert!(
            logged(&ctx, "cannot open a prompt"),
            "and says why: {:?}",
            ctx.get_logs()
        );
    }

    #[test]
    fn the_prompt_refusal_applies_only_inside_a_step() {
        // The flag is cleared however a step ends, including by the error
        // above -- otherwise the first failing worker would make every later
        // prompt on that thread impossible. Here the thread is the test's.
        let (ctx, env) = editor();
        run(
            r#"(define-worker 'asker
                 (fiber (progn (minibuffer-read "Why?:" nil nil nil) (yield))))"#,
            &env,
            &ctx,
        );
        steps_due(&ctx);
        run("(setq frame-width 80) (setq frame-height 24)", &env, &ctx);
        assert!(
            try_run(r#"(minibuffer-read "Fine:" nil nil nil)"#, &env, &ctx).is_ok(),
            "an ordinary command may still prompt"
        );
    }
}
