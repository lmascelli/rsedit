//! Finding every match: the arithmetic, and the shape of what comes back.
//!
//! # What is worth testing here
//!
//! A scan is a loop that has to be right about three things at once -- where a
//! match is in the whole text, which line it is on, and where it sits within
//! that line -- and the three are computed by one walk that only ever moves
//! forward. Every way of getting that wrong is quiet: an off-by-one in the
//! line counter shows up as a result list that opens the wrong line, which
//! reads as "the search is broken" long after the scan itself is forgotten.
//!
//! So the cases below are the ones where the three answers can disagree:
//! several matches on one line, a match that spans a line break, text that is
//! not ASCII (where a byte offset and a character offset part company), and a
//! pattern that matches nothing at all at every position.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::text::search::Pattern;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn editor(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        ctx.with_current_buffer_mut(|buf| {
            buf.text = GapBuffer::from(text);
            buf.text.cursor_move(0, 0);
            buf.is_modified = false;
        });
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

    /// One entry, flattened: (KIND SOURCE LINE COLUMN OFFSET TEXT START END).
    fn fields(entry: &LispExp<Ctx>) -> Vec<String> {
        entry
            .iter()
            .map(|item| match item {
                LispExp::String(text) => text.to_string(),
                LispExp::Number(n) => format!("{n}"),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// The entries of a `(TRUNCATED ENTRIES)` answer.
    fn entries(answer: &LispExp<Ctx>) -> Vec<Vec<String>> {
        let parts: Vec<LispExp<Ctx>> = answer.iter().collect();
        assert_eq!(parts.len(), 2, "a scan answers (TRUNCATED ENTRIES)");
        parts[1].iter().map(|entry| fields(&entry)).collect()
    }

    fn truncated(answer: &LispExp<Ctx>) -> bool {
        let parts: Vec<LispExp<Ctx>> = answer.iter().collect();
        !parts[0].is_nil()
    }

    // ----------------------------------------------------------------
    // The scanner itself
    // ----------------------------------------------------------------

    #[test]
    fn a_literal_scan_reports_every_match_in_order() {
        let pattern = Pattern::new("cat", false, false).expect("pattern");
        let scan = pattern.scan("cat dog cat", 100);
        assert!(!scan.truncated);
        assert_eq!(
            scan.found
                .iter()
                .map(|f| (f.start, f.end))
                .collect::<Vec<_>>(),
            vec![(0, 3), (8, 11)]
        );
    }

    #[test]
    fn a_match_knows_its_line_its_column_and_the_line_it_is_on() {
        let pattern = Pattern::new("x", false, false).expect("pattern");
        let scan = pattern.scan("one\ntwo x here\nthree", 100);
        let found = &scan.found[0];
        assert_eq!(found.line, 2, "lines count from 1");
        assert_eq!(found.column, 4, "columns count from 0");
        assert_eq!(found.start, 8);
        assert_eq!(found.line_text, "two x here");
        assert_eq!((found.in_line(), found.end_in_line()), (4, 5));
    }

    #[test]
    fn several_matches_on_one_line_each_get_their_own_column() {
        // The cached line text is the optimisation that makes twenty matches
        // on a line cost one walk to its end; a cache that also froze the
        // column would give them all the first one's.
        let pattern = Pattern::new("a", false, false).expect("pattern");
        let scan = pattern.scan("banana\n", 100);
        assert_eq!(
            scan.found
                .iter()
                .map(|f| (f.line, f.column))
                .collect::<Vec<_>>(),
            vec![(1, 1), (1, 3), (1, 5)]
        );
        assert!(scan.found.iter().all(|f| f.line_text == "banana"));
    }

    #[test]
    fn matches_on_different_lines_each_carry_their_own_line() {
        // The line text is cached so that twenty matches on one line cost one
        // walk to its end. A cache that did not check *which* line it held
        // would hand every later match the first one's text -- a results list
        // where every entry shows the same line while claiming different
        // numbers, which reads as a display fault rather than a scan one.
        let pattern = Pattern::new("x", false, false).expect("pattern");
        let scan = pattern.scan("one x\ntwo x\nthree x\n", 100);
        assert_eq!(
            scan.found
                .iter()
                .map(|f| (f.line, f.line_text.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, "one x".to_string()),
                (2, "two x".to_string()),
                (3, "three x".to_string()),
            ]
        );
    }

    #[test]
    fn the_last_line_has_no_newline_to_end_it() {
        let pattern = Pattern::new("end", false, false).expect("pattern");
        let scan = pattern.scan("one\nthe end", 100);
        assert_eq!(scan.found[0].line, 2);
        assert_eq!(scan.found[0].line_text, "the end");
    }

    #[test]
    fn a_match_spanning_a_line_break_is_reported_where_it_starts() {
        // And the end within the line is clamped to the line, or a view
        // highlighting it would be asked for characters it is not showing.
        let pattern = Pattern::new("two\\nthree", true, false).expect("pattern");
        let scan = pattern.scan("one\ntwo\nthree\n", 100);
        let found = &scan.found[0];
        assert_eq!(found.line, 2);
        assert_eq!(found.line_text, "two");
        assert_eq!(found.end_in_line(), 3, "clamped to the line it shows");
    }

    #[test]
    fn offsets_are_characters_and_not_bytes() {
        // The failure this prevents: a match reported in bytes lands in the
        // middle of a letter, and `goto-char' puts point there.
        let pattern = Pattern::new("cat", false, false).expect("pattern");
        let scan = pattern.scan("héllo wörld cat", 100);
        assert_eq!(scan.found[0].start, 12, "twelve characters, not bytes");
        assert_eq!(scan.found[0].column, 12);
    }

    #[test]
    fn a_multi_byte_pattern_is_found_and_measured_in_characters() {
        let pattern = Pattern::new("ö", false, false).expect("pattern");
        let scan = pattern.scan("wörld wörld", 100);
        assert_eq!(
            scan.found
                .iter()
                .map(|f| (f.start, f.end))
                .collect::<Vec<_>>(),
            vec![(1, 2), (7, 8)]
        );
    }

    #[test]
    fn a_pattern_that_matches_nothing_still_terminates() {
        // `x*` matches the empty string at every position. A scan that did
        // not step past an empty match would sit on one for ever -- the same
        // rule the replace loop follows.
        let pattern = Pattern::new("x*", true, false).expect("pattern");
        let scan = pattern.scan("abc", 100);
        assert!(scan.found.len() <= 4, "one per position at most: {scan:?}");
        assert!(scan.found.iter().all(|f| f.start == f.end));
    }

    #[test]
    fn matches_do_not_overlap() {
        let pattern = Pattern::new("aa", false, false).expect("pattern");
        let scan = pattern.scan("aaaa", 100);
        assert_eq!(
            scan.found.iter().map(|f| f.start).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn a_limit_stops_the_scan_and_says_so() {
        let pattern = Pattern::new("a", false, false).expect("pattern");
        let scan = pattern.scan("aaaaa", 3);
        assert_eq!(scan.found.len(), 3);
        assert!(scan.truncated, "a partial list must say it is partial");

        let whole = pattern.scan("aaaaa", 5);
        assert!(!whole.truncated, "an exact fit is not truncated");
    }

    #[test]
    fn folding_is_the_callers_decision() {
        let folded = Pattern::new("cat", false, true).expect("pattern");
        assert_eq!(folded.scan("Cat CAT cat", 100).found.len(), 3);
        let exact = Pattern::new("cat", false, false).expect("pattern");
        assert_eq!(exact.scan("Cat CAT cat", 100).found.len(), 1);
    }

    #[test]
    fn groups_come_back_with_the_match() {
        let pattern = Pattern::new("(\\w+)@(\\w+)", true, false).expect("pattern");
        let scan = pattern.scan("write to ada@example now", 100);
        assert_eq!(scan.found[0].groups[1].as_deref(), Some("ada"));
        assert_eq!(scan.found[0].groups[2].as_deref(), Some("example"));
    }

    #[test]
    fn scanning_a_large_buffer_is_one_pass_rather_than_one_per_match() {
        // The reason this function exists. `search_forward` stringifies the
        // whole text on every call, so asking it for every match costs a pass
        // per match: at these sizes that is 40 million character copies and
        // several seconds. One pass is milliseconds, and the difference is
        // what this test would show as a hang.
        let line = "some text with a needle in it and more text after it\n";
        let haystack = line.repeat(20_000);
        let pattern = Pattern::new("needle", false, false).expect("pattern");
        let scan = pattern.scan(&haystack, 50_000);
        assert_eq!(scan.found.len(), 20_000);
        assert_eq!(scan.found[19_999].line, 20_000, "the line count kept up");
    }

    // ----------------------------------------------------------------
    // What Lisp sees
    // ----------------------------------------------------------------

    #[test]
    fn an_entry_carries_everything_a_view_needs_to_show_it() {
        let (ctx, env) = editor("one\ntwo TODO here\n");
        let answer = run(r#"(scan-buffer "TODO")"#, &env, &ctx);
        assert!(!truncated(&answer));
        let found = entries(&answer);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0],
            vec![
                "buffer".to_string(),
                "*scratch*".to_string(),
                "2".to_string(),
                "4".to_string(),
                "8".to_string(),
                "two TODO here".to_string(),
                "4".to_string(),
                "8".to_string(),
            ]
        );
    }

    #[test]
    fn the_offset_is_one_goto_char_can_use() {
        // The whole point of reporting it: a view opens the result by going
        // there, and a position that needed adjusting first would be a
        // position every view had to adjust the same way.
        let (ctx, env) = editor("one\ntwo TODO here\n");
        let found = entries(&run(r#"(scan-buffer "TODO")"#, &env, &ctx));
        let offset: usize = found[0][4].parse().expect("a number");
        run(&format!("(goto-char {offset})"), &env, &ctx);
        assert_eq!(
            run("(list (line-number-at-point) (current-column))", &env, &ctx),
            run("'(2 4)", &env, &ctx)
        );
    }

    #[test]
    fn a_regexp_scan_is_asked_for_explicitly() {
        let (ctx, env) = editor("a1 b2 c3\n");
        assert_eq!(
            entries(&run(r#"(scan-buffer "[a-z][0-9]")"#, &env, &ctx)).len(),
            0
        );
        assert_eq!(
            entries(&run(r#"(scan-buffer "[a-z][0-9]" t)"#, &env, &ctx)).len(),
            3
        );
    }

    #[test]
    fn folding_follows_case_fold_search_like_every_other_search() {
        let (ctx, env) = editor("cat Cat CAT\n");
        run("(setq case-fold-search nil)", &env, &ctx);
        assert_eq!(entries(&run(r#"(scan-buffer "cat")"#, &env, &ctx)).len(), 1);
        run("(setq case-fold-search t)", &env, &ctx);
        assert_eq!(entries(&run(r#"(scan-buffer "cat")"#, &env, &ctx)).len(), 3);
    }

    #[test]
    fn a_limit_is_reported_to_lisp_as_well() {
        let (ctx, env) = editor("a a a a a\n");
        let answer = run(r#"(scan-buffer "a" nil 2)"#, &env, &ctx);
        assert!(truncated(&answer), "the caller has to be able to tell");
        assert_eq!(entries(&answer).len(), 2);
    }

    #[test]
    fn another_buffer_can_be_scanned_without_switching_to_it() {
        let (ctx, env) = editor("nothing here\n");
        run(
            r#"(buffer-create "*other*")
               (with-current-buffer "*other*" (lambda () (insert "a needle\n")))"#,
            &env,
            &ctx,
        );
        let found = entries(&run(
            r#"(scan-buffer "needle" nil nil "*other*")"#,
            &env,
            &ctx,
        ));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0][1], "*other*", "the entry names where it came from");
        assert_eq!(
            run("(current-buffer)", &env, &ctx),
            LispExp::string("*scratch*".into()),
            "scanning should not move anybody"
        );
    }

    #[test]
    fn a_buffer_that_is_not_there_is_reported_rather_than_invented() {
        let (ctx, env) = editor("");
        assert_eq!(
            run(r#"(scan-buffer "x" nil nil "*no-such*")"#, &env, &ctx),
            LispExp::nil()
        );
    }

    #[test]
    fn a_bad_regexp_signals_rather_than_answering_nothing() {
        // Answering "no matches" would be indistinguishable from a pattern
        // that is simply not there, which is the worst way to learn about a
        // typo in a regexp.
        let (ctx, env) = editor("text");
        let failed = try_run(r#"(scan-buffer "(unclosed" t)"#, &env, &ctx);
        assert!(
            matches!(failed, Err(EvalError::RuntimeMessage(_))),
            "a bad regexp should be reported: {failed:?}"
        );
    }

    #[test]
    fn an_empty_pattern_is_refused() {
        let (ctx, env) = editor("text");
        assert!(try_run(r#"(scan-buffer "")"#, &env, &ctx).is_err());
    }

    #[test]
    fn a_scan_costs_fuel_in_proportion_to_what_it_walked() {
        // A primitive that scanned a megabyte for one unit would let a loop
        // do unbounded work inside a budget meant to bound it.
        let (ctx, env) = editor(&"x".repeat(5_000));
        let (_, cheap) = crate::lisp::measure(&ctx.fuel_meter(), || {
            run(r#"(scan-buffer "needle")"#, &env, &ctx)
        });
        let (ctx, env) = editor(&"x".repeat(50_000));
        let (_, dear) = crate::lisp::measure(&ctx.fuel_meter(), || {
            run(r#"(scan-buffer "needle")"#, &env, &ctx)
        });
        assert!(
            dear > cheap * 5,
            "ten times the text should cost about ten times the fuel: {cheap} then {dear}"
        );
    }
}
