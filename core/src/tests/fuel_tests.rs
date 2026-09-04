//! Correctness of the editor's fuel policy.
//!
//! These moved out of the performance suite, where they had been sitting
//! because fuel was measured with a clock. Neither one measures anything: both
//! assert an invariant that either holds or does not, and belong with the rest
//! of the behaviour tests. What a command actually *costs* is section 4 of
//! `performance-cost.txt`.
#[cfg(test)]
mod tests {
    use crate::{
        buffer::{BufferTrait, gap_buffer::GapBuffer},
        editor::create_global_env,
        input::{KeyCode, KeyEvent, KeyModifiers},
        lisp::{EvalError, Parser, eval},
    };

    fn char_event(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::default(),
        }
    }

    /// Regression guard for the bug the `FuelMeter` exists to fix: fuel used to
    /// be a single budget for the entire session, never replenished, so after
    /// roughly five thousand keystrokes every further evaluation failed with
    /// `OutOfFuel` and the editor stopped accepting input while still running.
    #[test]
    fn fuel_is_replenished_for_every_command() {
        const ATTEMPTS: usize = 20_000;

        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let scratch = state
            .get_buffer("*scratch*")
            .expect("*scratch* buffer must exist");
        let before = scratch.read().unwrap().text.len();

        for _ in 0..ATTEMPTS {
            state.handle_key_event(char_event('a'), &env);
        }

        let inserted = scratch.read().unwrap().text.len() - before;
        let starved = state
            .get_logs()
            .iter()
            .filter(|line| line.contains("OutOfFuel"))
            .count();

        assert_eq!(
            inserted, ATTEMPTS,
            "only {inserted} of {ATTEMPTS} keystrokes were accepted -- fuel is not being \
             refilled per command"
        );
        assert_eq!(starved, 0, "no keystroke should exhaust a fresh budget");
    }

    /// Refilling per command must not amount to removing the guard.
    #[test]
    fn a_runaway_loop_is_still_aborted() {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        state.set_fuel_budget(50_000);

        let ast = Parser::new("(while t 1)")
            .next()
            .expect("test source must parse");

        assert_eq!(
            eval(&ast, env.clone(), &state).unwrap_err(),
            EvalError::OutOfFuel,
            "an infinite loop must still be stopped by the fuel guard"
        );
    }
}
