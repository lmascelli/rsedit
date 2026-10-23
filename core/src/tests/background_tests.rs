//! Background jobs: where their callbacks run, what is dropped, and how the
//! host hears that the screen changed.
//!
//! # How these run
//!
//! As in `worker_tests`: the editor's WorkManager is in manual mode, so a job
//! started with `background-call` takes its steps when the test drives it,
//! and then the test drains the callback queue the way the event loop drains
//! it.
//!
//! What that arrangement is *for* is the thing worth pinning down, because
//! every part of it is silent when wrong:
//!
//! - A callback run on the executor looks exactly like one run on the
//!   command thread, until the day it runs half-way through a command and
//!   corrupts something once a month.
//! - A superseded job's callback looks exactly like the new job's, until it
//!   reports the old job's half-built state as the new one's answer.
//! - A job that dies without saying so looks exactly like one still working,
//!   for ever.
//! - A change nobody tells the renderer about looks exactly like no change,
//!   until a key is pressed.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::modes::highlighter::TURN_INTERVAL;
    use crate::tests::work_support::{PATIENCE, Steps, exits, scripted, wakes};
    use crate::work::{Exit, Spec, Step};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    /// Take the steps that are due now.
    fn steps_due(ctx: &Ctx) -> usize {
        ctx.work().run_until_idle(ctx)
    }

    /// Let SPAN of the editor's time pass, taking every step that falls due.
    fn wait(ctx: &Ctx, span: Duration) -> usize {
        ctx.work().run_for(ctx, span)
    }

    fn running(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<String> {
        run("(running-workers)", env, ctx)
            .iter()
            .map(|name| match name {
                LispExp::String(name) => name.to_string(),
                other => panic!("a name, not {other:?}"),
            })
            .collect()
    }

    /// A fiber that takes three steps and then finishes.
    ///
    /// A `while` walking a counter rather than the `dotimes` anyone would
    /// reach for first, because a `dotimes` body is not a position a
    /// `(yield)` can be recorded in -- the evaluator refuses one there. It is
    /// worth a test writing it out the long way, since it is what every fiber
    /// in the tree has to do.
    const COUNTING_FIBER: &str =
        "(fiber (let ((left 3)) (while (> left 0) (setq left (- left 1)) (yield))))";

    // ----------------------------------------------------------------
    // The editor's WorkManager in a test
    // ----------------------------------------------------------------

    #[test]
    fn the_editor_runs_its_work_by_hand_in_tests_and_logs_its_reports() {
        let (editor, _env) = create_global_env::<GapBuffer>().expect("global env");
        let (job, trace) = scripted(&[Step::Continue]);
        editor.work().submit(Spec::new("probe"), job).unwrap();
        assert!(wakes(&trace).is_empty(), "nothing runs by itself in a test");
        editor.work().run_until_idle(&editor);
        assert_eq!(exits(&trace), [Exit::Finished]);

        let boom = Steps(|| -> Step { panic!("kaboom") });
        editor
            .work()
            .submit(Spec::new("boom"), Box::new(boom))
            .unwrap();
        editor.work().run_until_idle(&editor);
        assert!(
            editor
                .get_logs()
                .iter()
                .any(|line| line.contains("[BKG] panic: boom retired: kaboom")),
            "the editor's log has the report"
        );

        editor.shutdown(PATIENCE);
        let (late, _) = scripted(&[]);
        assert_eq!(editor.work().submit(Spec::new("late"), late), None);
    }

    // ----------------------------------------------------------------
    // Which thread the callback runs on
    // ----------------------------------------------------------------

    #[test]
    fn the_job_runs_where_it_is_driven_and_the_callback_does_not() {
        // The division the whole arrangement rests on. The job's own work
        // happens during the step; the callback happens when the command
        // thread gets round to it, and *not before* -- which is the part that
        // cannot be seen by looking at the result afterwards, because by then
        // both have happened.
        let (ctx, env) = editor();
        run(
            "(setq worked nil) (setq told nil)
             (defun the-work () (setq worked t))
             (defun the-callback (name finished) (setq told name))
             (background-call 'reporting 'the-work nil 'the-callback)",
            &env,
            &ctx,
        );
        assert!(run("worked", &env, &ctx).is_nil(), "nothing has run yet");
        steps_due(&ctx);
        assert!(
            !run("worked", &env, &ctx).is_nil(),
            "the step should have done the work"
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
             (defun the-callback (name finished) (setq outcome finished))
             (background-call 'finishing 'the-work nil 'the-callback)",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            run("outcome", &env, &ctx),
            LispExp::t(),
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
             (defun the-callback (name finished) (setq outcome (list name finished)))
             (background-call 'failing 'the-work nil 'the-callback)",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            run("outcome", &env, &ctx),
            run(r#"(list "failing" nil)"#, &env, &ctx),
            "it is told, and told that the job did not reach its end"
        );
    }

    #[test]
    fn a_job_that_spends_its_allowance_says_it_did_not_finish() {
        let (ctx, env) = editor();
        run(
            "(setq outcome 'unset) (setq worker-fuel 5000)
             (defun the-callback (name finished) (setq outcome (list name finished)))
             (background-call 'endless (fiber (while t (setq x 1))) nil 'the-callback)",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            run("outcome", &env, &ctx),
            run(r#"(list "endless" nil)"#, &env, &ctx)
        );
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("[BKG] error: endless: a step ran past its allowance")),
            "{:?}",
            ctx.get_logs()
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
            &format!(
                "(setq progress 0) (setq done 0)
                 (defun on-progress (name) (setq progress (+ progress 1)))
                 (defun on-done (name finished) (setq done (+ done 1)))
                 (background-call 'streaming {COUNTING_FIBER} 'on-progress 'on-done)"
            ),
            &env,
            &ctx,
        );
        steps_due(&ctx);
        wait(&ctx, Duration::from_secs(1));
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            number("progress", &env, &ctx),
            3.0,
            "three yields, and then the step that found the fiber done"
        );
        assert_eq!(number("done", &env, &ctx), 1.0, "and done exactly once");
    }

    #[test]
    fn a_job_that_is_one_function_has_no_progress_to_report() {
        let (ctx, env) = editor();
        run(
            "(setq progress 0)
             (defun the-work () 1)
             (defun on-progress (name) (setq progress (+ progress 1)))
             (background-call 'one-shot 'the-work 'on-progress nil)",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            number("progress", &env, &ctx),
            0.0,
            "there is nowhere in a plain function for progress to be"
        );
    }

    // ----------------------------------------------------------------
    // Being superseded, and being stopped
    // ----------------------------------------------------------------

    #[test]
    fn a_superseded_job_stops_and_its_queued_callbacks_are_dropped() {
        // The failure this prevents is the quiet one: the replaced job's
        // callback arrives after the replacement has started, and tells the
        // mode that the work it is now waiting for has finished -- with the
        // *previous* job's half-built state as the evidence.
        let (ctx, env) = editor();
        run(
            &format!(
                "(setq heard nil)
                 (defun on-progress (name) (setq heard (cons 'progress heard)))
                 (defun on-done (name finished) (setq heard (cons 'done heard)))
                 (background-call 'shared-name {COUNTING_FIBER} 'on-progress 'on-done)"
            ),
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert!(ctx.callbacks_owed(), "one step, which queued its progress");

        // The replacement, started before the queue was drained -- which is
        // the ordinary case, since the drain happens between commands and the
        // replacement is started by one. It reports nothing of its own, so
        // that anything heard is the first job's.
        run(
            "(background-call 'shared-name (fiber (while t (yield))) nil nil)",
            &env,
            &ctx,
        );
        assert!(
            !ctx.run_owed_callbacks(&env),
            "the first job's queued progress is dropped rather than delivered"
        );
        wait(&ctx, Duration::from_secs(1));
        ctx.run_owed_callbacks(&env);
        assert!(
            run("heard", &env, &ctx).is_nil(),
            "and it took no more steps, and did not say it was done"
        );
        assert_eq!(running(&ctx, &env), ["shared-name"], "the replacement runs");
    }

    #[test]
    fn a_stopped_job_is_not_heard_from_again() {
        // `stop-worker` is the other way a job is silenced, and the same
        // rule holds: whoever stopped it knows, and needs no ON-DONE.
        let (ctx, env) = editor();
        run(
            &format!(
                "(setq heard nil)
                 (defun on-progress (name) (setq heard (cons 'progress heard)))
                 (defun on-done (name finished) (setq heard (cons 'done heard)))
                 (background-call 'stoppable {COUNTING_FIBER} 'on-progress 'on-done)"
            ),
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert!(ctx.callbacks_owed(), "one step, which queued its progress");
        assert_eq!(run("(stop-worker 'stoppable)", &env, &ctx), LispExp::t());
        assert!(
            !ctx.run_owed_callbacks(&env),
            "its queued progress went with it, at once"
        );
        wait(&ctx, Duration::from_secs(1));
        assert!(
            !ctx.callbacks_owed(),
            "and when it heard, it left without queueing an ON-DONE at all"
        );
        assert!(run("heard", &env, &ctx).is_nil());
    }

    #[test]
    fn a_job_closed_by_shutdown_is_not_reported_done() {
        // The editor is going: nobody is left to tell, and a mode told that
        // its index finished when it was cut off would believe it.
        let (ctx, env) = editor();
        run(
            &format!(
                "(defun on-done (name finished) nil)
                 (background-call 'cut-off {COUNTING_FIBER} nil 'on-done)"
            ),
            &env,
            &ctx,
        );
        steps_due(&ctx);
        ctx.shutdown(Duration::from_secs(1));
        assert!(!ctx.callbacks_owed());
        assert!(running(&ctx, &env).is_empty());
    }

    #[test]
    fn a_finished_job_is_still_entitled_to_report() {
        // The distinction this rests on, stated as a test because getting it
        // wrong is invisible: a job that has stopped is not a job that has
        // been replaced. Asking "is it still running" at the drain would
        // silence every done callback there is, since a job queues its done
        // and then leaves.
        let (ctx, env) = editor();
        run(
            "(setq told nil)
             (defun on-done (name finished) (setq told t))
             (background-call 'brief (lambda () 1) nil 'on-done)",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert!(running(&ctx, &env).is_empty(), "it has finished");
        assert!(
            run("(stop-worker 'brief)", &env, &ctx).is_nil(),
            "and there is nothing left to stop"
        );
        ctx.run_owed_callbacks(&env);
        assert!(
            !run("told", &env, &ctx).is_nil(),
            "and its report is delivered anyway"
        );
    }

    #[test]
    fn a_callback_may_start_another_job_and_it_goes_to_the_work_like_any_other() {
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
               (background-call 'chained-two 'stage-two nil nil))
             (background-call 'chained-one 'stage-one nil 'after-one)",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        ctx.run_owed_callbacks(&env);
        assert_eq!(
            run("stages", &env, &ctx).iter().count(),
            1,
            "the second stage was submitted, not run here"
        );
        assert_eq!(
            running(&ctx, &env),
            ["chained-two"],
            "and it is running under its own name"
        );
        steps_due(&ctx);
        assert_eq!(run("stages", &env, &ctx).iter().count(), 2);
    }

    #[test]
    fn a_callback_that_fails_is_logged_and_the_rest_still_run() {
        let (ctx, env) = editor();
        run(
            "(setq reached nil)
             (defun bad-callback (name finished) (no-such-function))
             (defun good-callback (name finished) (setq reached t))
             (background-call 'bad (lambda () 1) nil 'bad-callback)
             (background-call 'good (lambda () 1) nil 'good-callback)",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        ctx.run_owed_callbacks(&env);
        assert!(
            !run("reached", &env, &ctx).is_nil(),
            "one broken callback does not take down the ones behind it"
        );
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("the callback of background job bad failed")),
            "{:?}",
            ctx.get_logs()
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
        run(
            "(define-worker 'worker (fiber (while t (yield))))",
            &env,
            &ctx,
        );
        assert_eq!(
            running(&ctx, &env),
            ["named", "worker"],
            "a job and a worker are listed in one place"
        );
        assert!(
            !run("(stop-worker 'named)", &env, &ctx).is_nil(),
            "and stopped by one call"
        );
        assert_eq!(running(&ctx, &env), ["worker"]);
    }

    #[test]
    fn a_job_that_is_neither_a_fiber_nor_a_function_is_refused_at_the_door() {
        // Refused here rather than discovered a step later on another thread,
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
        assert!(running(&ctx, &env).is_empty());
    }

    #[test]
    fn a_job_cannot_open_a_prompt() {
        // The same refusal a worker gets, for the same reason and through the
        // same flag: a minibuffer put up by a job is a window nobody is
        // typing into, over a command nobody started.
        let (ctx, env) = editor();
        run(
            "(setq asked nil) (setq outcome 'unset)
             (defun nosy () (minibuffer-read \"Why?:\" nil nil nil) (setq asked t))
             (defun on-done (name finished) (setq outcome finished))
             (background-call 'nosy 'nosy nil 'on-done)",
            &env,
            &ctx,
        );
        steps_due(&ctx);
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
    // Telling the host the screen changed
    // ----------------------------------------------------------------

    /// An editor whose host counts how often it is told to look.
    fn watched() -> (Ctx, Arc<Env<Ctx>>, Arc<AtomicUsize>) {
        let (ctx, env) = editor();
        let told = Arc::new(AtomicUsize::new(0));
        ctx.on_background_change({
            let told = told.clone();
            move || {
                told.fetch_add(1, Ordering::SeqCst);
            }
        });
        (ctx, env, told)
    }

    /// Whether the editor wants a frame for what the jobs changed: soon, at
    /// most a turn away. Its other reasons to wake -- the welcome message
    /// expiring -- are seconds away.
    fn wants_a_frame_soon(ctx: &Ctx, env: &Arc<Env<Ctx>>, frame: &crate::FrameSnapshot) -> bool {
        ctx.next_redraw_in(env, frame)
            .is_some_and(|wait| wait <= TURN_INTERVAL)
    }

    #[test]
    fn a_step_tells_the_host_once_until_it_looks_again() {
        let (ctx, env, told) = watched();
        // The editor's own jobs take their first steps, and the snapshot
        // takes in whatever they did.
        steps_due(&ctx);
        let frame = ctx.snapshot(&env, 80, 24);
        let before = told.load(Ordering::SeqCst);
        assert!(
            !wants_a_frame_soon(&ctx, &env, &frame),
            "nothing since the frame"
        );

        run(
            "(setq counter 0)
             (define-worker 'busy
               (fiber (let ((left 5))
                        (while (> left 0)
                          (setq counter (+ counter 1))
                          (setq left (- left 1))
                          (yield 'continue)))))",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert_eq!(number("counter", &env, &ctx), 5.0);
        assert_eq!(
            told.load(Ordering::SeqCst),
            before + 1,
            "five steps, one message: the host has not looked in between"
        );
        assert!(
            wants_a_frame_soon(&ctx, &env, &frame),
            "and once awake it is told to draw within a turn"
        );

        let frame = ctx.snapshot(&env, 80, 24);
        assert!(
            !wants_a_frame_soon(&ctx, &env, &frame),
            "the snapshot has it, so there is nothing more to draw"
        );
        run(
            "(define-worker 'again (fiber (yield 'continue)))",
            &env,
            &ctx,
        );
        steps_due(&ctx);
        assert_eq!(
            told.load(Ordering::SeqCst),
            before + 2,
            "a step after the snapshot is news again"
        );
    }

    #[test]
    fn the_frame_for_a_change_waits_a_turn_after_the_last_one() {
        // A job colouring a large file takes a step every millisecond or so;
        // a frame for each would repaint the terminal continuously. So the
        // wait is a turn counted from the last snapshot: what is measured
        // here is bounded by the real time around it, which is all a test
        // can know about a clock it does not drive.
        let (ctx, env, _told) = watched();
        steps_due(&ctx);
        let before = Instant::now();
        let frame = ctx.snapshot(&env, 80, 24);
        run("(define-worker 'one (fiber (yield 'continue)))", &env, &ctx);
        steps_due(&ctx);
        let wait = ctx
            .next_redraw_in(&env, &frame)
            .expect("there is a change to draw");
        let since = before.elapsed();
        assert!(wait <= TURN_INTERVAL, "{wait:?}");
        assert!(
            wait >= TURN_INTERVAL.saturating_sub(since),
            "a turn after the frame, less what has passed since: {wait:?} after {since:?}"
        );
        std::thread::sleep(TURN_INTERVAL);
        assert_eq!(
            ctx.next_redraw_in(&env, &frame),
            Some(Duration::ZERO),
            "and at once when the last frame is older than a turn"
        );
    }

    #[test]
    fn a_file_the_watcher_reloads_tells_the_host() {
        // The case that showed the need: a file changed on disk is reloaded
        // by the watcher, and the screen went on showing the old text until a
        // key was pressed.
        let (ctx, env, told) = watched();
        let dir = std::env::temp_dir().join(format!(
            "rsedit-watch-redraw-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch directory");
        let path = dir.join("notes.txt");
        std::fs::write(&path, "first\n").expect("write");
        run(&format!(r#"(find-file "{}")"#, path.display()), &env, &ctx);
        wait(&ctx, Duration::from_secs(4));
        let frame = ctx.snapshot(&env, 80, 24);
        let before = told.load(Ordering::SeqCst);

        // Bigger as well as later, so the change shows however coarse the
        // file system's clock is.
        std::fs::write(&path, "second, from somewhere else\n").expect("rewrite");
        wait(&ctx, Duration::from_secs(4));
        let text = ctx.with_current_buffer(|buf| buf.text.to_string());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            text, "second, from somewhere else\n",
            "the watcher reloaded it"
        );
        assert!(
            told.load(Ordering::SeqCst) > before,
            "and the host was told"
        );
        assert!(wants_a_frame_soon(&ctx, &env, &frame));
        let fresh = ctx.snapshot(&env, 80, 24);
        assert_ne!(fresh, frame, "and the next frame shows it");
    }

    // ----------------------------------------------------------------
    // Which thread a task believes it is on
    // ----------------------------------------------------------------

    /// Where a thread of the editor's own believes it is, as it starts.
    fn where_a_thread_wakes(ctx: &Ctx) -> bool {
        let (tell, told) = std::sync::mpsc::channel();
        ctx.spawn_work("where-am-i", move |_state, _stop, _me| {
            let _ = tell.send(crate::editor::in_worker());
        })
        .expect("the thread started");
        told.recv_timeout(Duration::from_secs(10))
            .expect("the thread never ran")
    }

    #[test]
    fn a_thread_of_its_own_still_knows_it_is_not_the_command_thread() {
        // The flag is what `minibuffer-read' consults before refusing to put
        // up a prompt nobody is typing into, and a thread of its own is
        // further from the command thread than anything else here -- so it
        // is the case most in need of the flag and the one most easily left
        // out.
        let (ctx, _env) = editor();
        assert!(
            where_a_thread_wakes(&ctx),
            "a thread must not believe it is the command thread"
        );
    }

    #[test]
    fn an_inner_turn_ending_does_not_make_the_thread_the_command_thread() {
        // Two can hold one at once: a step taken by hand inside something
        // that already holds one. If the inner one cleared the flag instead
        // of restoring it, everything after would believe it was the thread
        // the user is typing on -- and the one thing the flag guards, a prompt
        // appearing out of nowhere, is exactly the thing nobody would think to
        // look for there.
        assert!(!crate::editor::in_worker(), "this thread is the user's");
        let outer = crate::editor::WorkerTurn::begin();
        {
            let _inner = crate::editor::WorkerTurn::begin();
            assert!(crate::editor::in_worker());
        }
        assert!(
            crate::editor::in_worker(),
            "the outer turn has not ended, so the thread has not changed"
        );
        drop(outer);
        assert!(!crate::editor::in_worker(), "and now it has");
    }

    #[test]
    fn the_command_thread_knows_it_is_the_command_thread() {
        // The other half, and the one that would fail if the flag were ever
        // left set: a thread setting it must not be visible here.
        let (ctx, _env) = editor();
        where_a_thread_wakes(&ctx);
        assert!(!crate::editor::in_worker(), "this thread is the user's");
    }

    // ----------------------------------------------------------------
    // Work that writes into a buffer
    // ----------------------------------------------------------------

    /// Start a thread that writes into NAME, as a command or a search does,
    /// and waits to be stopped: it reports when it has been, and whether its
    /// last line was let in.
    fn waiting_writer(ctx: &Ctx, name: &str) -> std::sync::mpsc::Receiver<bool> {
        let (tell, told) = std::sync::mpsc::channel();
        let buffer = name.to_string();
        ctx.spawn_writer(name, move |state, stop, me| {
            let (unblock, blocked) = std::sync::mpsc::channel::<()>();
            stop.on_cancel(move || drop(unblock));
            let _ = blocked.recv();
            let _ = tell.send(state.append_as_writer(&buffer, me, "last\n"));
        })
        .expect("the thread started");
        told
    }

    #[test]
    fn a_writer_for_a_buffer_that_is_not_there_is_stopped_at_once() {
        // Its buffer went before it could be told who writes into it -- closed
        // from another thread in that moment: nothing would ever stop the
        // work, so it is stopped then, before it has written anything.
        let (ctx, _env) = editor();
        let (tell, told) = std::sync::mpsc::channel();
        ctx.spawn_writer("*nowhere*", move |_state, stop, _me| {
            let _ = tell.send(stop.is_cancelled());
        })
        .expect("the thread started");
        assert_eq!(told.recv_timeout(Duration::from_secs(10)), Ok(true));
    }

    #[test]
    fn closing_a_buffer_stops_what_writes_into_it() {
        let (ctx, env) = editor();
        run(r#"(buffer-create "*writer*")"#, &env, &ctx);
        let told = waiting_writer(&ctx, "*writer*");
        assert!(ctx.with_buffer("*writer*", |buf| buf.writer.is_some()) == Some(true));
        assert!(ctx.close_buffer("*writer*", &env));
        assert_eq!(
            told.recv_timeout(Duration::from_secs(10)),
            Ok(false),
            "it was stopped, and its last line had nowhere to go"
        );
    }

    #[test]
    fn a_stopped_writer_cannot_write_into_a_new_buffer_of_the_same_name() {
        // Close a command's output and start another, and the new buffer may
        // well have the old one's name -- while the old command, stopping,
        // still has its last line to write.
        let (ctx, env) = editor();
        run(r#"(buffer-create "*writer*")"#, &env, &ctx);
        let told = waiting_writer(&ctx, "*writer*");
        // The buffer made to forget its writer first, so that closing it
        // does not stop the writer: still waiting when the new buffer
        // arrives, it is stopped by name after.
        ctx.with_buffer_mut("*writer*", |buf| buf.writer = None);
        run(
            r#"(close-buffer "*writer*") (buffer-create "*writer*")"#,
            &env,
            &ctx,
        );
        assert!(!run(r#"(stop-worker "*writer*")"#, &env, &ctx).is_nil());
        assert_eq!(told.recv_timeout(Duration::from_secs(10)), Ok(false));
        assert_eq!(
            ctx.with_buffer("*writer*", |buf| buf.text.to_string()),
            Some(String::new()),
            "the new buffer is not the old writer's"
        );
    }

    #[test]
    fn a_writer_s_buffer_is_its_own_while_it_runs_and_free_after() {
        let (ctx, env) = editor();
        run(r#"(buffer-create "*writer*")"#, &env, &ctx);
        let told = waiting_writer(&ctx, "*writer*");
        assert!(!run(r#"(stop-worker "*writer*")"#, &env, &ctx).is_nil());
        assert_eq!(
            told.recv_timeout(Duration::from_secs(10)),
            Ok(true),
            "stopped by name, the buffer is still its own"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while ctx.with_buffer("*writer*", |buf| buf.writer.is_some()) == Some(true) {
            assert!(
                Instant::now() < deadline,
                "the buffer still names its writer"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            ctx.with_buffer("*writer*", |buf| buf.text.to_string()),
            Some("last\n".to_string())
        );
    }

    #[test]
    fn a_name_work_has_taken_is_not_given_to_a_new_buffer() {
        // A buffer and the work writing into it share a name: a name only the
        // buffers had given up -- the old output renamed -- must not be
        // handed out again while the work under it runs.
        let (ctx, env) = editor();
        run(r#"(buffer-create "*writer*")"#, &env, &ctx);
        let told = waiting_writer(&ctx, "*writer*");
        assert!(ctx.rename_buffer("*writer*", "*renamed*"));
        assert_eq!(ctx.free_name_for_work("*writer*"), "*writer*<2>");
        assert!(!run(r#"(stop-worker "*writer*")"#, &env, &ctx).is_nil());
        assert!(told.recv_timeout(Duration::from_secs(10)).is_ok());
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
