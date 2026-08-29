#[cfg(test)]
mod tests {
    use crate::lisp::*;

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
}
