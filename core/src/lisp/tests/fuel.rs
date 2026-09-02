#[cfg(test)]
mod tests {
    use crate::lisp::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, RwLock};

    #[test]
    fn a_nested_scope_does_not_refill_the_budget() {
        // The point of the depth counter: code that re-enters the evaluator
        // mid-command must keep spending the budget it already has, rather
        // than quietly being handed a fresh one.
        let meter = FuelMeter::new(1_000);

        let outer = meter.begin();
        meter.consume(400).expect("600 should remain");
        {
            let _inner = meter.begin();
            meter.consume(400).expect("200 should remain");
        }
        assert!(
            meter.consume(300).is_err(),
            "the inner scope refilled the budget -- nested evaluation can launder its own limit"
        );
        drop(outer);
    }

    #[test]
    fn a_fresh_outermost_scope_does_refill() {
        let meter = FuelMeter::new(1_000);
        {
            let _scope = meter.begin();
            meter.consume(900).expect("first scope has a full budget");
        }
        let _scope = meter.begin();
        meter
            .consume(900)
            .expect("a new outermost scope must start from a full budget");
    }

    #[test]
    fn consume_spends_the_last_unit_and_then_reports_exhaustion() {
        // The old editor-side check was `remaining > amount`, which refused to
        // spend the final unit, and it decremented with a wrapping `fetch_sub`.
        let meter = FuelMeter::new(10);
        let _scope = meter.begin();
        meter
            .consume(10)
            .expect("spending exactly the remaining budget must succeed");
        assert_eq!(meter.consume(1).unwrap_err(), EvalError::OutOfFuel);
    }

    // ====================================================================== //
    //  A spawned thread's budget
    //
    //  Two properties, tested first on the meter alone and then through the
    //  `(spawn ...)` special form that is supposed to wire them up:
    //
    //    1. the counter is per-thread, so what a child spends is invisible to
    //       its parent and vice versa;
    //    2. a child starts on the meter's *configured* budget, which is what
    //       `arm_thread` is for -- without it the child would silently run on
    //       the compile-time `DEFAULT_FUEL` instead.
    // ====================================================================== //

    #[test]
    fn a_spawned_thread_does_not_inherit_the_parents_remaining_fuel() {
        let meter = Arc::new(FuelMeter::new(1_000));
        let _scope = meter.begin();
        meter
            .consume(999)
            .expect("one unit should be left on this thread");

        let child = Arc::clone(&meter);
        std::thread::spawn(move || {
            // No `arm_thread` here on purpose: the child starts on the
            // thread_local const initialiser, which is DEFAULT_FUEL -- not the
            // single unit the parent has left, and not zero.
            child
                .consume(DEFAULT_FUEL)
                .expect("a fresh thread starts from DEFAULT_FUEL");
            assert_eq!(child.consume(1).unwrap_err(), EvalError::OutOfFuel);
        })
        .join()
        .expect("the child thread panicked");

        // The child spent an entire default budget; none of it came out of the
        // parent's counter.
        meter
            .consume(1)
            .expect("the parent still holds its last unit");
        assert_eq!(meter.consume(1).unwrap_err(), EvalError::OutOfFuel);
    }

    #[test]
    fn arm_thread_starts_a_thread_on_the_configured_budget() {
        // 500 is deliberately nothing like DEFAULT_FUEL, so this distinguishes
        // "armed from the meter" from "fell back to the const initialiser".
        let meter = Arc::new(FuelMeter::new(500));
        let child = Arc::clone(&meter);
        std::thread::spawn(move || {
            child.arm_thread();
            child
                .consume(500)
                .expect("the child starts from the configured budget");
            assert_eq!(child.consume(1).unwrap_err(), EvalError::OutOfFuel);
        })
        .join()
        .expect("the child thread panicked");
    }

    // ---------- and the same two properties through `(spawn ...)` ----------

    /// A host context that actually meters, unlike the `()` used as a context
    /// elsewhere in these tests. `begin_thread_evaluation` is the wiring under
    /// test here: `(spawn ...)` is supposed to call it on the new thread, and
    /// nothing else ever does.
    #[derive(Clone, Debug)]
    struct FuelCtx {
        fuel: Arc<FuelMeter>,
        logs: Arc<RwLock<Vec<String>>>,
        ticks: Arc<AtomicU32>,
    }

    impl PartialEq for FuelCtx {
        fn eq(&self, other: &Self) -> bool {
            Arc::ptr_eq(&self.fuel, &other.fuel)
        }
    }

    impl LispContext for FuelCtx {
        fn consume_fuel(&self, amount: u32) -> Result<(), EvalError> {
            self.fuel.consume(amount)
        }

        fn log_diagnostic(&self, msg: &str) {
            self.logs
                .write()
                .expect("log lock poisoned")
                .push(msg.to_string());
        }

        fn begin_thread_evaluation(&self) {
            self.fuel.arm_thread();
        }
    }

    fn fuel_env(budget: u32) -> (Arc<Env<FuelCtx>>, FuelCtx) {
        let env = Env::new_root();
        setup_base_env(env.clone());
        // Counts how far a spawned body got round its loop, so a test can bound
        // the child's work without measuring time.
        env.set_function(
            "tick!".into(),
            LispExp::primitive(
                |_args, _env, ctx: &FuelCtx| {
                    ctx.ticks.fetch_add(1, Ordering::Relaxed);
                    Ok(LispExp::nil())
                },
                None,
            ),
        );
        let ctx = FuelCtx {
            fuel: Arc::new(FuelMeter::new(budget)),
            logs: Arc::new(RwLock::new(Vec::new())),
            ticks: Arc::new(AtomicU32::new(0)),
        };
        (env, ctx)
    }

    fn eval_script(
        script: &str,
        env: Arc<Env<FuelCtx>>,
        ctx: &FuelCtx,
    ) -> Result<LispExp<FuelCtx>, EvalError> {
        let wrapped = format!("(progn {script})");
        let mut parser = Parser::new(&wrapped);
        eval(
            &parser.next().expect("failed to parse test script"),
            env,
            ctx,
        )
    }

    /// `(spawn ...)` is fire-and-forget -- it returns nil immediately and never
    /// joins -- so the only thing to wait on is the effect.
    ///
    /// The cap is generous because a *failing* run is the slow one: a child
    /// that was never armed still dies eventually, just after ten million
    /// steps rather than a few thousand. Waiting that out lets the assertion
    /// after the wait report what actually went wrong instead of the test
    /// bailing out here with nothing to say. A healthy run leaves this loop on
    /// its first or second pass.
    fn wait_for(ctx: &FuelCtx, needle: &str) -> bool {
        for _ in 0..2_000 {
            if ctx
                .logs
                .read()
                .expect("log lock poisoned")
                .iter()
                .any(|line| line.contains(needle))
            {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn a_runaway_spawned_thread_is_stopped_by_its_own_budget() {
        let (env, ctx) = fuel_env(50_000);
        eval_script("(spawn (lambda () (while t 1)))", env, &ctx)
            .expect("spawning should succeed on the parent");
        assert!(
            wait_for(&ctx, "OutOfFuel"),
            "the child looped without ever being charged -- a spawned thread is unmetered"
        );
    }

    #[test]
    fn a_childs_exhaustion_leaves_the_parent_free_to_keep_running() {
        let (env, ctx) = fuel_env(30_000);
        eval_script("(spawn (lambda () (while t 1)))", env.clone(), &ctx).expect("spawn failed");
        assert!(
            wait_for(&ctx, "OutOfFuel"),
            "the child never reported running out of fuel"
        );

        // The child just burned a whole 30_000-step budget. Were the counter
        // shared rather than thread-local, there would be nothing left here.
        eval_script("(setq i 0) (while (< i 500) (setq i (+ i 1)))", env, &ctx)
            .expect("the child's exhaustion must not touch the parent's counter");
    }

    #[test]
    fn a_spawned_thread_is_armed_from_the_meter_not_the_compile_time_default() {
        // A tiny configured budget. Had `(spawn ...)` not armed the child, it
        // would have started on DEFAULT_FUEL -- five thousand times larger --
        // and got orders of magnitude further before dying. Counting iterations
        // makes that observable without timing anything.
        let (env, ctx) = fuel_env(2_000);
        eval_script("(spawn (lambda () (while t (tick!))))", env, &ctx).expect("spawn failed");
        assert!(
            wait_for(&ctx, "OutOfFuel"),
            "the child never reported running out of fuel"
        );

        let ticks = ctx.ticks.load(Ordering::Relaxed);
        assert!(
            ticks < 2_000,
            "the child got through {ticks} iterations on a 2000-step budget -- it was left on \
             DEFAULT_FUEL instead of being armed from the meter"
        );
    }
}
