//! What a frame draws for the moment: the delimiter at point and the one it
//! pairs with, read as the mode reads code, in the focused window only; and
//! every match of a search or a replace, on the lines on screen.
//!
//! Design: `doc/architettura-editor.typ, <evidenziazioni>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::feature::minibuffer::PROMPT_BUFFER;
    use crate::modes::sexp::{Pair, pairing};
    use crate::modes::{CommentStyle, SyntaxTable};
    use crate::ui::{Face, Highlight};
    use risp::{Env, EvalError, LispExp, Parser, eval};
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

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor showing TEXT in *scratch*, with point at POINT, and
    /// `show-paren-mode` on there, as its module turns it on.
    fn editor_with(text: &str, point: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/show-paren.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(point);
            b.text.cursor_move(line, col);
        });
        (ctx, env)
    }

    /// Returns the highlights in FACE of every view showing *scratch*, the
    /// focused one's first.
    fn drawn(ctx: &Ctx, env: &Arc<Env<Ctx>>, face: Face) -> Vec<Vec<Highlight>> {
        let frame = ctx.snapshot(env, W, H);
        let mut views: Vec<_> = frame
            .views
            .into_iter()
            .filter(|view| view.buffer_name == "*scratch*")
            .collect();
        views.sort_by_key(|view| !view.is_focused);
        views
            .into_iter()
            .map(|view| {
                view.highlights
                    .into_iter()
                    .filter(|highlight| highlight.face == face)
                    .collect()
            })
            .collect()
    }

    /// Returns (row, start, end) of each highlight.
    fn cells(highlights: &[Highlight]) -> Vec<(usize, usize, usize)> {
        let mut cells: Vec<_> = highlights
            .iter()
            .map(|h| (h.row, h.start_col, h.end_col))
            .collect();
        cells.sort();
        cells
    }

    fn pair_in(text: &str, point: usize) -> Option<Pair> {
        let text = GapBuffer::from(text);
        pairing(&text, &SyntaxTable::default(), None, point, 1000)
    }

    // ----------------------------------------------------------------
    // The scanner: which delimiter, and its partner
    // ----------------------------------------------------------------

    #[test]
    fn an_opener_after_point_pairs_with_its_closer() {
        assert_eq!(
            pair_in("(a (b) c)", 0),
            Some(Pair {
                at: 0,
                partner: Some(8),
                matched: true
            })
        );
    }

    #[test]
    fn a_closer_before_point_pairs_with_its_opener() {
        assert_eq!(
            pair_in("(a (b) c)", 6),
            Some(Pair {
                at: 5,
                partner: Some(3),
                matched: true
            })
        );
    }

    #[test]
    fn between_a_closer_and_an_opener_the_opener_is_the_one_paired() {
        assert_eq!(pair_in("()(x)", 2).map(|pair| pair.at), Some(2));
    }

    #[test]
    fn point_away_from_a_delimiter_pairs_nothing() {
        assert_eq!(pair_in("(a b)", 2), None);
    }

    #[test]
    fn delimiters_of_two_kinds_pair_but_do_not_match() {
        assert_eq!(
            pair_in("(a]", 0),
            Some(Pair {
                at: 0,
                partner: Some(2),
                matched: false
            })
        );
        assert_eq!(
            pair_in("(a]", 3),
            Some(Pair {
                at: 2,
                partner: Some(0),
                matched: false
            })
        );
    }

    #[test]
    fn an_opener_never_closed_and_a_stray_closer_have_no_partner() {
        let alone = |at| Pair {
            at,
            partner: None,
            matched: false,
        };
        assert_eq!(pair_in("(a b", 0), Some(alone(0)));
        assert_eq!(pair_in("a b)", 4), Some(alone(3)));
    }

    #[test]
    fn a_delimiter_in_a_string_is_text() {
        assert_eq!(pair_in(r#"x "(" y"#, 3), None);
        assert_eq!(pair_in(r#"x "(" y"#, 4), None);
        assert_eq!(
            pair_in(r#"(")")"#, 0).and_then(|pair| pair.partner),
            Some(4),
            "the closer in the string is passed over"
        );
    }

    #[test]
    fn a_delimiter_in_a_comment_is_text() {
        let mut table = SyntaxTable::default();
        table.set_comments(vec![CommentStyle::Line { opener: ";".into() }]);
        let text = GapBuffer::from("; (\n(a)");
        assert_eq!(pairing(&text, &table, None, 2, 1000), None);
        assert_eq!(
            pairing(&text, &table, None, 4, 1000).and_then(|pair| pair.partner),
            Some(6)
        );
    }

    #[test]
    fn a_partner_further_than_the_reach_is_not_looked_for() {
        let text = GapBuffer::from(&*format!("({})", "x".repeat(50)));
        let table = SyntaxTable::default();
        assert_eq!(pairing(&text, &table, None, 0, 10), None);
        assert_eq!(pairing(&text, &table, None, 52, 10), None, "nor behind");
        assert!(pairing(&text, &table, None, 0, 100).is_some());
    }

    // ----------------------------------------------------------------
    // The frame: show-paren-mode
    // ----------------------------------------------------------------

    #[test]
    fn the_delimiter_at_point_and_its_partner_are_drawn() {
        let (ctx, env) = editor_with("(foo bar)", 0);
        let views = drawn(&ctx, &env, Face::SHOW_PAREN_MATCH);
        assert_eq!(cells(&views[0]), vec![(0, 0, 1), (0, 8, 9)]);
    }

    #[test]
    fn a_mismatch_is_drawn_in_its_own_face() {
        let (ctx, env) = editor_with("(foo]", 0);
        assert!(drawn(&ctx, &env, Face::SHOW_PAREN_MATCH)[0].is_empty());
        let views = drawn(&ctx, &env, Face::SHOW_PAREN_MISMATCH);
        assert_eq!(cells(&views[0]), vec![(0, 0, 1), (0, 4, 5)]);
    }

    #[test]
    fn a_partner_on_another_line_is_drawn_where_it_is() {
        let (ctx, env) = editor_with("(foo\n  bar)", 11);
        let views = drawn(&ctx, &env, Face::SHOW_PAREN_MATCH);
        assert_eq!(cells(&views[0]), vec![(0, 0, 1), (1, 5, 6)]);
    }

    #[test]
    fn only_the_focused_window_draws_the_pair() {
        let (ctx, env) = editor_with("(foo bar)", 0);
        run("(split-window-below)", &env, &ctx);
        let views = drawn(&ctx, &env, Face::SHOW_PAREN_MATCH);
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].len(), 2);
        assert!(views[1].is_empty(), "{:?}", views[1]);
    }

    #[test]
    fn a_buffer_with_show_paren_mode_off_pairs_nothing() {
        let (ctx, env) = editor_with("(foo bar)", 0);
        run("(show-paren-mode -1)", &env, &ctx);
        assert!(drawn(&ctx, &env, Face::SHOW_PAREN_MATCH)[0].is_empty());
        run("(show-paren-mode 1)", &env, &ctx);
        assert_eq!(drawn(&ctx, &env, Face::SHOW_PAREN_MATCH)[0].len(), 2);
    }

    #[test]
    fn show_paren_mode_is_on_in_every_buffer_made_after_its_module_is_loaded() {
        let (ctx, env) = editor_with("", 0);
        run("(show-paren-mode -1)", &env, &ctx);
        run(
            r#"(make-mode 'prose-mode) (buffer-create "a" 'prose-mode) (buffer-create "b")"#,
            &env,
            &ctx,
        );
        for buffer in ["a", "b"] {
            assert!(
                run(
                    &format!("(minor-mode-p 'show-paren-mode {buffer:?})"),
                    &env,
                    &ctx
                )
                .is_truthy(),
                "{buffer}"
            );
        }
        assert!(
            run("(minor-mode-p 'show-paren-mode)", &env, &ctx).is_nil(),
            "the one that turned it off keeps it off"
        );
    }

    #[test]
    fn an_editor_without_the_module_pairs_nothing() {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        ctx.with_buffer_mut("*scratch*", |b| b.text = GapBuffer::from("(foo bar)"));
        assert!(drawn(&ctx, &env, Face::SHOW_PAREN_MATCH)[0].is_empty());
    }

    // ----------------------------------------------------------------
    // The frame: every match
    // ----------------------------------------------------------------

    /// Starts a search and types PATTERN into it.
    fn search(pattern: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run("(isearch-forward)", env, ctx);
        ctx.with_buffer_mut(PROMPT_BUFFER, |buf| {
            for c in pattern.chars() {
                buf.text.insert(c);
            }
        });
        run("(isearch-update)", env, ctx);
    }

    #[test]
    fn a_search_draws_every_match_on_screen() {
        let (ctx, env) = editor_with("foo bar foo\nbaz foo", 0);
        search("foo", &env, &ctx);
        let views = drawn(&ctx, &env, Face::LAZY_HIGHLIGHT);
        assert_eq!(cells(&views[0]), vec![(0, 0, 3), (0, 8, 11), (1, 4, 7)]);
        // The one the search is at is the region, drawn over its match.
        let region = drawn(&ctx, &env, Face::REGION);
        assert_eq!(cells(&region[0]), vec![(0, 0, 3)]);
    }

    #[test]
    fn the_matches_follow_what_is_typed_and_go_when_the_search_ends() {
        let (ctx, env) = editor_with("foo bar foo", 0);
        search("fo", &env, &ctx);
        assert_eq!(drawn(&ctx, &env, Face::LAZY_HIGHLIGHT)[0].len(), 2);
        run("(minibuffer-confirm)", &env, &ctx);
        assert!(drawn(&ctx, &env, Face::LAZY_HIGHLIGHT)[0].is_empty());
    }

    #[test]
    fn a_regexp_not_yet_valid_draws_nothing() {
        let (ctx, env) = editor_with("aaa", 0);
        run("(isearch-forward-regexp)", &env, &ctx);
        ctx.with_buffer_mut(PROMPT_BUFFER, |buf| buf.text.insert('['));
        run("(isearch-update)", &env, &ctx);
        assert!(drawn(&ctx, &env, Face::LAZY_HIGHLIGHT)[0].is_empty());
    }

    #[test]
    fn only_the_lines_on_screen_are_searched() {
        let text: String = (0..200).map(|_| "foo\n").collect();
        let (ctx, env) = editor_with(&text, 0);
        search("foo", &env, &ctx);
        let views = drawn(&ctx, &env, Face::LAZY_HIGHLIGHT);
        let rows: Vec<usize> = views[0].iter().map(|h| h.row).collect();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| *row < H), "{rows:?}");
        assert!(rows.len() < H, "a match per row on screen, no more");
    }

    #[test]
    fn with_highlight_all_matches_off_only_the_current_match_shows() {
        let (ctx, env) = editor_with("foo bar foo", 0);
        run("(setq highlight-all-matches nil)", &env, &ctx);
        search("foo", &env, &ctx);
        assert!(drawn(&ctx, &env, Face::LAZY_HIGHLIGHT)[0].is_empty());
    }

    #[test]
    fn a_replace_draws_the_matches_left_in_its_region() {
        let (ctx, env) = editor_with("foo foo foo foo", 4);
        // The region from the second foo to the end of the third.
        ctx.with_buffer_mut("*scratch*", |buf| {
            let mut mark = crate::buffer::Mark::new(11);
            mark.active = true;
            buf.mark = Some(mark);
        });
        assert_eq!(ctx.begin_replace("foo", "x", false, true, false), None);
        let views = drawn(&ctx, &env, Face::LAZY_HIGHLIGHT);
        assert_eq!(cells(&views[0]), vec![(0, 4, 7), (0, 8, 11)]);
        // Skipping the first leaves only the second to draw.
        assert!(ctx.replace_skip());
        let views = drawn(&ctx, &env, Face::LAZY_HIGHLIGHT);
        assert_eq!(cells(&views[0]), vec![(0, 8, 11)]);
    }
}
