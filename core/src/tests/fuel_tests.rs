//! The editor's fuel: what holds or does not. What a command costs is in the
//! report the performance suite writes, `performance-cost.txt`.
#[cfg(test)]
mod tests {
    use crate::{
        buffer::{BufferTrait, gap_buffer::GapBuffer},
        editor::create_global_env,
        input::{KeyCode, KeyEvent, KeyModifiers},
    };
    use risp::{EvalError, Parser, eval};

    fn char_event(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::default(),
        }
    }

    /// Fuel is refilled for every command: one budget for the whole session
    /// would run out after some thousands of keys, and every evaluation after
    /// that would fail with `OutOfFuel`, the editor running and taking no
    /// input.
    #[test]
    fn fuel_is_replenished_for_every_command() {
        const ATTEMPTS: usize = 20_000;

        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let length = || {
            state
                .with_buffer("*scratch*", |b| b.text.len())
                .expect("*scratch* buffer must exist")
        };
        let before = length();

        for _ in 0..ATTEMPTS {
            state.handle_key_event(char_event('a'), &env);
        }

        let inserted = length() - before;
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
