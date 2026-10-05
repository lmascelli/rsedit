//! Background jobs: where their callbacks run, and what is dropped.
//!
//! # What is worth testing here, and what cannot be
//!
//! The same shape as `worker_tests`, for the same reason: a job runs on the
//! scheduler's thread on a timer, which is what a test cannot wait on without
//! being a race. So most of these drive the turns the scheduler would drive --
//! `ScheduledTask::execute`, one call per turn -- and then drain the callback
//! queue the way the event loop drains it.
//!
//! What that arrangement is *for* is the thing worth pinning down, because
//! every part of it is silent when wrong:
//!
//! - A callback run on the worker thread looks exactly like one run on the
//!   command thread, until the day it runs half-way through a command and
//!   corrupts something once a month.
//! - A superseded job's callback looks exactly like the new job's, until it
//!   reports the old job's half-built state as the new one's answer.
//! - A job that dies without saying so looks exactly like one still working,
//!   for ever.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, LispExp, Parser, eval};
    use crate::task::{ImmediateTask, ScheduledTask, WorkerMessage};
    use crate::worker::{BackgroundJob, DEFAULT_WORKER_FUEL, JobBody};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env")
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn number(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> f64 {
        match run(src, env, ctx) {
            LispExp::Number(n) => n,
            other => panic!("expected a number from {src}, got {other:?}"),
        }
    }

    /// A job built the way `background-call` builds one, handed back so a test
    /// can take its turns itself instead of waiting for the scheduler.
    fn job(
        name: &str,
        body: JobBody<GapBuffer>,
        on_progress: Option<&str>,
        on_done: Option<&str>,
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
    ) -> BackgroundJob<GapBuffer> {
        let generation = ctx.runtime_mut(|runtime| runtime.next_worker_generation(name));
        BackgroundJob::new(
            name.to_string(),
            generation,
            body,
            on_progress.map(|f| LispExp::symbol(f.into())),
            on_done.map(|f| LispExp::symbol(f.into())),
            env.clone(),
            DEFAULT_WORKER_FUEL,
        )
    }

    fn once(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> JobBody<GapBuffer> {
        JobBody::Once(run(src, env, ctx))
    }

    fn steps(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> JobBody<GapBuffer> {
        JobBody::Steps(run(src, env, ctx))
    }

    /// A fiber that takes three turns and then finishes.
    ///
    /// A `while` walking a counter rather than the `dotimes` anyone would
    /// reach for first, because a `dotimes` body is not a position a
    /// `(yield)` can be recorded in -- the evaluator refuses one there. It is
    /// worth a test writing it out the long way, since it is what every fiber
    /// in the tree has to do.
    const COUNTING_FIBER: &str =
        "(fiber (let ((left 3)) (while (> left 0) (setq left (- left 1)) (yield))))";

    /// Run turns until the job says it is finished, or fail. The scheduler's
    /// loop, minus the waiting.
    fn take_turns(job: &mut BackgroundJob<GapBuffer>, ctx: &Ctx) -> usize {
        for turn in 1..=1000 {
            if !job.execute(ctx) {
                return turn;
            }
        }
        panic!("the job never finished");
    }

    // ----------------------------------------------------------------
    // Which thread the callback runs on
    // ----------------------------------------------------------------

    #[test]
    fn the_job_runs_where_it_is_driven_and_the_callback_does_not() {
        // The division the whole arrangement rests on. The job's own work
        // happens during the turn; the callback happens when the command
        // thread gets round to it, and *not before* -- which is the part that
        // cannot be seen by looking at the result afterwards, because by then
        // both have happened.
        let (ctx, env) = editor();
        run(
            "(setq worked nil) (setq told nil)
             (defun the-work () (setq worked t))
             (defun the-callback (name finished) (setq told name))",
            &env,
            &ctx,
        );
        let mut job = job(
            "reporting",
            once("'the-work", &env, &ctx),
            None,
            Some("the-callback"),
            &env,
            &ctx,
        );
        job.execute(&ctx);
        assert!(
            !run("worked", &env, &ctx).is_nil(),
            "the turn should have done the work"
        );
        assert!(
            run("told", &env, &ctx).is_nil(),
            "but the callback must wait for the thread that runs commands"
        );
        assert!(ctx.callbacks_owed(), "and the editor must know it is owed");

        assert!(ctx.run_owed_callbacks(&env), "draining runs it");
        assert_eq!(
            run("told", &env, &ctx),
            LispExp::string("reporting".into()),
            "and it is given the name the job ran under"
        );
        assert!(!ctx.callbacks_owed(), "the queue is empty afterwards");
    }

    #[test]
    fn a_callback_is_given_the_name_and_whether_the_job_reached_its_end() {
        let (ctx, env) = editor();
        run(
            "(setq outcome 'unset)
             (defun the-work () 1)
             (defun the-callback (name finished) (setq outcome finished))",
            &env,
            &ctx,
        );
        let mut job = job(
            "finishing",
            once("'the-work", &env, &ctx),
            None,
            Some("the-callback"),
            &env,
            &ctx,
        );
        take_turns(&mut job, &ctx);
        ctx.run_owed_callbacks(&env);
        assert!(
            !run("outcome", &env, &ctx).is_nil(),
            "a job that returned reached its end"
        );
    }

    #[test]
    fn a_job_that_fails_still_says_it_is_done() {
        // Rule six. A mode that is told nothing stays waiting for ever, and
        // "the index is still building" is a state nothing else can leave.
        let (ctx, env) = editor();
        run(
            "(setq outcome 'unset)
             (defun the-work () (this-function-does-not-exist))
             (defun the-callback (name finished) (setq outcome (list name finished)))",
            &env,
            &ctx,
        );
        let mut job = job(
            "failing",
            once("'the-work", &env, &ctx),
            None,
            Some("the-callback"),
            &env,
            &ctx,
        );
        take_turns(&mut job, &ctx);
        ctx.run_owed_callbacks(&env);
        let outcome = run("outcome", &env, &ctx);
        let parts: Vec<LispExp<Ctx>> = outcome.iter().collect();
        assert_eq!(
            parts.first(),
            Some(&LispExp::string("failing".into())),
            "it is told, {outcome:?}"
        );
        assert!(
            parts.get(1).is_some_and(|finished| finished.is_nil()),
            "and told that the job did not reach its end: {outcome:?}"
        );
    }

    // ----------------------------------------------------------------
    // Progress
    // ----------------------------------------------------------------

    #[test]
    fn every_yield_but_the_last_is_a_progress_point() {
        // What makes a fiber the streaming shape: the job says where it can be
        // put down, and that is also where it has something to report. There
        // is nothing else to write and nothing else to register.
        let (ctx, env) = editor();
        run(
            "(setq progress 0) (setq done 0)
             (defun on-progress (name) (setq progress (+ progress 1)))
             (defun on-done (name finished) (setq done (+ done 1)))",
            &env,
            &ctx,
        );
        let mut job = job(
            "streaming",
            steps(COUNTING_FIBER, &env, &ctx),
            Some("on-progress"),
            Some("on-done"),
            &env,
            &ctx,
        );
        let turns = take_turns(&mut job, &ctx);
        assert_eq!(
            turns, 4,
            "three yields and the turn that found the fiber done"
        );
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            number("progress", &env, &ctx),
            (turns - 1) as f64,
            "one per turn except the one that finished ({turns} turns)"
        );
        assert_eq!(number("done", &env, &ctx), 1.0, "and done exactly once");
    }

    #[test]
    fn a_job_that_is_one_function_has_no_progress_to_report() {
        let (ctx, env) = editor();
        run(
            "(setq progress 0)
             (defun the-work () 1)
             (defun on-progress (name) (setq progress (+ progress 1)))",
            &env,
            &ctx,
        );
        let mut job = job(
            "one-shot",
            once("'the-work", &env, &ctx),
            Some("on-progress"),
            None,
            &env,
            &ctx,
        );
        take_turns(&mut job, &ctx);
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            number("progress", &env, &ctx),
            0.0,
            "there is nowhere in a plain function for progress to be"
        );
    }

    // ----------------------------------------------------------------
    // Being superseded
    // ----------------------------------------------------------------

    #[test]
    fn a_superseded_job_stops_and_its_queued_callbacks_are_dropped() {
        // The failure this prevents is the quiet one: the replaced job's
        // callback arrives after the replacement has started, and tells the
        // mode that the work it is now waiting for has finished -- with the
        // *previous* job's half-built state as the evidence.
        let (ctx, env) = editor();
        run(
            "(setq heard nil)
             (defun on-progress (name) (setq heard (cons 'progress heard)))
             (defun on-done (name finished) (setq heard (cons 'done heard)))",
            &env,
            &ctx,
        );
        let mut first = job(
            "shared-name",
            steps(COUNTING_FIBER, &env, &ctx),
            Some("on-progress"),
            Some("on-done"),
            &env,
            &ctx,
        );
        assert!(first.execute(&ctx), "one turn, with more to come");
        assert!(ctx.callbacks_owed(), "which queued its progress");

        // The replacement, started before the queue was drained -- which is
        // the ordinary case, since the drain happens between commands and the
        // replacement is started by one.
        let _second = job(
            "shared-name",
            steps("(fiber (while t (yield)))", &env, &ctx),
            Some("on-progress"),
            Some("on-done"),
            &env,
            &ctx,
        );
        assert!(
            !ctx.run_owed_callbacks(&env),
            "the first job's queued progress is dropped rather than delivered"
        );
        assert!(
            run("heard", &env, &ctx).is_nil(),
            "so nothing heard from it"
        );
        assert!(
            !first.execute(&ctx),
            "and the job itself stops at its next turn"
        );
        ctx.run_owed_callbacks(&env);
        assert!(
            run("heard", &env, &ctx).is_nil(),
            "including its done, which belongs to the job that replaced it"
        );
    }

    #[test]
    fn a_finished_job_is_still_entitled_to_report() {
        // The distinction this rests on, stated as a test because getting it
        // wrong is invisible: a job that has stopped is not a job that has
        // been replaced. Asking "is it still running" at the drain would
        // silence every done callback there is, since a job queues its done
        // and then stops.
        let (ctx, env) = editor();
        run(
            "(setq told nil)
             (defun on-done (name finished) (setq told t))",
            &env,
            &ctx,
        );
        let mut job = job(
            "brief",
            once("(lambda () 1)", &env, &ctx),
            None,
            Some("on-done"),
            &env,
            &ctx,
        );
        take_turns(&mut job, &ctx);
        assert_eq!(
            ctx.runtime(|runtime| runtime.running_workers()).len(),
            0,
            "it has retired itself"
        );
        ctx.run_owed_callbacks(&env);
        assert!(
            !run("told", &env, &ctx).is_nil(),
            "and its report is delivered anyway"
        );
    }

    // ----------------------------------------------------------------
    // The queue, and the renderer
    // ----------------------------------------------------------------

    #[test]
    fn the_renderer_is_asked_to_come_back_while_a_callback_is_owed() {
        // The window this closes: a job queues its last callback and then
        // stops, taking the count of running work to zero. A renderer that
        // asked in that instant would be told there was nothing to wait for
        // and would sleep until a key was pressed, with the callback still
        // sitting there.
        let (ctx, env) = editor();
        run(
            "(defun the-work () 1) (defun on-done (name finished) nil)",
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, 80, 24);
        // Not `None`: a newly started editor has a message in the echo area
        // and is already waiting for it to expire. What matters is that it is
        // willing to wait, rather than having something to do now.
        assert_ne!(
            ctx.next_redraw_in(&env, &frame),
            Some(Duration::ZERO),
            "nothing is owed yet, so the editor may sleep"
        );
        let mut job = job(
            "waking",
            once("'the-work", &env, &ctx),
            None,
            Some("on-done"),
            &env,
            &ctx,
        );
        take_turns(&mut job, &ctx);
        assert_eq!(
            ctx.background_work_running(),
            0,
            "nothing is running any more"
        );
        assert_eq!(
            ctx.next_redraw_in(&env, &frame),
            Some(Duration::ZERO),
            "but there is something to do, so the wait is over"
        );
        ctx.run_owed_callbacks(&env);
        assert_ne!(
            ctx.next_redraw_in(&env, &frame),
            Some(Duration::ZERO),
            "and once it is done there is nothing to come back for"
        );
    }

    #[test]
    fn a_callback_may_start_another_job_and_it_goes_to_the_worker_like_any_other() {
        // Chaining two jobs is the ordinary way of doing work in stages, and
        // the thing worth checking is that the second stage does not simply
        // happen here: a callback runs on the thread that draws, and a stage
        // that ran inside one would be the whole point of this thrown away.
        let (ctx, env) = editor();
        run(
            "(setq stages nil)
             (defun stage-two () (setq stages (cons 'two stages)))
             (defun stage-one () (setq stages (cons 'one stages)))
             (defun after-one (name finished)
               (background-call 'chained-two 'stage-two nil nil))",
            &env,
            &ctx,
        );
        let mut first = job(
            "chained-one",
            once("'stage-one", &env, &ctx),
            None,
            Some("after-one"),
            &env,
            &ctx,
        );
        take_turns(&mut first, &ctx);
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            run("stages", &env, &ctx).iter().count(),
            1,
            "the second stage was posted, not run here"
        );
        assert!(
            ctx.runtime(|runtime| runtime.running_workers())
                .contains(&"chained-two".to_string()),
            "and it is running under its own name"
        );
    }

    #[test]
    fn a_callback_that_fails_is_logged_and_the_rest_still_run() {
        let (ctx, env) = editor();
        run(
            "(setq reached nil)
             (defun bad-callback (name finished) (no-such-function))
             (defun good-callback (name finished) (setq reached t))",
            &env,
            &ctx,
        );
        let mut bad = job(
            "bad",
            once("(lambda () 1)", &env, &ctx),
            None,
            Some("bad-callback"),
            &env,
            &ctx,
        );
        let mut good = job(
            "good",
            once("(lambda () 1)", &env, &ctx),
            None,
            Some("good-callback"),
            &env,
            &ctx,
        );
        take_turns(&mut bad, &ctx);
        take_turns(&mut good, &ctx);
        ctx.run_owed_callbacks(&env);
        assert!(
            !run("reached", &env, &ctx).is_nil(),
            "one broken callback does not take down the ones behind it"
        );
    }

    // ----------------------------------------------------------------
    // The door Lisp comes through
    // ----------------------------------------------------------------

    #[test]
    fn starting_a_job_names_it_and_listing_finds_it() {
        let (ctx, env) = editor();
        run("(defun slow-work () (fiber (while t (yield))))", &env, &ctx);
        assert_eq!(
            run("(background-call 'named (slow-work) nil nil)", &env, &ctx),
            LispExp::symbol("named".into()),
            "it answers with the name, the way define-worker does"
        );
        assert!(
            ctx.runtime(|runtime| runtime.running_workers())
                .contains(&"named".to_string()),
            "and a job and a worker are listed in one place"
        );
        assert!(
            !run("(stop-worker 'named)", &env, &ctx).is_nil(),
            "and stopped by one call"
        );
    }

    #[test]
    fn a_job_that_is_neither_a_fiber_nor_a_function_is_refused_at_the_door() {
        // Refused here rather than discovered a turn later on another thread,
        // where the only way to report it is a line in a log nobody is
        // reading.
        let (ctx, env) = editor();
        for bad in ["(background-call 'n 3 nil nil)", "(background-call 'n nil)"] {
            let ast = Parser::new(bad).next().expect("parses");
            assert!(
                eval(&ast, env.clone(), &ctx).is_err(),
                "{bad} should not be accepted"
            );
        }
        for bad in [
            "(background-call 'n (lambda () 1) 3)",
            "(background-call 'n (lambda () 1) nil \"not a function\")",
        ] {
            let ast = Parser::new(bad).next().expect("parses");
            assert!(
                eval(&ast, env.clone(), &ctx).is_err(),
                "{bad} names a callback that could never be called"
            );
        }
    }

    #[test]
    fn a_job_posted_for_real_reaches_the_worker_and_comes_back() {
        // The one test that does wait, because the thing it is checking is
        // the plumbing between the two threads and there is nothing else to
        // check it with: the primitive really posts, the scheduler really
        // takes turns, and the callback really arrives on this thread.
        let (ctx, env) = editor();
        run(
            "(setq gathered nil) (setq finished nil)
             (defun gather ()
               (fiber (let ((left 3))
                        (while (> left 0)
                          (setq gathered (cons left gathered))
                          (setq left (- left 1))
                          (yield)))))
             (defun gather-progress (name) nil)
             (defun gather-done (name finished-p) (setq finished t))",
            &env,
            &ctx,
        );
        run(
            "(background-call 'real (gather) 'gather-progress 'gather-done 1)",
            &env,
            &ctx,
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while run("finished", &env, &ctx).is_nil() {
            assert!(Instant::now() < deadline, "the job never reported back");
            ctx.run_owed_callbacks(&env);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            run("gathered", &env, &ctx).iter().count(),
            3,
            "and it did its work a turn at a time"
        );
    }

    #[test]
    fn a_job_cannot_open_a_prompt() {
        // The same refusal a worker gets, for the same reason and through the
        // same flag: a minibuffer put up by a job is a window nobody is
        // typing into, over a command nobody started. Worth its own test
        // rather than resting on the worker's, because the two set the flag
        // in two places and a job is the one a module is likely to reach for.
        let (ctx, env) = editor();
        run(
            "(setq asked nil) (setq outcome 'unset)
             (defun nosy () (minibuffer-read \"Why?:\" nil nil nil) (setq asked t))
             (defun on-done (name finished) (setq outcome finished))",
            &env,
            &ctx,
        );
        let mut job = job(
            "nosy",
            once("'nosy", &env, &ctx),
            None,
            Some("on-done"),
            &env,
            &ctx,
        );
        take_turns(&mut job, &ctx);
        assert!(run("asked", &env, &ctx).is_nil(), "the prompt was refused");
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("cannot open a prompt")),
            "and it says why: {:?}",
            ctx.get_logs()
        );
        ctx.run_owed_callbacks(&env);
        assert!(
            run("outcome", &env, &ctx).is_nil(),
            "and the job reports that it did not reach its end"
        );
    }

    // ----------------------------------------------------------------
    // Which thread a task believes it is on
    // ----------------------------------------------------------------

    /// An `ImmediateTask` that does nothing but report where it woke up.
    struct WhereAmI(Arc<Mutex<Option<bool>>>);

    impl ImmediateTask<GapBuffer> for WhereAmI {
        fn execute(self: Box<Self>, _state: &Ctx) {
            *self.0.lock().expect("the probe's slot") = Some(crate::worker::in_worker());
        }
    }

    #[test]
    fn a_task_on_its_own_thread_still_knows_it_is_not_the_command_thread() {
        // The flag is what `minibuffer-read' consults before refusing to put
        // up a prompt nobody is typing into, and a task with a thread of its
        // own is further from the command thread than anything else here --
        // so it is the case most in need of the flag and the one most easily
        // left out, now that the scheduler sets it rather than each task.
        let (ctx, _env) = editor();
        let answer = Arc::new(Mutex::new(None));
        assert!(
            ctx.send_to_worker(WorkerMessage::RunNow(Box::new(WhereAmI(answer.clone())))),
            "the scheduler took it"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(seen) = *answer.lock().expect("the probe's slot") {
                assert!(seen, "a task must not believe it is the command thread");
                return;
            }
            assert!(Instant::now() < deadline, "the task never ran");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The same probe, as a scheduled task that never sets the flag itself --
    /// which is what the highlighter, the prescanner, the file watcher and
    /// the auto-saver all are.
    struct WhereAmIEachTurn(Arc<Mutex<Option<bool>>>);

    impl ScheduledTask<GapBuffer> for WhereAmIEachTurn {
        fn execute(&mut self, _state: &Ctx) -> bool {
            *self.0.lock().expect("the probe's slot") = Some(crate::worker::in_worker());
            false
        }
    }

    #[test]
    fn a_scheduled_task_that_never_says_so_itself_is_still_off_the_command_thread() {
        // Two of the four scheduled tasks the editor ships -- the file
        // watcher and the auto-saver -- hold an environment and run Lisp, and
        // neither ever set this flag. A hook of theirs calling
        // `minibuffer-read' would therefore have put up a prompt from the
        // background, which is precisely what the flag exists to refuse. The
        // scheduler saying it once covers every task, including the ones that
        // do not know the flag exists.
        let (ctx, _env) = editor();
        let answer = Arc::new(Mutex::new(None));
        assert!(ctx.send_to_worker(WorkerMessage::Schedule {
            task: Box::new(WhereAmIEachTurn(answer.clone())),
            interval: Duration::from_millis(1),
        }));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(seen) = *answer.lock().expect("the probe's slot") {
                assert!(seen, "a scheduled task is not on the command thread either");
                return;
            }
            assert!(Instant::now() < deadline, "the task never ran");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn an_inner_turn_ending_does_not_make_the_thread_the_command_thread() {
        // Two hold one at once and always will: the scheduler around
        // everything it drives, and the job around its own turn. If the inner
        // one cleared the flag instead of restoring it, everything the
        // scheduler ran *after* the first job's first turn would believe it
        // was the thread the user is typing on -- and the one thing the flag
        // guards, a prompt appearing out of nowhere, is exactly the thing
        // nobody would think to look for there.
        assert!(!crate::worker::in_worker(), "this thread is the user's");
        let outer = crate::worker::WorkerTurn::begin();
        {
            let _inner = crate::worker::WorkerTurn::begin();
            assert!(crate::worker::in_worker());
        }
        assert!(
            crate::worker::in_worker(),
            "the outer turn has not ended, so the thread has not changed"
        );
        drop(outer);
        assert!(!crate::worker::in_worker(), "and now it has");
    }

    #[test]
    fn the_command_thread_knows_it_is_the_command_thread() {
        // The other half, and the one that would fail if the flag were ever
        // left set: a task's thread setting it must not be visible here.
        let (ctx, _env) = editor();
        let answer = Arc::new(Mutex::new(None));
        ctx.send_to_worker(WorkerMessage::RunNow(Box::new(WhereAmI(answer.clone()))));
        let deadline = Instant::now() + Duration::from_secs(10);
        while answer.lock().expect("the probe's slot").is_none() {
            assert!(Instant::now() < deadline, "the task never ran");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!crate::worker::in_worker(), "this thread is the user's");
    }

    // ----------------------------------------------------------------
    // Saying the candidates changed
    // ----------------------------------------------------------------

    #[test]
    fn invalidating_completions_forgets_the_answer_and_tells_the_presenter() {
        let (ctx, env) = editor();
        run(
            "(setq *minibuffer-completions-for* \"abc\")
             (setq told 0)
             (defun was-told () (setq told (+ told 1)))
             (setq *completion-invalidated-function* 'was-told)
             (completion-invalidate)",
            &env,
            &ctx,
        );
        assert!(
            run("*minibuffer-completions-for*", &env, &ctx).is_nil(),
            "the next Tab has to ask again rather than repeat a short answer"
        );
        assert_eq!(number("told", &env, &ctx), 1.0, "and the strip redraws");
    }

    #[test]
    fn invalidating_completions_is_safe_with_nobody_listening() {
        // Which is the state it is in for most of a session, and on a machine
        // where completion.lisp was never loaded it is the only state.
        let (ctx, env) = editor();
        run("(completion-invalidate)", &env, &ctx);
    }
}
