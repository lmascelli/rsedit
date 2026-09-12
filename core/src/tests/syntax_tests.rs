//! The grammar, the lexer that applies it, the cache that remembers it, and
//! the worker that fills the cache in.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::modes::{Grammar, SyntaxRegion, SyntaxRule, SyntaxState, highlight_line};
    use crate::ui::Face;
    use regex::Regex;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
            b.is_modified = false;
        });
        (ctx, env)
    }

    fn rule(pattern: &str, face: Face) -> SyntaxRule {
        SyntaxRule {
            pattern: Regex::new(pattern).expect("test pattern"),
            face,
            group: 0,
        }
    }

    fn group_rule(pattern: &str, face: Face, group: usize) -> SyntaxRule {
        SyntaxRule {
            pattern: Regex::new(pattern).expect("test pattern"),
            face,
            group,
        }
    }

    fn region(begin: &str, end: &str, face: Face) -> SyntaxRegion {
        SyntaxRegion {
            begin: Regex::new(begin).expect("test pattern"),
            end: Regex::new(end).expect("test pattern"),
            escape: None,
            face,
            nestable: false,
        }
    }

    /// The faces of one line, as `(start, end, face-name)`, which reads far
    /// better in a failure than a list of structs.
    fn faces(grammar: &Grammar, line: &str, entering: &SyntaxState) -> Vec<(usize, usize, String)> {
        highlight_line(grammar, line, entering)
            .0
            .into_iter()
            .map(|span| (span.start, span.end, span.face.name().to_string()))
            .collect()
    }

    fn state(grammar: &Grammar, line: &str, entering: &SyntaxState) -> SyntaxState {
        highlight_line(grammar, line, entering).1
    }

    // -----------------------------------------------------------------------
    // Rules
    // -----------------------------------------------------------------------

    #[test]
    fn a_rule_colours_what_it_matches() {
        let grammar = Grammar {
            rules: vec![rule(r"\bfn\b", Face::KEYWORD)],
            regions: vec![],
        };
        assert_eq!(
            faces(&grammar, "pub fn main", &vec![]),
            vec![(4, 6, "keyword".to_string())]
        );
    }

    /// The reason `group` exists: `\b\w+(?=\()` is unavailable, so a rule
    /// matches the parenthesis too and faces only the name.
    #[test]
    fn a_rule_can_face_a_capture_group_rather_than_the_whole_match() {
        let grammar = Grammar {
            rules: vec![group_rule(r"\b([a-z_]\w*)\s*\(", Face::FUNCTION, 1)],
            regions: vec![],
        };
        assert_eq!(
            faces(&grammar, "call foo ()", &vec![]),
            vec![(5, 8, "function".to_string())],
            "the name is coloured and the parenthesis is not"
        );
    }

    /// The scan still steps over the whole match, or the parenthesis the rule
    /// consumed would be offered to the next pattern.
    #[test]
    fn a_group_rule_consumes_everything_it_matched() {
        let grammar = Grammar {
            rules: vec![
                group_rule(r"\b([a-z_]\w*)\s*\(", Face::FUNCTION, 1),
                rule(r"\(", Face::BUILTIN),
            ],
            regions: vec![],
        };
        assert_eq!(
            faces(&grammar, "foo()", &vec![]),
            vec![(0, 3, "function".to_string())],
            "the parenthesis was consumed, so the second rule never sees it"
        );
    }

    #[test]
    fn earlier_rules_win_where_two_match_at_the_same_place() {
        let grammar = Grammar {
            rules: vec![
                rule(r"\blet\b", Face::KEYWORD),
                rule(r"\b[a-z]+\b", Face::TYPE),
            ],
            regions: vec![],
        };
        assert_eq!(
            faces(&grammar, "let", &vec![]),
            vec![(0, 3, "keyword".to_string())]
        );
    }

    /// A rule that can match nothing would otherwise be found at the same
    /// position forever.
    #[test]
    fn a_rule_that_matches_the_empty_string_terminates() {
        let grammar = Grammar {
            rules: vec![rule("x*", Face::KEYWORD)],
            regions: vec![],
        };
        // The point is that this returns at all.
        let (_, leaving) = highlight_line(&grammar, "abc", &vec![]);
        assert!(leaving.is_empty());
    }

    // -----------------------------------------------------------------------
    // Regions
    // -----------------------------------------------------------------------

    #[test]
    fn a_region_colours_its_delimiters_too() {
        let grammar = Grammar {
            rules: vec![],
            regions: vec![region("/\\*", "\\*/", Face::COMMENT)],
        };
        assert_eq!(
            faces(&grammar, "a /* b */ c", &vec![]),
            vec![(2, 9, "comment".to_string())]
        );
    }

    /// The whole reason for a state: a region left open colours the next line.
    #[test]
    fn a_region_that_does_not_close_stays_open_into_the_next_line() {
        let grammar = Grammar {
            rules: vec![],
            regions: vec![region("/\\*", "\\*/", Face::COMMENT)],
        };
        let after_first = state(&grammar, "code /* opens", &vec![]);
        assert_eq!(after_first, vec![0], "the region is still open");

        assert_eq!(
            faces(&grammar, "still inside", &after_first),
            vec![(0, 12, "comment".to_string())],
            "and the whole of the next line belongs to it"
        );
        assert_eq!(
            state(&grammar, "closes */ here", &after_first),
            SyntaxState::new(),
            "until it closes"
        );
    }

    #[test]
    fn a_region_closing_mid_line_gives_the_rest_back() {
        let grammar = Grammar {
            rules: vec![rule(r"\bfn\b", Face::KEYWORD)],
            regions: vec![region("/\\*", "\\*/", Face::COMMENT)],
        };
        assert_eq!(
            faces(&grammar, "*/ fn", &vec![0]),
            vec![(0, 2, "comment".to_string()), (3, 5, "keyword".to_string()),]
        );
    }

    /// The mistake the whole scan is arranged to avoid: scanning regions first
    /// would open a block comment inside a line comment and swallow the file.
    #[test]
    fn a_line_comment_swallows_a_block_comment_opener() {
        let grammar = Grammar {
            rules: vec![rule("//.*", Face::COMMENT)],
            regions: vec![region("/\\*", "\\*/", Face::COMMENT)],
        };
        assert_eq!(
            state(&grammar, "// this /* is not a comment start", &vec![]),
            SyntaxState::new(),
            "no region may be left open by text inside a line comment"
        );
    }

    #[test]
    fn a_nestable_region_closes_where_it_should() {
        let mut nested = region("/\\*", "\\*/", Face::COMMENT);
        nested.nestable = true;
        let grammar = Grammar {
            rules: vec![rule(r"\bfn\b", Face::KEYWORD)],
            regions: vec![nested],
        };

        assert_eq!(
            state(&grammar, "/* a /* b */", &vec![]),
            vec![0],
            "the inner close must not end the outer comment"
        );
        assert_eq!(
            state(&grammar, "/* a /* b */ c */ fn", &vec![]),
            SyntaxState::new(),
            "and the outer one closes at its own delimiter"
        );
    }

    #[test]
    fn a_region_that_does_not_nest_closes_at_the_first_end() {
        let grammar = Grammar {
            rules: vec![],
            regions: vec![region("/\\*", "\\*/", Face::COMMENT)],
        };
        assert_eq!(
            state(&grammar, "/* a /* b */", &vec![]),
            SyntaxState::new(),
            "without nestable, the first end delimiter wins"
        );
    }

    /// `(?<!\\)"` is unavailable, which is why a region names its escape.
    #[test]
    fn an_escape_stops_a_region_ending_early() {
        let mut string = region("\"", "\"", Face::STRING);
        string.escape = Some(Regex::new(r"\\.").expect("test pattern"));
        let grammar = Grammar {
            rules: vec![rule(r"\bfn\b", Face::KEYWORD)],
            regions: vec![string],
        };

        assert_eq!(
            faces(&grammar, r#""say \"hi\"" fn"#, &vec![]),
            vec![
                (0, 12, "string".to_string()),
                (13, 15, "keyword".to_string()),
            ],
            "the escaped quotes are inside the string"
        );
    }

    #[test]
    fn without_an_escape_the_region_ends_at_the_first_delimiter() {
        let grammar = Grammar {
            rules: vec![],
            regions: vec![region("\"", "\"", Face::STRING)],
        };
        assert_eq!(
            faces(&grammar, r#""a \" b""#, &vec![]),
            vec![(0, 5, "string".to_string()), (7, 8, "string".to_string()),],
            "the backslash-quote closes it, and the last quote opens another"
        );
    }

    /// Some languages escape the delimiter by doubling it -- `'it''s'` is one
    /// string, not two. The doubled pair matches at the *same* position as the
    /// end delimiter, so the escape has to win that tie.
    #[test]
    fn a_doubled_delimiter_escape_wins_its_tie_against_the_end() {
        let mut string = region("'", "'", Face::STRING);
        string.escape = Some(Regex::new("''").expect("test pattern"));
        let grammar = Grammar {
            rules: vec![rule(r"\bfn\b", Face::KEYWORD)],
            regions: vec![string],
        };

        assert_eq!(
            faces(&grammar, "'it''s' fn", &vec![]),
            vec![(0, 7, "string".to_string()), (8, 10, "keyword".to_string()),],
            "the doubled quote is inside the string, not the end of it"
        );
    }

    /// No rule applies inside a region: a string is uniformly a string. That
    /// is the v1 contract, and worth pinning so a later `contains` feature is
    /// a deliberate change rather than an accident.
    #[test]
    fn rules_do_not_apply_inside_a_region() {
        let grammar = Grammar {
            rules: vec![rule(r"\bfn\b", Face::KEYWORD)],
            regions: vec![region("\"", "\"", Face::STRING)],
        };
        assert_eq!(
            faces(&grammar, r#""fn""#, &vec![]),
            vec![(0, 4, "string".to_string())]
        );
    }

    /// Columns are characters. Bytes would put every span after a multi-byte
    /// character in the wrong place.
    #[test]
    fn spans_are_measured_in_characters_not_bytes() {
        let grammar = Grammar {
            rules: vec![rule(r"\bfn\b", Face::KEYWORD)],
            regions: vec![],
        };
        assert_eq!(
            faces(&grammar, "é é fn", &vec![]),
            vec![(4, 6, "keyword".to_string())],
            "two two-byte characters before it: bytes would say 6..8"
        );
    }

    // -----------------------------------------------------------------------
    // Defining a grammar from Lisp
    // -----------------------------------------------------------------------

    fn grammar_of(ctx: &Ctx, mode: &str) -> Grammar {
        ctx.mode_registry
            .read()
            .expect("mode registry")
            .get(mode)
            .expect("the mode")
            .grammar
            .clone()
    }

    #[test]
    fn lisp_can_define_rules_and_regions() {
        let (ctx, env) = editor_with("");
        eval_str(
            r#"(progn (make-mode 'toy)
                      (add-syntax-rule 'toy "\\bfn\\b" 'keyword)
                      (add-syntax-region 'toy "/\\*" "\\*/" 'comment))"#,
            &env,
            &ctx,
        )
        .expect("defining the grammar");

        let grammar = grammar_of(&ctx, "toy");
        assert_eq!(grammar.rules.len(), 1);
        assert_eq!(grammar.regions.len(), 1);
        assert_eq!(
            faces(&grammar, "fn /* c */", &vec![]),
            vec![
                (0, 2, "keyword".to_string()),
                (3, 10, "comment".to_string()),
            ]
        );
    }

    #[test]
    fn lisp_can_ask_for_a_capture_group_a_nestable_region_and_an_escape() {
        let (ctx, env) = editor_with("");
        eval_str(
            r#"(progn (make-mode 'toy)
                      (add-syntax-rule 'toy "\\b([a-z]+)\\s*\\(" 'function 1)
                      (add-syntax-region 'toy "/\\*" "\\*/" 'comment nil t)
                      (add-syntax-region 'toy "\"" "\"" 'string "\\\\."))"#,
            &env,
            &ctx,
        )
        .expect("defining the grammar");

        let grammar = grammar_of(&ctx, "toy");
        assert_eq!(grammar.rules[0].group, 1);
        assert!(grammar.regions[0].nestable);
        assert!(!grammar.regions[1].nestable);
        assert!(grammar.regions[1].escape.is_some());
    }

    /// A grammar is configuration: one bad pattern should cost that pattern,
    /// not the file it is in.
    #[test]
    fn a_bad_pattern_is_reported_rather_than_signalled() {
        let (ctx, env) = editor_with("");
        eval_str("(make-mode 'toy)", &env, &ctx).expect("make-mode");

        assert_eq!(
            eval_str(r#"(add-syntax-rule 'toy "(unclosed" 'keyword)"#, &env, &ctx)
                .expect("must not signal"),
            LispExp::nil()
        );
        assert!(grammar_of(&ctx, "toy").rules.is_empty());
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("Invalid Regex")),
            "and it should be written down"
        );
    }

    #[test]
    fn defining_a_grammar_for_an_unknown_mode_answers_nil() {
        let (ctx, env) = editor_with("");
        assert_eq!(
            eval_str(r#"(add-syntax-rule 'nope "x" 'keyword)"#, &env, &ctx).expect("no signal"),
            LispExp::nil()
        );
        assert_eq!(
            eval_str(r#"(add-syntax-region 'nope "a" "b" 'comment)"#, &env, &ctx)
                .expect("no signal"),
            LispExp::nil()
        );
    }

    // -----------------------------------------------------------------------
    // The cache, and the worker that fills it
    // -----------------------------------------------------------------------

    fn define_toy(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        eval_str(
            r#"(progn (make-mode 'toy)
                      (add-syntax-rule 'toy "\\bfn\\b" 'keyword)
                      (add-syntax-region 'toy "/\\*" "\\*/" 'comment))"#,
            env,
            ctx,
        )
        .expect("defining the grammar");
        ctx.mutate_buffer(ctx.get_buffer("*scratch*").expect("*scratch*"), |b| {
            b.current_mode = "toy".into();
        });
    }

    /// Colour the whole buffer by turning the worker's crank directly, rather
    /// than sleeping and hoping the background thread got there.
    fn colour_fully(ctx: &Ctx) {
        for _ in 0..50 {
            ctx.highlight_one_turn();
        }
    }

    fn line_faces(ctx: &Ctx, line: usize) -> Vec<(usize, usize, String)> {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .syntax
            .spans(line)
            .iter()
            .map(|span| (span.start, span.end, span.face.name().to_string()))
            .collect()
    }

    #[test]
    fn the_worker_colours_the_buffer() {
        let (ctx, env) = editor_with("fn one\nfn two\n");
        define_toy(&ctx, &env);
        colour_fully(&ctx);

        assert_eq!(line_faces(&ctx, 0), vec![(0, 2, "keyword".to_string())]);
        assert_eq!(line_faces(&ctx, 1), vec![(0, 2, "keyword".to_string())]);
    }

    /// The state is carried from line to line through the cache, which is what
    /// lets a comment opened on one line colour the next.
    #[test]
    fn a_region_spanning_lines_is_coloured_through_the_cache() {
        let (ctx, env) = editor_with("/* open\nmiddle\nclose */ fn\n");
        define_toy(&ctx, &env);
        colour_fully(&ctx);

        assert_eq!(line_faces(&ctx, 1), vec![(0, 6, "comment".to_string())]);
        assert_eq!(
            line_faces(&ctx, 2),
            vec![
                (0, 8, "comment".to_string()),
                (9, 11, "keyword".to_string()),
            ]
        );
    }

    #[test]
    fn an_edit_bumps_the_version_and_untrusts_what_follows() {
        let (ctx, env) = editor_with("fn one\nfn two\nfn three\n");
        define_toy(&ctx, &env);
        colour_fully(&ctx);

        let before = ctx
            .get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .version;
        eval_str("(progn (goto-char 8) (self-insert \"x\"))", &env, &ctx).expect("edit");

        let buffer = ctx.get_buffer("*scratch*").expect("*scratch*");
        let buf = buffer.read().unwrap();
        assert!(buf.version > before, "an edit must bump the version");
        assert!(
            buf.syntax.valid_to() <= 1,
            "nothing from the edited line on is trusted, got {}",
            buf.syntax.valid_to()
        );
    }

    /// Clearing instead would flash the rest of the file to plain text on
    /// every keystroke.
    #[test]
    fn stale_colour_is_kept_on_screen_while_it_is_recomputed() {
        let (ctx, env) = editor_with("fn one\nfn two\nfn three\n");
        define_toy(&ctx, &env);
        colour_fully(&ctx);

        eval_str("(progn (goto-char 0) (self-insert \"x\"))", &env, &ctx).expect("edit");

        assert_eq!(
            line_faces(&ctx, 2),
            vec![(0, 2, "keyword".to_string())],
            "the line below the edit should still be coloured while it waits"
        );
    }

    #[test]
    fn colour_comes_back_after_an_edit() {
        let (ctx, env) = editor_with("fn one\n/* c */\n");
        define_toy(&ctx, &env);
        colour_fully(&ctx);

        // A space, not a letter: inserting `x` before `fn` would destroy the
        // word boundary and correctly leave no keyword to find.
        eval_str(r#"(progn (goto-char 0) (self-insert " "))"#, &env, &ctx).expect("edit");
        colour_fully(&ctx);

        assert_eq!(
            line_faces(&ctx, 0),
            vec![(1, 3, "keyword".to_string())],
            "the edited line is re-coloured at its new columns"
        );
        assert_eq!(line_faces(&ctx, 1), vec![(0, 7, "comment".to_string())]);
    }

    /// Lines are only ever recorded in order, because each one's entering
    /// state is the previous one's exit. A line out of order has no state to
    /// stand on, so it is refused rather than written somewhere misleading.
    #[test]
    fn the_cache_refuses_a_line_out_of_order() {
        let mut cache = crate::buffer::syntax::SyntaxCache::default();

        assert!(cache.record(0, &vec![], Vec::new()).is_ok());
        assert!(
            cache.record(5, &vec![], Vec::new()).is_err(),
            "line 5 cannot be recorded while lines 1..5 are unknown"
        );
        assert_eq!(cache.valid_to(), 1);
        assert!(cache.record(1, &vec![], Vec::new()).is_ok());
    }

    /// The rule that stops stale colour being painted at columns that have
    /// moved. Only observable between computing a turn and storing it, which
    /// is why those are two calls.
    #[test]
    fn a_turn_computed_against_older_text_is_refused() {
        let (ctx, env) = editor_with("fn one\nfn two\n");
        define_toy(&ctx, &env);

        let turn = ctx.turn_for("*scratch*").expect("there is work to do");
        let coloured = turn.run();

        // The user typed while that was being worked out.
        eval_str(r#"(progn (goto-char 0) (self-insert " "))"#, &env, &ctx).expect("edit");
        ctx.store_turn(&turn, coloured);

        assert_eq!(
            ctx.get_buffer("*scratch*")
                .expect("*scratch*")
                .read()
                .unwrap()
                .syntax
                .valid_to(),
            0,
            "nothing computed from the old text may be stored against the new"
        );
    }

    /// A file longer than one turn is coloured over several, and the state has
    /// to be carried across the join -- otherwise a region opened in the first
    /// chunk would be forgotten at the start of the second.
    #[test]
    fn a_region_survives_the_boundary_between_two_turns() {
        let chunk = crate::buffer::syntax::LINES_PER_TURN;
        let mut text = String::from("/* opens\n");
        for line in 0..chunk + 40 {
            text.push_str(&format!("line {line}\n"));
        }
        text.push_str("closes */\n");

        let (ctx, env) = editor_with(&text);
        define_toy(&ctx, &env);

        ctx.highlight_one_turn();
        assert!(
            ctx.get_buffer("*scratch*")
                .expect("*scratch*")
                .read()
                .unwrap()
                .syntax
                .valid_to()
                < chunk + 42,
            "one turn should not have finished the whole file"
        );

        colour_fully(&ctx);
        let faced = line_faces(&ctx, chunk + 20);
        assert_eq!(
            faced.len(),
            1,
            "a line well past the first chunk should be one uniform run"
        );
        assert_eq!(
            (faced[0].0, faced[0].2.as_str()),
            (0, "comment"),
            "and it should still be inside the comment opened on line 0"
        );
    }

    #[test]
    fn a_buffer_whose_mode_has_no_grammar_is_left_alone() {
        let (ctx, _env) = editor_with("fn one\nfn two\n");
        colour_fully(&ctx);

        assert!(
            line_faces(&ctx, 0).is_empty(),
            "fundamental mode colours nothing"
        );
    }

    /// Interning is what keeps the cache proportional to the interesting parts
    /// of a file rather than to the file.
    #[test]
    fn the_cache_interns_the_states_it_sees() {
        let (ctx, env) = editor_with("fn a\nfn b\nfn c\n/* open\nstill\nclose */\n");
        define_toy(&ctx, &env);
        colour_fully(&ctx);

        assert_eq!(
            ctx.get_buffer("*scratch*")
                .expect("*scratch*")
                .read()
                .unwrap()
                .syntax
                .distinct_states(),
            2,
            "six lines, but only two states: inside the comment and outside it"
        );
    }

    // -----------------------------------------------------------------------
    // Reaching the screen
    // -----------------------------------------------------------------------

    #[test]
    fn coloured_lines_reach_the_frame() {
        let (ctx, env) = editor_with("fn one\n");
        define_toy(&ctx, &env);
        colour_fully(&ctx);

        let frame = ctx.snapshot(&env, W, H);
        let view = frame.focused_view().expect("a focused window");
        assert!(
            view.highlights.iter().any(|h| {
                h.row == 0 && h.start_col == 0 && h.end_col == 2 && h.face == Face::KEYWORD
            }),
            "the keyword should be in the frame, got {:?}",
            view.highlights
        );
    }

    /// The region is pushed after the syntax so the renderer, which draws later
    /// entries over earlier ones, keeps a selection visible on top of colour.
    #[test]
    fn the_region_is_drawn_over_the_colouring() {
        let (ctx, env) = editor_with("fn one\n");
        define_toy(&ctx, &env);
        colour_fully(&ctx);
        eval_str("(progn (goto-char 0) (set-mark 6))", &env, &ctx).expect("a region");

        let frame = ctx.snapshot(&env, W, H);
        let view = frame.focused_view().expect("a focused window");
        let syntax = view
            .highlights
            .iter()
            .position(|h| h.face == Face::KEYWORD)
            .expect("the keyword");
        let region = view
            .highlights
            .iter()
            .position(|h| h.face == Face::REGION)
            .expect("the region");

        assert!(syntax < region, "the region must be drawn last");
    }

    #[test]
    fn an_uncoloured_buffer_contributes_no_highlights() {
        let (ctx, env) = editor_with("fn one\n");
        define_toy(&ctx, &env);
        // Deliberately not coloured: this is what a large file looks like for
        // the first moments after it opens.
        let frame = ctx.snapshot(&env, W, H);
        let view = frame.focused_view().expect("a focused window");
        assert!(view.highlights.is_empty(), "plain text, not a wait");
    }

    // -----------------------------------------------------------------------
    // Modules
    // -----------------------------------------------------------------------

    #[test]
    fn add_auto_mode_decides_the_mode_a_file_opens_in() {
        let (ctx, env) = editor_with("");
        eval_str(
            r#"(progn (make-mode 'toy) (add-auto-mode "\\.toy$" 'toy))"#,
            &env,
            &ctx,
        )
        .expect("add-auto-mode");

        assert_eq!(ctx.auto_mode_for("/tmp/thing.toy"), Some("toy".to_string()));
        assert_eq!(ctx.auto_mode_for("/tmp/thing.txt"), None);
    }

    /// Overlapping patterns are common, so which wins has to be a decision
    /// somebody made rather than whichever happened to be looked at first.
    #[test]
    fn the_first_matching_auto_mode_wins() {
        let (ctx, env) = editor_with("");
        eval_str(
            r#"(progn (make-mode 'special) (make-mode 'general)
                      (add-auto-mode "build\\.rs$" 'special)
                      (add-auto-mode "\\.rs$" 'general))"#,
            &env,
            &ctx,
        )
        .expect("add-auto-mode");

        assert_eq!(ctx.auto_mode_for("build.rs"), Some("special".to_string()));
        assert_eq!(ctx.auto_mode_for("main.rs"), Some("general".to_string()));
    }

    #[test]
    fn a_file_opens_in_the_mode_its_name_claims() {
        let (ctx, env) = editor_with("");
        let path = std::env::temp_dir().join("rsedit-syntax-auto-mode.toy");
        std::fs::write(&path, "fn one\n").expect("writing the file");

        eval_str(
            &format!(
                r#"(progn (make-mode 'toy) (add-auto-mode "\\.toy$" 'toy) (find-file "{}"))"#,
                path.display()
            ),
            &env,
            &ctx,
        )
        .expect("find-file");

        assert_eq!(
            ctx.get_buffer("rsedit-syntax-auto-mode.toy")
                .expect("the opened buffer")
                .read()
                .unwrap()
                .current_mode,
            "toy"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// The module that ships, loaded the way a user would load it.
    #[test]
    fn the_rust_module_defines_a_working_grammar() {
        let (ctx, env) = editor_with("");
        eval_str(include_str!("../../lisp/rust-mode.lisp"), &env, &ctx).expect("loading rust-mode");

        assert_eq!(ctx.auto_mode_for("src/main.rs"), Some("rust-mode".into()));
        let grammar = grammar_of(&ctx, "rust-mode");

        assert_eq!(
            faces(&grammar, "pub fn main() {", &vec![]),
            vec![
                (0, 3, "keyword".to_string()),
                (4, 6, "keyword".to_string()),
                (7, 11, "function".to_string()),
            ]
        );
        assert_eq!(
            faces(&grammar, r#"let s = "a \" b"; // done"#, &vec![]),
            vec![
                (0, 3, "keyword".to_string()),
                (8, 16, "string".to_string()),
                (18, 25, "comment".to_string()),
            ],
            "the escaped quote must not end the string"
        );
    }

    /// The two faces the module invents exist only because it named them --
    /// which is the whole reason the face set was opened up.
    #[test]
    fn the_rust_module_defines_faces_the_editor_never_heard_of() {
        let (ctx, env) = editor_with("");
        eval_str(include_str!("../../lisp/rust-mode.lisp"), &env, &ctx).expect("loading rust-mode");

        let doc = Face::named("doc-comment").expect("the module should have defined it");
        assert_ne!(doc, Face::DEFAULT);
        assert_eq!(
            faces(&grammar_of(&ctx, "rust-mode"), "/// docs", &vec![]),
            vec![(0, 8, "doc-comment".to_string())]
        );
        assert!(
            ctx.face_style(doc).italic,
            "and given it an appearance of its own"
        );
    }
}
