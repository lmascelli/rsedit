//! Finding every match: where each is in the text, on which line and where in
//! the line -- three answers one walk computes -- checked where they can
//! disagree: several matches on a line, a match across a line break, text that
//! is not ASCII, a pattern that matches nothing everywhere. Then what Lisp sees.
//!
//! Design: `doc/architettura-editor.typ, <ricerca>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::text::search::Pattern;
    use risp::{Env, EvalError, LispExp, Parser, eval};
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
        // One pass: `search_forward` for every match would copy the whole text
        // once per match, seconds at this size against milliseconds.
        let line = "some text with a needle in it and more text after it\n";
        let haystack = line.repeat(20_000);
        let pattern = Pattern::new("needle", false, false).expect("pattern");
        let scan = pattern.scan(&haystack, 50_000);
        assert_eq!(scan.found.len(), 20_000);
        assert_eq!(scan.found[19_999].line, 20_000, "the line count kept up");
    }

    /// The column is counted as the scan goes, not from the start of the line
    /// at every match, which cost one long line with many matches its length
    /// once per match: right past characters that are not ASCII, and from 0
    /// again on the next line.
    #[test]
    fn columns_on_one_long_line_are_counted_as_the_scan_goes() {
        let line = "é a ".repeat(2_000);
        let haystack = format!("first\n{line}\nend a");
        let pattern = Pattern::new("a", false, false).expect("pattern");
        let scan = pattern.scan(&haystack, 100_000);
        assert_eq!(scan.found.len(), 2_001);
        assert_eq!(scan.found[0].column, 2);
        assert_eq!(scan.found[1_999].column, 1_999 * 4 + 2);
        assert_eq!(
            (scan.found[2_000].line, scan.found[2_000].column),
            (3, 4),
            "a new line starts the count again"
        );
    }

    // ----------------------------------------------------------------
    // Searching backward, and up to a limit
    // ----------------------------------------------------------------

    /// Every step of the walk copied the text again: `C-M-r` on a file of
    /// some hundred KB with many matches held the editor for minutes a key.
    #[test]
    fn a_regexp_search_backward_is_one_pass() {
        let buffer = GapBuffer::from("ab ".repeat(40_000).as_str());
        let pattern = Pattern::new("b", true, false).expect("pattern");
        let started = std::time::Instant::now();
        let found = pattern
            .search_backward(&buffer, buffer.len(), 0)
            .expect("a match");
        let took = started.elapsed();
        assert_eq!((found.start, found.end), (119_998, 119_999));
        assert!(
            took < std::time::Duration::from_secs(5),
            "{} characters took {took:?}",
            buffer.len()
        );
    }

    /// A match running past the limit was dropped, or stopped the backward
    /// walk, though a shorter one fitted.
    #[test]
    fn a_regexp_finds_what_fits_before_the_limit() {
        let buffer = GapBuffer::from("aaaa");
        let pattern = Pattern::new("a+", true, false).expect("pattern");
        let forward = pattern.search_forward(&buffer, 0, 2).expect("a match");
        assert_eq!((forward.start, forward.end), (0, 2));
        let backward = pattern.search_backward(&buffer, 2, 0).expect("a match");
        assert_eq!((backward.start, backward.end), (0, 2));
        let words = GapBuffer::from("ab ab ab");
        let ab = Pattern::new("ab", true, false).expect("pattern");
        let last = ab.search_backward(&words, 7, 0).expect("a match");
        assert_eq!(
            (last.start, last.end),
            (3, 5),
            "the one cut by the limit does not fit"
        );
    }

    /// The groups and offsets of the match kept are those of the text, in
    /// characters, past text that is not ASCII.
    #[test]
    fn a_regexp_search_backward_answers_in_characters() {
        let buffer = GapBuffer::from("é(1) è(22) x");
        let pattern = Pattern::new(r"\((\d+)\)", true, false).expect("pattern");
        let found = pattern
            .search_backward(&buffer, buffer.len(), 0)
            .expect("a match");
        assert_eq!((found.start, found.end), (6, 10));
        assert_eq!(found.groups[1].as_deref(), Some("22"));
        assert!(
            pattern.search_backward(&buffer, 5, 2).is_none(),
            "(1) starts before the floor"
        );
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
        let (_, cheap) = risp::measure(&ctx.fuel_meter(), || {
            run(r#"(scan-buffer "needle")"#, &env, &ctx)
        });
        let (ctx, env) = editor(&"x".repeat(50_000));
        let (_, dear) = risp::measure(&ctx.fuel_meter(), || {
            run(r#"(scan-buffer "needle")"#, &env, &ctx)
        });
        assert!(
            dear > cheap * 5,
            "ten times the text should cost about ten times the fuel: {cheap} then {dear}"
        );
    }
}
