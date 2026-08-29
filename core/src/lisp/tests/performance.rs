//! Performance characterisation of the interpreter itself.
//!
//! Runs against the bare `()` host context rather than `EditorState`, so what
//! is measured is the evaluator and nothing else: no buffers, no keymaps, no
//! fuel accounting, no diagnostic logging. When a number here moves, it moved
//! because the interpreter changed.
//!
//! Every assertion is a ratio between two measurements taken in the same run --
//! see `crate::tests::bench_util` for why. Absolute timings are printed for
//! tracking, never asserted on.
#[cfg(test)]
mod tests {
    use crate::lisp::{Env, LispExp, Parser, eval, setup_base_env};
    use crate::tests::bench_util::{growth, time_median};
    use std::sync::Arc;
    use std::time::Duration;

    /// A root environment with the standard primitives installed.
    fn fresh_env() -> Arc<Env<()>> {
        let env = Env::new_root();
        setup_base_env(env.clone());
        env
    }

    /// Parse `src` (wrapped in an implicit `progn`) ahead of any timing, so
    /// that eval benchmarks measure evaluation and not parsing.
    fn parse(src: &str) -> LispExp<()> {
        Parser::new(&format!("(progn {src})"))
            .next()
            .unwrap_or_else(|e| panic!("failed to parse benchmark source: {e:?}\n{src}"))
    }

    fn eval_ok(ast: &LispExp<()>, env: &Arc<Env<()>>) -> LispExp<()> {
        eval(ast, env.clone(), &()).expect("benchmark script failed to evaluate")
    }

    /// Time the same script shape at `n` and `2n`, and report how the cost grew.
    /// `build` receives a size and returns the source to run at that size.
    fn growth_when_doubling(env: &Arc<Env<()>>, n: usize, build: impl Fn(usize) -> String) -> f64 {
        let small = parse(&build(n));
        let large = parse(&build(n * 2));
        let t_small = time_median(|| {
            eval_ok(&small, env);
        });
        let t_large = time_median(|| {
            eval_ok(&large, env);
        });
        println!("    n={n}: {t_small:?}   n={}: {t_large:?}", n * 2);
        growth(t_large, t_small)
    }

    // ---------------------------------------------------------------------
    // Complexity of the evaluator's core loop
    // ---------------------------------------------------------------------

    /// Evaluating twice as many loop iterations must cost about twice as much.
    ///
    /// This is the broadest health check there is: it exercises special-form
    /// dispatch, variable lookup and primitive invocation together, and would
    /// catch anything that made per-step cost depend on how much work has
    /// already been done (a growing environment chain, an unbounded call-frame
    /// stack, a cache that never evicts).
    #[test]
    fn eval_cost_is_linear_in_iterations() {
        let env = fresh_env();
        let g = growth_when_doubling(&env, 10_000, |n| {
            format!("(setq i 0) (while (< i {n}) (setq i (+ i 1)))")
        });
        println!("eval loop: doubling iterations cost {g:.2}x (linear is ~2x)");

        assert!(
            g < 3.0,
            "doubling the iteration count cost {g:.2}x rather than ~2x -- per-step evaluation \
             cost now depends on how long the program has been running"
        );
    }

    /// Parsing twice as much source must cost about twice as much.
    ///
    /// Parsing runs on every `eval-string`, every `eval-file` and every M-:, and
    /// it is also the step any future bytecode compiler would have to run before
    /// it could compile anything -- so its complexity is a floor on all of that.
    #[test]
    fn parser_cost_is_linear_in_source_size() {
        let build = |n: usize| {
            let mut src = String::from("(progn ");
            for i in 0..n {
                src.push_str(&format!(
                    "(defun helper-{i} (a b) \"Doc.\" (if (< a b) (+ a b) (list a b {i}))) "
                ));
            }
            src.push(')');
            src
        };

        let measure = |src: &str| -> Duration {
            time_median(|| {
                let ast: LispExp<()> = Parser::new(src).next().expect("source must parse");
                assert!(matches!(ast, LispExp::List(_)));
            })
        };

        // Sized so each parse takes tens of milliseconds. At a few hundred
        // forms the measurements were short enough that scheduler noise moved
        // the ratio between 1.7x and 3.7x run to run -- enough to trip the bound
        // below on a busy machine.
        let small_src = build(1_000);
        let large_src = build(2_000);
        let t_small = measure(&small_src);
        let t_large = measure(&large_src);
        let g = growth(t_large, t_small);
        println!(
            "parser: 1000 forms {t_small:?}, 2000 forms {t_large:?} -> {g:.2}x (linear is ~2x)"
        );

        assert!(
            g < 3.0,
            "doubling the source size cost {g:.2}x rather than ~2x -- parsing is no longer linear"
        );
    }

    /// Tail calls are eliminated by the trampoline in `eval`: a call in tail
    /// position returns `EvalStep::TailCall` and is looped on rather than
    /// recursing in Rust. This checks both consequences at once.
    ///
    /// Correctness: 20,000 frames deep must not overflow the Rust stack.
    /// Cost: doubling the depth must roughly double the time. A super-linear
    /// result would mean frames are being retained somewhere they should not be
    /// -- for instance environments chaining onto the caller's rather than onto
    /// the closure's definition environment, which would also make variable
    /// lookup inside deep recursion quadratic.
    #[test]
    fn tail_recursion_is_linear_in_depth_and_uses_constant_stack() {
        let env = fresh_env();
        eval_ok(
            &parse("(defun countdown (n) (if (= n 0) 0 (countdown (- n 1))))"),
            &env,
        );

        let g = growth_when_doubling(&env, 10_000, |n| format!("(countdown {n})"));
        println!("tail recursion: doubling depth cost {g:.2}x (linear is ~2x)");

        assert!(
            g < 3.0,
            "doubling tail-recursion depth cost {g:.2}x rather than ~2x -- tail calls may no \
             longer run in constant space"
        );
    }

    // ---------------------------------------------------------------------
    // Cost of the language's building blocks, relative to each other
    // ---------------------------------------------------------------------

    /// Variable lookup walks the `Env` parent chain, taking an `RwLock` read and
    /// a `HashMap` probe at *every* level until it hits. So what a variable
    /// access costs depends on how deeply nested the code touching it is -- and
    /// since `let` and every function call push a frame, real code is never at
    /// depth zero.
    ///
    /// Runs an identical loop at depth 0 and depth 12. If lexical addressing
    /// ever lands -- resolving a name to a fixed frame and slot once, instead of
    /// re-searching by name on every access -- this ratio collapses toward 1.0x,
    /// and this test is how you would demonstrate the win.
    #[test]
    fn variable_lookup_cost_grows_with_scope_depth() {
        const ITERS: usize = 4_000;
        const DEPTH: usize = 12;

        let env = fresh_env();
        // `i` is created at the root first, so the `setq` inside the nested lets
        // updates that root binding -- walking the whole chain to reach it --
        // rather than shadowing it locally.
        eval_ok(&parse("(setq i 0)"), &env);

        let nested = |depth: usize| -> LispExp<()> {
            let mut src = String::new();
            for d in 0..depth {
                src.push_str(&format!("(let ((pad{d} 0)) "));
            }
            src.push_str(&format!(
                "(setq i 0) (while (< i {ITERS}) (setq i (+ i 1)))"
            ));
            src.push_str(&")".repeat(depth));
            parse(&src)
        };

        let shallow_ast = nested(0);
        let deep_ast = nested(DEPTH);
        let shallow = time_median(|| {
            eval_ok(&shallow_ast, &env);
        });
        let deep = time_median(|| {
            eval_ok(&deep_ast, &env);
        });

        let g = growth(deep, shallow);
        println!("variable lookup: depth 0 {shallow:?}, depth {DEPTH} {deep:?} -> {g:.2}x");

        assert!(
            g < 12.0,
            "running inside {DEPTH} nested scopes now costs {g:.2}x running at the top level -- \
             scope-chain lookup got materially worse"
        );
    }

    /// What a user-defined function call costs over and above doing the same
    /// arithmetic inline: argument evaluation, a child `Env` allocation,
    /// parameter binding, and the trampolined body.
    ///
    /// The number that would move if call frames were pooled, or if binding
    /// stopped allocating a `HashMap` per call.
    #[test]
    fn function_call_overhead_over_inline_arithmetic() {
        const ITERS: usize = 4_000;

        let env = fresh_env();
        eval_ok(&parse("(defun inc (x) (+ x 1))"), &env);

        let inline_ast = parse(&format!(
            "(setq i 0) (while (< i {ITERS}) (setq i (+ i 1)))"
        ));
        let called_ast = parse(&format!(
            "(setq i 0) (while (< i {ITERS}) (setq i (inc i)))"
        ));

        let inline = time_median(|| {
            eval_ok(&inline_ast, &env);
        });
        let called = time_median(|| {
            eval_ok(&called_ast, &env);
        });

        let g = growth(called, inline);
        println!("call overhead: inline {inline:?}, via (inc i) {called:?} -> {g:.2}x");

        assert!(
            g < 8.0,
            "a user-defined call now costs {g:.2}x an inline primitive call -- call setup got \
             materially more expensive"
        );
    }

    /// Macros are re-expanded on *every* evaluation -- expansion is not cached
    /// anywhere -- so a macro call pays to rebuild its expansion before it can
    /// run it, every single time. Measured against the equivalent function call.
    ///
    /// If expansion caching is ever added (the natural companion to caching
    /// dispatch decisions on AST nodes), this ratio is how you would show it
    /// worked.
    #[test]
    fn macro_expansion_overhead_over_function_call() {
        const ITERS: usize = 2_000;

        let env = fresh_env();
        eval_ok(&parse("(defun add-fn (a b) (+ a b))"), &env);
        eval_ok(&parse("(defmacro add-macro (a b) (list '+ a b))"), &env);

        let fn_ast = parse(&format!(
            "(setq i 0) (while (< i {ITERS}) (setq i (add-fn i 1)))"
        ));
        let macro_ast = parse(&format!(
            "(setq i 0) (while (< i {ITERS}) (setq i (add-macro i 1)))"
        ));

        let via_fn = time_median(|| {
            eval_ok(&fn_ast, &env);
        });
        let via_macro = time_median(|| {
            eval_ok(&macro_ast, &env);
        });

        let g = growth(via_macro, via_fn);
        println!("macro overhead: function {via_fn:?}, macro {via_macro:?} -> {g:.2}x");

        assert!(
            g < 6.0,
            "macro expansion now costs {g:.2}x a plain function call per invocation"
        );
    }

    /// Documents a structural property of the current value representation
    /// rather than a regression: **`cons` is O(n), not O(1)**.
    ///
    /// Lists are `LispExp::List(Arc<Vec<LispExp>>)` -- a flat vector -- rather
    /// than genuine cons cells sharing a tail. So `primitive_cons` cannot link a
    /// new head onto an existing tail; it allocates a fresh vector and copies
    /// every element. The most fundamental Lisp operation therefore costs time
    /// proportional to the list it extends, which makes *any* loop accumulating
    /// n elements O(n^2) -- including the `cons`-then-`reverse` idiom that exists
    /// in real Lisp precisely *because* it is supposed to be the linear one.
    ///
    /// That reaches well past microbenchmarks: it is why a Lisp-side `grep`
    /// collecting matches, or any library function accumulating results,
    /// degrades sharply with result count.
    ///
    /// The measurement subtracts a control loop running the same iterations with
    /// O(1) work per iteration, so what remains is the list work alone rather
    /// than interpreter overhead. The assertion pins the *current* quadratic
    /// behaviour: if lists ever move to a representation with an O(1) `cons`,
    /// this starts failing, and that failure is the signal to flip it into an
    /// assertion of linearity.
    #[test]
    fn cons_copies_so_list_accumulation_is_quadratic() {
        // Large enough that the quadratic term dominates the noise left after
        // the subtraction below; at smaller sizes the result wobbles close
        // enough to linear to be unreliable.
        const N: usize = 2_000;

        let env = fresh_env();

        // Identical loop shape in both, so subtracting one from the other
        // cancels the per-iteration interpreter cost and leaves the `cons`es.
        let list_work_at = |n: usize| -> Duration {
            let control = parse(&format!(
                "(setq acc nil) (setq i 0) (while (< i {n}) (setq i (+ i 1))) i"
            ));
            let with_cons = parse(&format!(
                "(setq acc nil) (setq i 0) \
                 (while (< i {n}) (setq acc (cons i acc)) (setq i (+ i 1))) \
                 (length acc)"
            ));
            let control_time = time_median(|| {
                eval_ok(&control, &env);
            });
            let cons_time = time_median(|| {
                assert_eq!(eval_ok(&with_cons, &env), LispExp::number(n as f64));
            });
            cons_time.saturating_sub(control_time)
        };

        let small = list_work_at(N);
        let large = list_work_at(N * 2);
        let g = growth(large, small);
        println!(
            "list accumulation (interpreter overhead subtracted): {N} elements {small:?}, {} \
             elements {large:?} -> {g:.2}x (O(1) cons would be ~2x, copying cons ~4x)",
            N * 2
        );

        assert!(
            g > 2.6,
            "accumulating a list now scales at {g:.2}x when doubling its length, close to \
             linear -- if `cons` became O(1), invert this test to assert linearity and tighten it"
        );
    }
}
