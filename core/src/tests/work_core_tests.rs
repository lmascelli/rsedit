//! The scheduler's core, driven by hand on a clock that moves only when told.
//! No test here starts a thread or waits: a job meant to take time advances
//! the `ManualClock` from inside its step, so a step takes 30 ms because the
//! test says so, not because the machine was slow.
//!
//! Design: `doc/architettura-editor.typ, <prove-sfondo>`.
#[cfg(test)]
mod tests {
    use crate::work::{
        Class, Core, Exit, Job, JobError, ManualClock, Options, Place, Spec, Start, Step,
        StepContext, Topic, Topics, Wake, WorkId,
    };
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A core with its clock, options and topics at hand.
    struct Harness {
        core: Core<()>,
        clock: Arc<ManualClock>,
        options: Arc<Options>,
        topics: Arc<Topics>,
        start: Instant,
    }

    fn harness() -> Harness {
        let clock = ManualClock::new();
        let options = Arc::new(Options::default());
        let topics = Arc::new(Topics::default());
        let core = Core::new(options.clone(), topics.clone(), clock.clone());
        let start = crate::work::Clock::now(&*clock);
        Harness {
            core,
            clock,
            options,
            topics,
            start,
        }
    }

    impl Harness {
        fn turn(&mut self) -> bool {
            self.core.run_once(&())
        }

        fn run_for(&mut self, span: Duration) -> usize {
            self.core.run_for(&(), &self.clock, span, 10_000)
        }

        fn reports(&mut self) -> Vec<String> {
            self.core.take_reports()
        }
    }

    /// What a scripted job lived through, shared with the test.
    #[derive(Default)]
    struct Trace {
        /// Why each step happened, and when, from the harness's start.
        steps: Vec<(Wake, Duration)>,
        exits: Vec<Exit>,
    }

    /// A job that answers a fixed list, then a fixed answer for ever.
    struct Scripted {
        answers: VecDeque<Result<Step, JobError>>,
        then: Step,
        /// How long each step "runs": the clock moves this much inside it.
        takes: Duration,
        /// A topic to emit during the step with this index, from 0.
        emit_during: Option<(usize, Topic)>,
        worked: Option<(u64, &'static str)>,
        clock: Arc<ManualClock>,
        start: Instant,
        trace: Arc<Mutex<Trace>>,
    }

    impl Scripted {
        fn new(h: &Harness, answers: &[Step]) -> (Self, Arc<Mutex<Trace>>) {
            let trace = Arc::new(Mutex::new(Trace::default()));
            let job = Self {
                answers: answers.iter().map(|step| Ok(*step)).collect(),
                then: Step::Done,
                takes: Duration::ZERO,
                emit_during: None,
                worked: None,
                clock: h.clock.clone(),
                start: h.start,
                trace: trace.clone(),
            };
            (job, trace)
        }

        fn forever(mut self, step: Step) -> Self {
            self.then = step;
            self
        }

        fn taking(mut self, span: Duration) -> Self {
            self.takes = span;
            self
        }

        fn failing(mut self, message: &str) -> Self {
            self.answers.push_back(Err(JobError(message.to_string())));
            self
        }

        fn emitting_during(mut self, step: usize, topic: Topic) -> Self {
            self.emit_during = Some((step, topic));
            self
        }

        fn working(mut self, units: u64, on: &'static str) -> Self {
            self.worked = Some((units, on));
            self
        }
    }

    impl Job<()> for Scripted {
        fn step(&mut self, cx: &mut StepContext<'_, ()>) -> Result<Step, JobError> {
            let index = {
                let mut trace = self.trace.lock().unwrap();
                let at = crate::work::Clock::now(&*self.clock) - self.start;
                trace.steps.push((cx.woken_by, at));
                trace.steps.len() - 1
            };
            self.clock.advance(self.takes);
            if let Some((step, topic)) = &self.emit_during
                && *step == index
            {
                topic.emit();
            }
            if let Some((units, on)) = self.worked {
                cx.worked(units, on);
            }
            self.answers.pop_front().unwrap_or(Ok(self.then))
        }

        fn exit(&mut self, _state: &(), why: Exit) {
            self.trace.lock().unwrap().exits.push(why);
        }
    }

    /// A job whose every step panics.
    struct Panicking;

    impl Job<()> for Panicking {
        fn step(&mut self, _cx: &mut StepContext<'_, ()>) -> Result<Step, JobError> {
            panic!("index out of bounds");
        }
    }

    fn wakes(trace: &Arc<Mutex<Trace>>) -> Vec<Wake> {
        trace
            .lock()
            .unwrap()
            .steps
            .iter()
            .map(|(wake, _)| *wake)
            .collect()
    }

    fn times(trace: &Arc<Mutex<Trace>>) -> Vec<Duration> {
        trace
            .lock()
            .unwrap()
            .steps
            .iter()
            .map(|(_, at)| *at)
            .collect()
    }

    fn exits(trace: &Arc<Mutex<Trace>>) -> Vec<Exit> {
        trace.lock().unwrap().exits.clone()
    }

    // -- starting, stepping, leaving -----------------------------------------

    #[test]
    fn a_job_starts_at_once_and_leaves_exactly_once() {
        let mut h = harness();
        let (job, trace) = Scripted::new(&h, &[Step::Done]);
        h.core.submit(Spec::new("once"), Box::new(job));

        assert!(
            h.turn(),
            "a job submitted to start now runs on the next turn"
        );
        assert!(!h.turn(), "and then there is nothing left to run");
        assert_eq!(wakes(&trace), [Wake::Start]);
        assert_eq!(exits(&trace), [Exit::Finished]);
        assert!(
            h.core.info().is_empty(),
            "a finished job is gone from the core"
        );
    }

    #[test]
    fn a_delayed_start_waits_for_its_deadline() {
        let mut h = harness();
        let (job, trace) = Scripted::new(&h, &[Step::Done]);
        h.core.submit(
            Spec::new("later").start(Start::After(ms(50))),
            Box::new(job),
        );

        assert!(!h.turn(), "nothing is due yet");
        h.clock.advance(ms(49));
        assert!(!h.turn(), "still not due a millisecond early");
        h.clock.advance(ms(1));
        assert!(h.turn());
        assert_eq!(
            wakes(&trace),
            [Wake::Start],
            "its first step is still a start"
        );
    }

    #[test]
    fn a_wait_too_long_for_the_clock_is_a_year_not_a_crash() {
        // `Instant + Duration::MAX` panics, and the core adds a job's answer to
        // the clock outside the step, where a panic would take the executor
        // down with every job on it. A sleep, a bounded wait and a delayed
        // start all land a year out instead.
        let mut h = harness();
        let (sleeper, _) = Scripted::new(&h, &[Step::Sleep(Duration::MAX)]);
        let (waiter, _) = Scripted::new(&h, &[Step::WaitAtMost(Duration::MAX)]);
        let (late, late_trace) = Scripted::new(&h, &[]);
        h.core.submit(Spec::new("sleeper"), Box::new(sleeper));
        h.core.submit(Spec::new("waiter").on("t"), Box::new(waiter));
        h.core.submit(
            Spec::new("late").start(Start::After(Duration::MAX)),
            Box::new(late),
        );

        h.run_for(ms(10));
        let year = Duration::from_secs(365 * 24 * 60 * 60);
        let wakeup = h.core.next_wakeup().expect("each is due some time");
        assert_eq!(wakeup.duration_since(h.start), year);
        let places: Vec<Place> = h.core.info().iter().map(|job| job.place).collect();
        assert_eq!(places, [Place::Sleeping, Place::Waiting, Place::Sleeping]);
        assert!(
            wakes(&late_trace).is_empty(),
            "and the late one is not due yet"
        );
    }

    #[test]
    fn a_sleep_counts_from_the_end_of_the_step() {
        let mut h = harness();
        let (job, trace) = Scripted::new(&h, &[Step::Sleep(ms(10)), Step::Sleep(ms(10))]);
        h.core
            .submit(Spec::new("periodic"), Box::new(job.taking(ms(30))));

        h.run_for(ms(200));
        // Each step runs 30 ms and sleeps 10 from where it ended: 0, 40, 80.
        assert_eq!(times(&trace), [ms(0), ms(40), ms(80)]);
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Timer, Wake::Timer]);
    }

    #[test]
    fn a_missed_period_is_not_made_up() {
        let mut h = harness();
        let (job, trace) = Scripted::new(&h, &[]);
        h.core.submit(
            Spec::new("periodic"),
            Box::new(job.forever(Step::Sleep(ms(10)))),
        );
        assert!(h.turn());

        // A tenth of a second passes with the executor away: ten periods.
        h.clock.advance(ms(100));
        assert!(h.turn(), "one step for the deadline that passed");
        assert!(!h.turn(), "and no burst to catch up on the other nine");
        assert_eq!(wakes(&trace).len(), 2);
    }

    // -- who goes first -------------------------------------------------------

    /// The first letter of the job that took each of the next N steps.
    fn order(h: &mut Harness, traces: &[(char, &Arc<Mutex<Trace>>)], n: usize) -> String {
        let mut seen: Vec<usize> = traces
            .iter()
            .map(|(_, t)| t.lock().unwrap().steps.len())
            .collect();
        let mut out = String::new();
        for _ in 0..n {
            assert!(h.turn(), "a step was expected");
            for (i, (letter, trace)) in traces.iter().enumerate() {
                let now = trace.lock().unwrap().steps.len();
                if now != seen[i] {
                    out.push(*letter);
                    seen[i] = now;
                }
            }
        }
        out
    }

    #[test]
    fn jobs_of_a_class_take_turns() {
        let mut h = harness();
        let (a, ta) = Scripted::new(&h, &[]);
        let (b, tb) = Scripted::new(&h, &[]);
        h.core
            .submit(Spec::new("a"), Box::new(a.forever(Step::Continue)));
        h.core
            .submit(Spec::new("b"), Box::new(b.forever(Step::Continue)));

        assert_eq!(order(&mut h, &[('a', &ta), ('b', &tb)], 6), "ababab");
        assert_eq!(wakes(&ta)[1], Wake::Turn, "a step after Continue is a turn");
    }

    #[test]
    fn a_normal_job_gets_a_step_after_high_steps_high_ones() {
        let mut h = harness();
        let (e, te) = Scripted::new(&h, &[]);
        let (i, ti) = Scripted::new(&h, &[]);
        h.core.submit(
            Spec::new("highlighter").class(Class::High),
            Box::new(e.forever(Step::Continue)),
        );
        h.core
            .submit(Spec::new("index"), Box::new(i.forever(Step::Continue)));

        assert_eq!(order(&mut h, &[('E', &te), ('I', &ti)], 10), "EEEEIEEEEI");
    }

    #[test]
    fn high_steps_changed_while_running_counts_from_the_next_choice() {
        let mut h = harness();
        let (e, te) = Scripted::new(&h, &[]);
        let (i, ti) = Scripted::new(&h, &[]);
        h.core.submit(
            Spec::new("highlighter").class(Class::High),
            Box::new(e.forever(Step::Continue)),
        );
        h.core
            .submit(Spec::new("index"), Box::new(i.forever(Step::Continue)));

        assert_eq!(order(&mut h, &[('E', &te), ('I', &ti)], 5), "EEEEI");
        h.options.set("high-steps", 2).unwrap();
        assert_eq!(order(&mut h, &[('E', &te), ('I', &ti)], 6), "EEIEEI");
    }

    // -- events ---------------------------------------------------------------

    #[test]
    fn a_burst_of_events_wakes_a_waiting_job_once() {
        let mut h = harness();
        let changed = h.topics.get("buffer-changed");
        let (job, trace) = Scripted::new(&h, &[Step::Wait, Step::Wait]);
        h.core
            .submit(Spec::new("probe").on("buffer-changed"), Box::new(job));
        assert!(h.turn(), "the first step answers Wait");

        // Three keystrokes in ten milliseconds.
        for _ in 0..3 {
            changed.emit();
            h.turn();
            h.clock.advance(ms(5));
        }
        h.run_for(ms(100));
        assert_eq!(
            wakes(&trace),
            [Wake::Start, Wake::Event],
            "one wake for the burst"
        );
        assert_eq!(
            times(&trace)[1],
            ms(20),
            "event-delay after the first event the core noticed"
        );
    }

    #[test]
    fn an_event_during_the_step_is_not_lost() {
        let mut h = harness();
        let changed = h.topics.get("buffer-changed");
        let (job, trace) = Scripted::new(&h, &[Step::Wait, Step::Wait]);
        h.core.submit(
            Spec::new("probe").on("buffer-changed"),
            Box::new(job.emitting_during(0, changed).taking(ms(5))),
        );

        h.run_for(ms(100));
        assert_eq!(
            wakes(&trace),
            [Wake::Start, Wake::Event],
            "the event that arrived while the first step ran wakes the job again"
        );
        assert_eq!(times(&trace)[1], ms(25), "event-delay after the step ended");
    }

    #[test]
    fn a_sleeping_job_is_not_woken_by_events() {
        let mut h = harness();
        let changed = h.topics.get("buffer-changed");
        let (job, trace) = Scripted::new(&h, &[Step::Sleep(ms(100))]);
        h.core
            .submit(Spec::new("probe").on("buffer-changed"), Box::new(job));
        assert!(h.turn());

        changed.emit();
        h.run_for(ms(200));
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Timer]);
        assert_eq!(times(&trace)[1], ms(100));
    }

    #[test]
    fn wait_at_most_wakes_on_the_event_when_it_comes_first() {
        let mut h = harness();
        h.options.set("event-delay", 0).unwrap();
        let changed = h.topics.get("buffer-changed");
        let (job, trace) = Scripted::new(&h, &[Step::WaitAtMost(ms(50))]);
        h.core
            .submit(Spec::new("probe").on("buffer-changed"), Box::new(job));
        assert!(h.turn());

        h.clock.advance(ms(5));
        changed.emit();
        h.run_for(ms(100));
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Event]);
        assert_eq!(times(&trace)[1], ms(5));
    }

    #[test]
    fn wait_at_most_wakes_on_the_deadline_when_no_event_comes() {
        let mut h = harness();
        let (job, trace) = Scripted::new(&h, &[Step::WaitAtMost(ms(50))]);
        h.core
            .submit(Spec::new("probe").on("buffer-changed"), Box::new(job));

        h.run_for(ms(100));
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Timer]);
        assert_eq!(times(&trace)[1], ms(50));
    }

    #[test]
    fn waiting_with_no_topic_ends_the_job_and_says_why() {
        let mut h = harness();
        let (job, trace) = Scripted::new(&h, &[Step::Wait]);
        h.core.submit(Spec::new("forgotten"), Box::new(job));

        assert!(h.turn());
        h.turn();
        assert_eq!(exits(&trace), [Exit::Failed]);
        let reports = h.reports();
        assert!(
            reports
                .iter()
                .any(|line| line.contains("forgotten waits for an event")),
            "{reports:?}"
        );
    }

    #[test]
    fn next_wakeup_is_the_earliest_deadline_or_maturing_event() {
        let mut h = harness();
        let changed = h.topics.get("buffer-changed");
        let (sleeper, _) = Scripted::new(&h, &[Step::Sleep(ms(50))]);
        let (waiter, _) = Scripted::new(&h, &[Step::Wait]);
        h.core.submit(Spec::new("sleeper"), Box::new(sleeper));
        h.core
            .submit(Spec::new("waiter").on("buffer-changed"), Box::new(waiter));
        h.core.run_until_idle(&(), 10);

        let at = |h: &Harness| h.core.next_wakeup().map(|t| t - h.start);
        assert_eq!(at(&h), Some(ms(50)), "only the sleeper's deadline");
        changed.emit();
        assert_eq!(at(&h), Some(ms(20)), "the event matures first");
    }

    /// A job that names its topics from inside its steps, as a Lisp job does
    /// with `(yield '(wait ...))`: each step subscribes to the topics of its
    /// plan, if it has any, and answers the plan's step.
    struct Resubscribing {
        plan: VecDeque<(Option<Vec<&'static str>>, Step)>,
        then: (Option<Vec<&'static str>>, Step),
        trace: Arc<Mutex<Trace>>,
        clock: Arc<ManualClock>,
        start: Instant,
    }

    impl Resubscribing {
        fn new(
            h: &Harness,
            plan: Vec<(Option<Vec<&'static str>>, Step)>,
            then: (Option<Vec<&'static str>>, Step),
        ) -> (Self, Arc<Mutex<Trace>>) {
            let trace = Arc::new(Mutex::new(Trace::default()));
            let job = Self {
                plan: plan.into(),
                then,
                trace: trace.clone(),
                clock: h.clock.clone(),
                start: h.start,
            };
            (job, trace)
        }
    }

    impl Job<()> for Resubscribing {
        fn step(&mut self, cx: &mut StepContext<'_, ()>) -> Result<Step, JobError> {
            let at = crate::work::Clock::now(&*self.clock) - self.start;
            self.trace.lock().unwrap().steps.push((cx.woken_by, at));
            let (topics, step) = self.plan.pop_front().unwrap_or_else(|| self.then.clone());
            if let Some(topics) = topics {
                cx.subscribe(topics);
            }
            Ok(step)
        }

        fn exit(&mut self, _state: &(), why: Exit) {
            self.trace.lock().unwrap().exits.push(why);
        }
    }

    #[test]
    fn a_step_names_the_topics_that_wake_it_from_then_on() {
        let mut h = harness();
        let mine = h.topics.get("mine");
        let other = h.topics.get("other");
        let (job, trace) = Resubscribing::new(
            &h,
            vec![(Some(vec!["mine"]), Step::Wait)],
            (Some(vec!["mine"]), Step::Wait),
        );
        h.core
            .submit(Spec::new("listener").on("other"), Box::new(job));

        h.run_for(ms(1_000));
        assert_eq!(
            wakes(&trace),
            [Wake::Start, Wake::Event],
            "a topic it did not have counts as changed: one more look, then it waits"
        );
        assert_eq!(times(&trace)[1], ms(20), "event-delay after the step");

        other.emit();
        h.run_for(ms(1_000));
        assert_eq!(
            wakes(&trace).len(),
            2,
            "the topic it was submitted with is gone"
        );
        mine.emit();
        h.run_for(ms(1_000));
        assert_eq!(wakes(&trace).len(), 3, "and the one it named wakes it");
    }

    #[test]
    fn naming_the_topics_it_has_costs_no_step() {
        // The steady state of a Lisp job that always waits the same way.
        let mut h = harness();
        let mine = h.topics.get("mine");
        let (job, trace) = Resubscribing::new(&h, vec![], (Some(vec!["mine", "mine"]), Step::Wait));
        h.core.submit(Spec::new("steady").on("mine"), Box::new(job));

        h.run_for(ms(1_000));
        assert_eq!(
            wakes(&trace),
            [Wake::Start],
            "nothing new, so nothing to look at"
        );
        mine.emit();
        h.run_for(ms(1_000));
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Event]);
    }

    #[test]
    fn an_event_during_the_step_that_resubscribes_is_not_lost() {
        // The count a topic it keeps was last seen at is from before the step,
        // as for any job: an emission while it ran still wakes it.
        let mut h = harness();
        let mine = h.topics.get("mine");
        struct EmitsThenWaits(Topic, Arc<Mutex<usize>>);
        impl Job<()> for EmitsThenWaits {
            fn step(&mut self, cx: &mut StepContext<'_, ()>) -> Result<Step, JobError> {
                let mut steps = self.1.lock().unwrap();
                *steps += 1;
                if *steps == 1 {
                    self.0.emit();
                }
                cx.subscribe(["mine"]);
                Ok(Step::Wait)
            }
        }
        let steps = Arc::new(Mutex::new(0));
        h.core.submit(
            Spec::new("racer").on("mine"),
            Box::new(EmitsThenWaits(mine.clone(), steps.clone())),
        );
        h.run_for(ms(1_000));
        assert_eq!(*steps.lock().unwrap(), 2);
    }

    #[test]
    fn subscribing_to_nothing_and_waiting_fails_the_job() {
        let mut h = harness();
        let (job, trace) = Resubscribing::new(&h, vec![], (Some(vec![]), Step::Wait));
        h.core.submit(Spec::new("deaf").on("mine"), Box::new(job));
        h.run_for(ms(100));
        assert_eq!(exits(&trace), [Exit::Failed]);
    }

    #[test]
    fn a_step_knows_its_own_number() {
        let mut h = harness();
        struct Numbered(Arc<Mutex<Option<WorkId>>>);
        impl Job<()> for Numbered {
            fn step(&mut self, cx: &mut StepContext<'_, ()>) -> Result<Step, JobError> {
                *self.0.lock().unwrap() = Some(cx.id);
                Ok(Step::Done)
            }
        }
        let seen = Arc::new(Mutex::new(None));
        let id = h
            .core
            .submit(Spec::new("numbered"), Box::new(Numbered(seen.clone())))
            .expect("taken");
        h.turn();
        assert_eq!(*seen.lock().unwrap(), Some(id));
    }

    // -- stopping -------------------------------------------------------------

    #[test]
    fn a_cancelled_job_leaves_without_another_step() {
        let mut h = harness();
        let (job, trace) = Scripted::new(&h, &[Step::Sleep(ms(10))]);
        let id = h.core.submit(Spec::new("probe"), Box::new(job)).unwrap();
        assert!(h.turn());

        assert!(h.core.cancel(id));
        assert!(!h.core.cancel(id), "a job is cancelled once");
        h.run_for(ms(100));
        assert_eq!(wakes(&trace), [Wake::Start], "no step after the cancel");
        assert_eq!(exits(&trace), [Exit::Cancelled]);
    }

    #[test]
    fn a_job_under_a_taken_name_replaces_the_old_one() {
        let mut h = harness();
        let (old, old_trace) = Scripted::new(&h, &[Step::Sleep(ms(10))]);
        let (new, new_trace) = Scripted::new(&h, &[Step::Done]);
        let first: WorkId = h.core.submit(Spec::new("index"), Box::new(old)).unwrap();
        assert!(h.turn());

        let second = h.core.submit(Spec::new("index"), Box::new(new)).unwrap();
        assert_ne!(first, second, "a number is never reused");
        assert_eq!(h.core.info().len(), 1, "one job per name");
        h.turn();
        assert_eq!(exits(&old_trace), [Exit::Cancelled]);
        assert_eq!(wakes(&new_trace), [Wake::Start]);
    }

    #[test]
    fn shutdown_ends_every_job_and_takes_no_more() {
        let mut h = harness();
        let (a, ta) = Scripted::new(&h, &[Step::Sleep(ms(10))]);
        let (b, tb) = Scripted::new(&h, &[]);
        h.core.submit(Spec::new("a"), Box::new(a));
        h.core
            .submit(Spec::new("b").start(Start::After(ms(10))), Box::new(b));
        assert!(h.turn());

        h.core.shutdown(&());
        assert_eq!(exits(&ta), [Exit::Shutdown]);
        assert_eq!(exits(&tb), [Exit::Shutdown], "even a job that never ran");
        let (late, _) = Scripted::new(&h, &[]);
        assert_eq!(h.core.submit(Spec::new("late"), Box::new(late)), None);
    }

    // -- failures -------------------------------------------------------------

    #[test]
    fn an_error_ends_the_job_and_is_reported() {
        let mut h = harness();
        let (job, trace) = Scripted::new(&h, &[]);
        h.core.submit(
            Spec::new("parser"),
            Box::new(job.failing("unbalanced parenthesis")),
        );

        assert!(h.turn());
        h.turn();
        assert_eq!(exits(&trace), [Exit::Failed]);
        assert_eq!(h.reports(), ["[BKG] error: parser: unbalanced parenthesis"]);
    }

    #[test]
    fn a_panic_ends_the_job_and_the_others_go_on() {
        let mut h = harness();
        let (other, trace) = Scripted::new(&h, &[]);
        h.core.submit(Spec::new("counter"), Box::new(Panicking));
        h.core
            .submit(Spec::new("other"), Box::new(other.forever(Step::Continue)));

        for _ in 0..4 {
            assert!(h.turn());
        }
        assert_eq!(wakes(&trace).len(), 3, "the other job kept its turns");
        assert_eq!(h.core.info().len(), 1, "the panicking job is gone");
        assert_eq!(
            h.reports(),
            ["[BKG] panic: counter retired: index out of bounds"]
        );
    }

    // -- measures and reports -------------------------------------------------

    #[test]
    fn a_long_step_is_reported_once_then_summarised() {
        let mut h = harness();
        let (job, _) = Scripted::new(&h, &[]);
        h.core.submit(
            Spec::new("highlighter").unit("lines"),
            Box::new(
                job.forever(Step::Sleep(ms(100)))
                    .taking(ms(90))
                    .working(3, "app.min.js"),
            ),
        );

        assert!(h.turn());
        assert_eq!(
            h.reports(),
            ["[BKG] long step: highlighter, 90 ms on app.min.js (3 lines, 30 ms each)"]
        );
        h.run_for(ms(5_000));
        let quiet = h.reports();
        assert!(
            quiet.is_empty(),
            "the next ones are only counted: {quiet:?}"
        );
        h.run_for(ms(6_000));
        let reports = h.reports();
        assert_eq!(reports.len(), 1, "{reports:?}");
        assert!(
            reports[0].starts_with("[BKG] summary: highlighter, ")
                && reports[0].ends_with("long steps in the last 10 s (worst 90 ms)"),
            "{reports:?}"
        );
    }

    #[test]
    fn a_long_step_counted_in_something_cheap_gives_a_cost_that_says_something() {
        // A Lisp job counts fuel, which costs a fraction of a microsecond a
        // step, and names no file: "0 ms each on" would say nothing at all.
        let mut h = harness();
        let (job, _) = Scripted::new(&h, &[Step::Done]);
        h.core.submit(
            Spec::new("manpage-index").unit("fuel"),
            Box::new(job.taking(ms(50)).working(200_000, "")),
        );
        assert!(h.turn());
        assert_eq!(
            h.reports(),
            ["[BKG] long step: manpage-index, 50 ms (200000 fuel, 250 ns each)"]
        );

        let (job, _) = Scripted::new(&h, &[Step::Done]);
        h.core.submit(
            Spec::new("indexer").unit("files"),
            Box::new(job.taking(ms(45)).working(9, "src")),
        );
        assert!(h.turn());
        assert_eq!(
            h.reports(),
            ["[BKG] long step: indexer, 45 ms on src (9 files, 5 ms each)"]
        );

        let (job, _) = Scripted::new(&h, &[Step::Done]);
        h.core.submit(
            Spec::new("scanner").unit("lines"),
            Box::new(job.taking(ms(42)).working(4_200, "big.c")),
        );
        assert!(h.turn());
        assert_eq!(
            h.reports(),
            ["[BKG] long step: scanner, 42 ms on big.c (4200 lines, 10 µs each)"]
        );
    }

    #[test]
    fn a_late_start_names_what_ran_meanwhile() {
        let mut h = harness();
        let (high, _) = Scripted::new(&h, &[]);
        let (normal, _) = Scripted::new(&h, &[Step::Done]);
        h.core.submit(
            Spec::new("highlighter").class(Class::High),
            Box::new(high.forever(Step::Continue).taking(ms(60))),
        );
        h.core.submit(Spec::new("watcher"), Box::new(normal));

        for _ in 0..5 {
            h.turn();
        }
        let reports = h.reports();
        assert!(
            reports.iter().any(|line| line
                == "[BKG] late start: watcher started 240 ms late, behind highlighter (4 steps)"),
            "{reports:?}"
        );
    }

    #[test]
    fn a_busy_second_is_reported() {
        let mut h = harness();
        let (job, _) = Scripted::new(&h, &[]);
        h.core.submit(
            Spec::new("index"),
            Box::new(job.forever(Step::Sleep(ms(50))).taking(ms(100))),
        );

        h.run_for(ms(1_200));
        let reports = h.reports();
        assert!(
            reports
                .iter()
                .any(|line| line.starts_with("[BKG] load: ") && line.contains("(index ")),
            "{reports:?}"
        );
    }

    #[test]
    fn info_says_where_each_job_is() {
        let mut h = harness();
        let (sleeper, _) = Scripted::new(&h, &[Step::Sleep(ms(10))]);
        let (waiter, _) = Scripted::new(&h, &[Step::Wait]);
        let (queued, _) = Scripted::new(&h, &[]);
        h.core.submit(Spec::new("sleeper"), Box::new(sleeper));
        h.core
            .submit(Spec::new("waiter").on("buffer-changed"), Box::new(waiter));
        h.core.run_until_idle(&(), 2);
        h.core.submit(Spec::new("queued"), Box::new(queued));

        let places: Vec<(String, Place, u64)> = h
            .core
            .info()
            .into_iter()
            .map(|info| (info.name, info.place, info.steps))
            .collect();
        assert_eq!(
            places,
            [
                ("sleeper".to_string(), Place::Sleeping, 1),
                ("waiter".to_string(), Place::Waiting, 1),
                ("queued".to_string(), Place::Ready, 0),
            ]
        );
    }

    #[test]
    fn options_refuse_unknown_names_and_values_out_of_range() {
        let options = Options::default();
        assert_eq!(options.get("high-steps"), Some(4));
        assert!(options.set("high-steps", 0).is_err(), "at least one step");
        assert!(options.set("no-such-option", 1).is_err());
        assert!(
            options.set("busy-share", 101).is_err(),
            "a share is a percentage"
        );
        assert_eq!(
            options.get("high-steps"),
            Some(4),
            "a refused value changes nothing"
        );
        assert_eq!(options.all().len(), 9);
    }

    // -- invariants over random sequences -------------------------------------

    /// A small generator written here, because the crate takes no dependency
    /// for one test: xorshift, seeded, so a failure names a seed to replay.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x % n
        }
    }

    #[test]
    fn random_sequences_keep_the_invariants() {
        for seed in 1..=60_u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut h = harness();
            let topics = [h.topics.get("a"), h.topics.get("b")];
            // Every job ever submitted: its class and its trace.
            let mut jobs: Vec<(WorkId, Class, Arc<Mutex<Trace>>)> = Vec::new();
            let mut steps_seen: HashMap<WorkId, usize> = HashMap::new();
            let mut high_streak = 0_u64;

            for _ in 0..400 {
                match rng.below(8) {
                    0 | 1 => {
                        let answers: Vec<Step> = (0..1 + rng.below(5))
                            .map(|_| match rng.below(6) {
                                0 => Step::Continue,
                                1 => Step::Sleep(ms(1 + rng.below(30))),
                                2 => Step::Wait,
                                3 => Step::WaitAtMost(ms(1 + rng.below(30))),
                                4 => Step::Continue,
                                _ => Step::Done,
                            })
                            .collect();
                        let (job, trace) = Scripted::new(&h, &answers);
                        let class = if rng.below(2) == 0 {
                            Class::High
                        } else {
                            Class::Normal
                        };
                        let mut spec = Spec::new(&format!("job{}", rng.below(6))).class(class);
                        if rng.below(2) == 0 {
                            spec = spec.on(topics[rng.below(2) as usize].name());
                        }
                        let id = h.core.submit(spec, Box::new(job)).unwrap();
                        jobs.push((id, class, trace));
                    }
                    2 => {
                        if !jobs.is_empty() {
                            let (id, _, _) = &jobs[rng.below(jobs.len() as u64) as usize];
                            h.core.cancel(*id);
                        }
                    }
                    3 => topics[rng.below(2) as usize].emit(),
                    4 => h.clock.advance(ms(rng.below(40))),
                    _ => {
                        let normal_ready =
                            h.core.info().iter().any(|info| {
                                info.class == Class::Normal && info.place == Place::Ready
                            });
                        if h.turn() {
                            // Which job took the step: the one whose trace grew.
                            let ran: Vec<Class> = jobs
                                .iter()
                                .filter(|(id, _, trace)| {
                                    let now = trace.lock().unwrap().steps.len();
                                    let before = steps_seen.insert(*id, now).unwrap_or(0);
                                    now != before
                                })
                                .map(|(_, class, _)| *class)
                                .collect();
                            assert_eq!(ran.len(), 1, "seed {seed}: one step per turn");
                            if normal_ready && high_streak >= h.options.high_steps() {
                                assert_eq!(
                                    ran[0],
                                    Class::Normal,
                                    "seed {seed}: a ready normal job goes after high-steps high steps"
                                );
                            }
                            high_streak = if ran[0] == Class::High {
                                high_streak + 1
                            } else {
                                0
                            };
                        }
                    }
                }

                // Every job is in at most one place, and has left at most once;
                // a job the core still holds has not left.
                let held: Vec<WorkId> = h.core.info().iter().map(|info| info.id).collect();
                let mut unique = held.clone();
                unique.sort();
                unique.dedup();
                assert_eq!(unique.len(), held.len(), "seed {seed}: a job in two places");
                for (id, _, trace) in &jobs {
                    let left = trace.lock().unwrap().exits.len();
                    assert!(left <= 1, "seed {seed}: a job left twice");
                    if held.contains(id) {
                        assert_eq!(left, 0, "seed {seed}: a job left but is still held");
                    }
                }
            }

            // At the end every job has left exactly once.
            h.core.shutdown(&());
            for (_, _, trace) in &jobs {
                assert_eq!(
                    trace.lock().unwrap().exits.len(),
                    1,
                    "seed {seed}: every job leaves once"
                );
            }
        }
    }
}
