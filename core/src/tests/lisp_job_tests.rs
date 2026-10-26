//! What a Lisp job asks for when it yields, and the primitives that reach the
//! scheduler from Lisp: `emit-event`, the options and `work-list`. A worker
//! counts its steps on a WorkManager driven by hand, so "250 ms later" is
//! exactly 250 ms later, and a wait nothing wakes stays asleep.
//!
//! Design: `doc/architettura-editor.typ, <lavori-lisp>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::modes::highlighter::TURN_INTERVAL;
    use crate::primitives::lisp_job::fuel_left;
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::time::Duration;

    type Ctx = EditorState<GapBuffer>;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        // The editor's own jobs take their first steps, so that what is
        // driven from here on is the test's.
        ctx.work().run_until_idle(&ctx);
        (ctx, env)
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

    /// Start a worker that counts its steps in `counter` and yields YIELDED
    /// at the end of each, forever; INTERVAL is `define-worker`'s. Its first
    /// step is taken before this returns.
    fn counting(ctx: &Ctx, env: &Arc<Env<Ctx>>, yielded: &str, interval: &str) {
        run(
            &format!(
                "(setq counter 0)
                 (define-worker 'counter
                   (fiber (while t (setq counter (+ counter 1)) (yield {yielded})))
                   {interval})"
            ),
            env,
            ctx,
        );
        ctx.work().run_until_idle(ctx);
    }

    /// How many steps the counting worker has taken.
    fn count(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> u64 {
        match run("counter", env, ctx) {
            LispExp::Number(n) => n as u64,
            other => panic!("a count, not {other:?}"),
        }
    }

    /// Let SPAN of the editor's time pass, taking every step that falls due.
    fn wait(ctx: &Ctx, span: Duration) {
        ctx.work().run_for(ctx, span);
    }

    fn logged(ctx: &Ctx, needle: &str) -> bool {
        ctx.get_logs().iter().any(|line| line.contains(needle))
    }

    // ----------------------------------------------------------------
    // The table
    // ----------------------------------------------------------------

    #[test]
    fn a_plain_yield_waits_a_turn() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "", "");
        assert_eq!(count(&ctx, &env), 1, "the first step is at once");
        wait(&ctx, TURN_INTERVAL - ms(1));
        assert_eq!(count(&ctx, &env), 1);
        wait(&ctx, ms(1));
        assert_eq!(count(&ctx, &env), 2);
    }

    #[test]
    fn a_plain_yield_waits_the_interval_the_worker_was_given() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "", "250");
        wait(&ctx, ms(249));
        assert_eq!(count(&ctx, &env), 1);
        wait(&ctx, ms(1));
        assert_eq!(count(&ctx, &env), 2);
    }

    #[test]
    fn any_other_value_waits_as_a_plain_yield_does() {
        // What a job written before the table yields: its progress. It goes
        // on exactly as it did.
        for yielded in [
            "42",
            r#""halfway""#,
            "'(3 . 10)",
            "'(progress 3)",
            "'sleep",
            "t",
        ] {
            let (ctx, env) = editor();
            counting(&ctx, &env, yielded, "100");
            wait(&ctx, ms(99));
            assert_eq!(count(&ctx, &env), 1, "{yielded}");
            wait(&ctx, ms(1));
            assert_eq!(count(&ctx, &env), 2, "{yielded}");
        }
    }

    #[test]
    fn continue_goes_on_as_soon_as_its_turn_comes_round() {
        let (ctx, env) = editor();
        run(
            "(setq counter 0)
             (define-worker 'eager
               (fiber (let ((left 5))
                        (while (> left 0)
                          (setq counter (+ counter 1))
                          (setq left (- left 1))
                          (yield 'continue)))))",
            &env,
            &ctx,
        );
        ctx.work().run_until_idle(&ctx);
        assert_eq!(
            count(&ctx, &env),
            5,
            "every step in one drive, no time passing"
        );
    }

    #[test]
    fn sleep_waits_what_it_says() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "'(sleep 250)", "");
        wait(&ctx, ms(249));
        assert_eq!(count(&ctx, &env), 1);
        wait(&ctx, ms(1));
        assert_eq!(count(&ctx, &env), 2);
        wait(&ctx, ms(250));
        assert_eq!(count(&ctx, &env), 3);
    }

    #[test]
    fn a_sleep_is_not_cut_short_by_an_event() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "'(sleep 250)", "");
        run("(emit-event 'buffer-changed)", &env, &ctx);
        wait(&ctx, ms(249));
        assert_eq!(count(&ctx, &env), 1);
    }

    #[test]
    fn wait_sleeps_until_a_topic_it_names_is_emitted() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "'(wait my-refresh)", "");
        let delay = ctx.work().options().event_delay();
        // A topic the worker did not have counts as changed, once: the step
        // that named it may have looked just before an emission.
        wait(&ctx, delay);
        assert_eq!(count(&ctx, &env), 2, "one more look, event-delay later");
        wait(&ctx, Duration::from_secs(60));
        assert_eq!(
            count(&ctx, &env),
            2,
            "and then nothing wakes it but the topic"
        );

        run("(emit-event 'some-other-topic)", &env, &ctx);
        wait(&ctx, Duration::from_secs(1));
        assert_eq!(count(&ctx, &env), 2, "not a topic it named");

        run("(emit-event 'my-refresh)", &env, &ctx);
        wait(&ctx, delay - ms(1));
        assert_eq!(count(&ctx, &env), 2, "an event wakes it event-delay later");
        wait(&ctx, ms(1));
        assert_eq!(count(&ctx, &env), 3);
    }

    #[test]
    fn the_editor_s_own_topics_wake_a_lisp_job() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "'(wait buffer-changed)", "");
        wait(&ctx, Duration::from_secs(1));
        let settled = count(&ctx, &env);
        run(r#"(insert "x")"#, &env, &ctx);
        wait(&ctx, Duration::from_secs(1));
        assert_eq!(count(&ctx, &env), settled + 1, "an edit is an emission");
    }

    #[test]
    fn wait_at_most_wakes_on_the_topic_or_the_deadline() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "'(wait-at-most 500 my-refresh)", "");
        let delay = ctx.work().options().event_delay();
        wait(&ctx, delay);
        assert_eq!(count(&ctx, &env), 2, "the look a new topic is worth");
        // From the end of that step: 500 ms with nothing emitted.
        wait(&ctx, ms(499));
        assert_eq!(count(&ctx, &env), 2);
        wait(&ctx, ms(1));
        assert_eq!(count(&ctx, &env), 3, "the deadline");
        run("(emit-event 'my-refresh)", &env, &ctx);
        wait(&ctx, delay);
        assert_eq!(
            count(&ctx, &env),
            4,
            "the topic, well before the next deadline"
        );
    }

    #[test]
    fn a_wait_naming_nothing_keeps_the_topics_it_had() {
        let (ctx, env) = editor();
        run(
            "(setq counter 0)
             (define-worker 'keeper
               (fiber (progn
                        (setq counter (+ counter 1))
                        (yield '(wait mine))
                        (while t
                          (setq counter (+ counter 1))
                          (yield '(wait))))))",
            &env,
            &ctx,
        );
        ctx.work().run_until_idle(&ctx);
        wait(&ctx, Duration::from_secs(1));
        let settled = count(&ctx, &env);
        run("(emit-event 'mine)", &env, &ctx);
        wait(&ctx, Duration::from_secs(1));
        assert_eq!(count(&ctx, &env), settled + 1, "still its topic");
    }

    #[test]
    fn a_wait_with_no_topic_ever_named_stops_the_worker_and_says_why() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "'(wait)", "");
        assert!(run("(running-workers)", &env, &ctx).is_nil());
        assert!(
            logged(
                &ctx,
                "counter waits for an event but has no topic to be woken by"
            ),
            "{:?}",
            ctx.get_logs()
        );
    }

    #[test]
    fn a_shape_written_wrong_stops_the_job_and_names_the_shape() {
        for (yielded, shape) in [
            ("'(sleep)", "(sleep MS)"),
            ("'(sleep -1)", "(sleep MS)"),
            (r#"'(sleep "soon")"#, "(sleep MS)"),
            ("'(sleep 1 2)", "(sleep MS)"),
            ("'(sleep . 5)", "(sleep MS)"),
            ("'(wait 42)", "(wait TOPIC...)"),
            (
                "'(wait-at-most buffer-changed)",
                "(wait-at-most MS TOPIC...)",
            ),
            ("'(wait-at-most)", "(wait-at-most MS TOPIC...)"),
        ] {
            let (ctx, env) = editor();
            run(
                &format!(
                    "(setq outcome 'unset)
                     (defun on-done (name finished) (setq outcome (list name finished)))
                     (background-call 'wrong (fiber (while t (yield {yielded}))) nil 'on-done)"
                ),
                &env,
                &ctx,
            );
            ctx.work().run_until_idle(&ctx);
            ctx.run_owed_callbacks(&env);
            assert_eq!(
                run("outcome", &env, &ctx),
                run(r#"(list "wrong" nil)"#, &env, &ctx),
                "{yielded} fails the job"
            );
            assert!(
                logged(&ctx, "[BKG] error: wrong: yielded") && logged(&ctx, shape),
                "{yielded}: {:?}",
                ctx.get_logs()
            );
        }
    }

    #[test]
    fn a_topic_may_be_named_with_a_string() {
        let (ctx, env) = editor();
        counting(&ctx, &env, r#"(list 'wait "my-refresh")"#, "");
        wait(&ctx, Duration::from_secs(1));
        let settled = count(&ctx, &env);
        run(r#"(emit-event "my-refresh")"#, &env, &ctx);
        wait(&ctx, Duration::from_secs(1));
        assert_eq!(count(&ctx, &env), settled + 1);
    }

    // ----------------------------------------------------------------
    // emit-event
    // ----------------------------------------------------------------

    #[test]
    fn one_emission_wakes_every_job_waiting_for_it_once() {
        let (ctx, env) = editor();
        run(
            "(setq a 0) (setq b 0)
             (define-worker 'a (fiber (while t (setq a (+ a 1)) (yield '(wait shared)))))
             (define-worker 'b (fiber (while t (setq b (+ b 1)) (yield '(wait shared)))))",
            &env,
            &ctx,
        );
        wait(&ctx, Duration::from_secs(1));
        assert_eq!(run("(list a b)", &env, &ctx), run("'(2 2)", &env, &ctx));
        assert!(run("(emit-event 'shared) (emit-event 'shared)", &env, &ctx).is_nil());
        wait(&ctx, Duration::from_secs(1));
        assert_eq!(
            run("(list a b)", &env, &ctx),
            run("'(3 3)", &env, &ctx),
            "a burst is one wake-up each"
        );
    }

    #[test]
    fn emit_event_wants_a_name() {
        let (ctx, env) = editor();
        assert!(try_run("(emit-event 42)", &env, &ctx).is_err());
        assert!(try_run("(emit-event)", &env, &ctx).is_err());
    }

    // ----------------------------------------------------------------
    // The options
    // ----------------------------------------------------------------

    #[test]
    fn an_option_reads_back_what_was_set_and_counts_at_once() {
        let (ctx, env) = editor();
        assert_eq!(
            run("(work-option 'event-delay)", &env, &ctx),
            LispExp::Number(20.0)
        );
        assert_eq!(
            run("(set-work-option 'event-delay 50)", &env, &ctx),
            LispExp::Number(50.0),
            "it answers with the value"
        );
        assert_eq!(
            run(r#"(work-option "event-delay")"#, &env, &ctx),
            LispExp::Number(50.0)
        );
        assert_eq!(
            ctx.work().options().event_delay(),
            ms(50),
            "the scheduler's own"
        );

        counting(&ctx, &env, "'(wait my-refresh)", "");
        wait(&ctx, ms(49));
        assert_eq!(count(&ctx, &env), 1, "the new delay, not the old");
        wait(&ctx, ms(1));
        assert_eq!(count(&ctx, &env), 2);
    }

    #[test]
    fn an_option_refuses_what_it_cannot_take_and_changes_nothing() {
        let (ctx, env) = editor();
        let message = |src: &str| match try_run(src, &env, &ctx) {
            Err(EvalError::RuntimeMessage(message)) => message,
            other => panic!("{src} should be refused with a message, got {other:?}"),
        };
        assert!(
            message("(set-work-option 'high-steps 0)")
                .contains("high-steps takes a value from 1 to 1000"),
            "the range is in the message"
        );
        assert!(
            message("(set-work-option 'no-such-option 1)")
                .contains("no option called no-such-option")
        );
        assert!(
            message("(work-option 'no-such-option)").contains("no option called no-such-option")
        );
        for bad in [
            "(set-work-option 'high-steps 2.5)",
            "(set-work-option 'high-steps -1)",
            "(set-work-option 'high-steps \"6\")",
        ] {
            assert!(try_run(bad, &env, &ctx).is_err(), "{bad}");
        }
        assert_eq!(
            run("(work-option 'high-steps)", &env, &ctx),
            LispExp::Number(4.0),
            "nothing changed"
        );
    }

    #[test]
    fn work_options_lists_every_option_with_its_value() {
        let (ctx, env) = editor();
        run("(set-work-option 'high-steps 6)", &env, &ctx);
        assert_eq!(
            run("(work-options)", &env, &ctx),
            run(
                "'(high-steps 6 event-delay 20 long-step 40 late-high 40 late-normal 200
                   stuck-step 1000 busy-share 50 summary-every 10000 watchdog-every 250)",
                &env,
                &ctx
            )
        );
    }

    // ----------------------------------------------------------------
    // work-list
    // ----------------------------------------------------------------

    /// The entry of `work-list` called NAME, as a list of its values in the
    /// order of KEYS.
    fn entry(name: &str, keys: &[&str], env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let getters: Vec<String> = keys
            .iter()
            .map(|key| format!("(nth 1 (memq '{key} entry))"))
            .collect();
        run(
            &format!(
                r#"(let ((found nil))
                     (mapc (lambda (entry)
                             (if (equal (nth 1 (memq 'name entry)) "{name}")
                                 (setq found (list {}))))
                           (work-list))
                     found)"#,
                getters.join(" ")
            ),
            env,
            ctx,
        )
    }

    #[test]
    fn work_list_shows_the_editor_s_work_and_lisp_s_alike() {
        let (ctx, env) = editor();
        counting(&ctx, &env, "'(sleep 1000)", "");
        assert_eq!(
            entry("highlighter", &["kind", "class", "state"], &env, &ctx),
            run("'(job high waiting)", &env, &ctx)
        );
        assert_eq!(
            entry("file-watcher", &["kind", "class", "state"], &env, &ctx),
            run("'(job normal sleeping)", &env, &ctx)
        );
        assert_eq!(
            entry("counter", &["kind", "class", "state", "steps"], &env, &ctx),
            run("'(job normal sleeping 1)", &env, &ctx)
        );
        wait(&ctx, ms(1000));
        assert_eq!(
            entry("counter", &["steps"], &env, &ctx),
            run("'(2)", &env, &ctx),
            "brought up to date when the scheduler runs out of work"
        );
        let times = entry("counter", &["total-ms", "max-ms"], &env, &ctx);
        assert!(
            times.iter().all(|time| matches!(time, LispExp::Number(_))),
            "{times:?}"
        );
    }

    #[test]
    fn running_workers_is_the_part_of_work_list_lisp_started() {
        let (ctx, env) = editor();
        run(
            "(define-worker 'mine (fiber (while t (yield))))
             (background-call 'also-mine (fiber (while t (yield))) nil nil)",
            &env,
            &ctx,
        );
        assert_eq!(
            run("(running-workers)", &env, &ctx),
            run(r#"'("also-mine" "mine")"#, &env, &ctx)
        );
        assert!(!entry("highlighter", &["kind"], &env, &ctx).is_nil());
        assert!(!entry("mine", &["kind"], &env, &ctx).is_nil());
    }

    #[test]
    fn stop_worker_stops_the_editor_s_own_work_too() {
        // One namespace: a name means one thing, whoever started it.
        let (ctx, env) = editor();
        assert_eq!(run("(stop-worker 'auto-saver)", &env, &ctx), LispExp::t());
        ctx.work().run_until_idle(&ctx);
        assert!(entry("auto-saver", &["kind"], &env, &ctx).is_nil());
    }

    // ----------------------------------------------------------------
    // The fuel a step spent
    // ----------------------------------------------------------------

    #[test]
    fn the_fuel_left_is_read_exactly() {
        // risp offers no reading of it, so it is read by spending it; this is
        // what says the reading is exact, to the unit, at the edges too.
        for amount in [0, 1, 2, 3, 1023, 1024, 200_000, u32::MAX - 1, u32::MAX] {
            risp::set_remaining(amount);
            assert_eq!(fuel_left(), amount);
            assert_eq!(fuel_left(), 0, "reading it spends it");
        }
        risp::set_remaining(risp::DEFAULT_FUEL);
    }
}
