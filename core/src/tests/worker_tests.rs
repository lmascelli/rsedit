//! Background workers: what a turn is, and what ends one.
//!
//! # What is worth testing here, and what cannot be
//!
//! A worker runs on the scheduler's thread, on a timer, which is exactly the
//! shape a test cannot wait on without being a race. So these do not wait.
//! They drive the job the scheduler would drive -- `ScheduledTask::execute`,
//! one call per turn -- and check what one turn does and what it answers.
//! That is the whole contract between a worker and the scheduler: the
//! scheduler's own loop is tested by the highlighter's existence.
//!
//! The things worth pinning down are the ones that are silent when wrong. A
//! worker that keeps its place across turns looks identical to one that starts
//! over, until the log is read in order. A replaced worker looks identical to
//! a single worker, until both are running and the count is doubled. And a
//! runaway worker looks like nothing at all -- the editor simply stops
//! colouring, with no error anywhere.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::task::ScheduledTask;
    use crate::worker::{DEFAULT_WORKER_FUEL, LispWorker};
    use std::sync::Arc;

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

    /// A worker built the way `define-worker` builds one, but handed back so a
    /// test can take its turns itself instead of waiting for the scheduler.
    fn worker(
        name: &str,
        fiber_src: &str,
        fuel: u32,
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
    ) -> LispWorker<GapBuffer> {
        let fiber = run(fiber_src, env, ctx);
        let generation = ctx.runtime_mut(|runtime| runtime.next_worker_generation(name));
        LispWorker::new(name.to_string(), generation, fiber, env.clone(), fuel)
    }

    fn number(value: f64) -> LispExp<Ctx> {
        LispExp::Number(value)
    }

    // ----------------------------------------------------------------
    // A turn
    // ----------------------------------------------------------------

    #[test]
    fn a_turn_resumes_the_worker_once_and_no_more() {
        // The bounded-work property the whole mechanism rests on: whatever
        // else a worker's loop would do, one turn advances it as far as its
        // next `yield` and stops.
        let (ctx, env) = editor();
        run("(setq counter 0)", &env, &ctx);
        let mut task = worker(
            "counter",
            "(fiber (while t (setq counter (+ counter 1)) (yield)))",
            DEFAULT_WORKER_FUEL,
            &env,
            &ctx,
        );
        assert!(task.execute(&ctx));
        assert_eq!(run("counter", &env, &ctx), number(1.0));
        assert!(task.execute(&ctx));
        assert_eq!(run("counter", &env, &ctx), number(2.0));
    }

    #[test]
    fn a_worker_keeps_its_place_between_turns() {
        // Resuming at the top of the loop instead of after the yield would
        // give the same count and a different order, which is why this reads
        // the sequence rather than the total.
        let (ctx, env) = editor();
        run("(setq log nil)", &env, &ctx);
        let mut task = worker(
            "ordered",
            r#"(fiber (while t
                        (setq log (append log (list "before")))
                        (yield)
                        (setq log (append log (list "after")))))"#,
            DEFAULT_WORKER_FUEL,
            &env,
            &ctx,
        );
        task.execute(&ctx);
        task.execute(&ctx);
        assert_eq!(
            run("log", &env, &ctx),
            run(r#"'("before" "after" "before")"#, &env, &ctx)
        );
    }

    #[test]
    fn a_worker_retires_when_its_program_finishes() {
        // False is how a `ScheduledTask` asks to be taken off the list, and a
        // worker whose fiber is done has nothing left to be woken for.
        let (ctx, env) = editor();
        let mut task = worker(
            "brief",
            "(fiber (progn (yield) 'done))",
            DEFAULT_WORKER_FUEL,
            &env,
            &ctx,
        );
        assert!(task.execute(&ctx), "the first turn reaches the yield");
        assert!(!task.execute(&ctx), "the second finishes the program");
        assert!(
            !ctx.runtime(|runtime| runtime.running_workers().contains(&"brief".to_string())),
            "and it stops being listed as running"
        );
    }

    // ----------------------------------------------------------------
    // The allowance
    // ----------------------------------------------------------------

    #[test]
    fn a_worker_that_never_yields_is_stopped_rather_than_holding_the_thread() {
        // The failure this prevents is the invisible one: without a bound,
        // this loop takes syntax colouring, the prescanner and shell output
        // with it for the rest of the session, and nothing anywhere says so.
        let (ctx, env) = editor();
        let mut task = worker(
            "runaway",
            "(fiber (while t (setq x 1)))",
            10_000,
            &env,
            &ctx,
        );
        assert!(!task.execute(&ctx), "the turn ends and the worker retires");
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("runaway") && line.contains("allowance")),
            "and says why: {:?}",
            ctx.get_logs()
        );
    }

    #[test]
    fn a_turn_gets_its_whole_allowance_back_every_time() {
        // Each turn is set to the allowance rather than topped up towards it,
        // so a worker that spends most of its budget one turn is not left
        // short the next. Without that, a busy worker would starve itself
        // after a few turns and then be reported as a runaway.
        let (ctx, env) = editor();
        run("(setq counter 0)", &env, &ctx);
        let mut task = worker(
            "steady",
            r#"(fiber (while t
                        (let ((n 0))
                          (while (< n 50) (setq n (+ n 1))))
                        (setq counter (+ counter 1))
                        (yield)))"#,
            5_000,
            &env,
            &ctx,
        );
        for turn in 1..=20 {
            assert!(task.execute(&ctx), "turn {turn} must not be the last");
        }
        assert_eq!(run("counter", &env, &ctx), number(20.0));
    }

    #[test]
    fn an_error_in_a_worker_stops_it_and_is_reported() {
        let (ctx, env) = editor();
        let mut task = worker(
            "broken",
            "(fiber (progn (undefined-fn)))",
            DEFAULT_WORKER_FUEL,
            &env,
            &ctx,
        );
        assert!(!task.execute(&ctx));
        assert!(
            ctx.get_logs().iter().any(|line| line.contains("broken")),
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
        run("(setq counter 0)", &env, &ctx);
        let body = "(fiber (while t (setq counter (+ counter 1)) (yield)))";
        let mut first = worker("indexer", body, DEFAULT_WORKER_FUEL, &env, &ctx);
        assert!(first.execute(&ctx));
        let mut second = worker("indexer", body, DEFAULT_WORKER_FUEL, &env, &ctx);
        assert!(
            !first.execute(&ctx),
            "the replaced one retires at its next turn"
        );
        assert!(second.execute(&ctx), "and the replacement carries on");
        assert_eq!(
            run("counter", &env, &ctx),
            number(2.0),
            "two turns ran in all, not three"
        );
    }

    #[test]
    fn a_retiring_worker_does_not_take_its_replacement_off_the_list() {
        // The generation check exists for exactly this: the old job wakes up
        // *after* the new one is live, and a retirement that did not name a
        // generation would clear the wrong entry.
        let (ctx, env) = editor();
        let body = "(fiber (while t (yield)))";
        let mut first = worker("indexer", body, DEFAULT_WORKER_FUEL, &env, &ctx);
        let mut second = worker("indexer", body, DEFAULT_WORKER_FUEL, &env, &ctx);
        assert!(!first.execute(&ctx));
        assert!(
            ctx.runtime(|runtime| runtime.running_workers().contains(&"indexer".to_string())),
            "the replacement is still running"
        );
        assert!(second.execute(&ctx));
    }

    #[test]
    fn stopping_a_worker_retires_it_at_its_next_turn() {
        let (ctx, env) = editor();
        let mut task = worker(
            "stoppable",
            "(fiber (while t (yield)))",
            DEFAULT_WORKER_FUEL,
            &env,
            &ctx,
        );
        assert!(task.execute(&ctx));
        assert_eq!(run("(stop-worker 'stoppable)", &env, &ctx), LispExp::t());
        assert!(!task.execute(&ctx));
        assert!(
            run("(stop-worker 'stoppable)", &env, &ctx).is_nil(),
            "stopping one that is already stopped says so"
        );
    }

    #[test]
    fn a_worker_stopped_before_its_first_turn_never_runs() {
        let (ctx, env) = editor();
        run("(setq counter 0)", &env, &ctx);
        let mut task = worker(
            "never",
            "(fiber (while t (setq counter (+ counter 1)) (yield)))",
            DEFAULT_WORKER_FUEL,
            &env,
            &ctx,
        );
        run("(stop-worker 'never)", &env, &ctx);
        assert!(!task.execute(&ctx));
        assert_eq!(run("counter", &env, &ctx), number(0.0));
    }

    // ----------------------------------------------------------------
    // The Lisp door
    // ----------------------------------------------------------------

    #[test]
    fn define_worker_registers_the_name_and_lists_it() {
        let (ctx, env) = editor();
        run("(define-worker 'a (fiber (while t (yield))))", &env, &ctx);
        run("(define-worker 'b (fiber (while t (yield))))", &env, &ctx);
        assert_eq!(
            run("(running-workers)", &env, &ctx),
            run(r#"'("a" "b")"#, &env, &ctx)
        );
    }

    #[test]
    fn define_worker_refuses_anything_that_is_not_a_fiber() {
        // Refused here, where it is an error the caller sees, rather than a
        // turn later on another thread where the only way to report it is a
        // line in a log nobody is reading.
        let (ctx, env) = editor();
        assert!(try_run("(define-worker 'bad (lambda () 1))", &env, &ctx).is_err());
        assert!(run("(running-workers)", &env, &ctx).is_nil());
    }

    #[test]
    fn define_worker_takes_a_string_name_as_readily_as_a_symbol() {
        let (ctx, env) = editor();
        run(
            r#"(define-worker "quoted" (fiber (while t (yield))))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(running-workers)", &env, &ctx),
            run(r#"'("quoted")"#, &env, &ctx)
        );
    }

    #[test]
    fn a_worker_cannot_open_a_prompt() {
        // A minibuffer put up by a worker is a window nobody is typing into,
        // over a command nobody started. Refused where it can still be
        // reported rather than half-working somewhere baffling.
        let (ctx, env) = editor();
        let mut task = worker(
            "asker",
            r#"(fiber (progn (minibuffer-read "Why?:" nil nil nil) (yield)))"#,
            DEFAULT_WORKER_FUEL,
            &env,
            &ctx,
        );
        assert!(!task.execute(&ctx), "the turn fails and the worker retires");
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("cannot open a prompt")),
            "and says why: {:?}",
            ctx.get_logs()
        );
    }

    #[test]
    fn the_prompt_refusal_applies_only_inside_a_worker_turn() {
        // The flag is cleared however a turn ends, including by the error
        // above -- otherwise the first failing worker would make every later
        // prompt on that thread impossible.
        let (ctx, env) = editor();
        let mut task = worker(
            "asker",
            r#"(fiber (progn (minibuffer-read "Why?:" nil nil nil) (yield)))"#,
            DEFAULT_WORKER_FUEL,
            &env,
            &ctx,
        );
        task.execute(&ctx);
        run("(setq frame-width 80) (setq frame-height 24)", &env, &ctx);
        assert!(
            try_run(r#"(minibuffer-read "Fine:" nil nil nil)"#, &env, &ctx).is_ok(),
            "an ordinary command may still prompt"
        );
    }
}
