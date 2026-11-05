//! The blanks at the ends of lines, drawn: in a buffer visiting a file, where
//! the variable or the mode says, and not on point's line while point is
//! among them.
//!
//! Design: `doc/architettura-editor.typ, <evidenziazioni>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
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

    /// An editor showing TEXT in *scratch*, with point at POINT, as a buffer
    /// visiting a file, and the drawing on.
    fn editor_with(text: &str, point: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        env.set_variable("show-trailing-whitespace".into(), LispExp::t());
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(point);
            b.text.cursor_move(line, col);
            b.file_path = Some("/tmp/rsedit-trailing-test.txt".into());
        });
        (ctx, env)
    }

    /// Returns (row, start, end) of the trailing blanks drawn in each view
    /// showing *scratch*, the focused one first.
    fn drawn(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<Vec<(usize, usize, usize)>> {
        let mut views: Vec<_> = ctx
            .snapshot(env, W, H)
            .views
            .into_iter()
            .filter(|view| view.buffer_name == "*scratch*")
            .collect();
        views.sort_by_key(|view| !view.is_focused);
        views
            .into_iter()
            .map(|view| {
                let mut cells: Vec<_> = view
                    .highlights
                    .iter()
                    .filter(|h: &&Highlight| h.face == Face::TRAILING_WHITESPACE)
                    .map(|h| (h.row, h.start_col, h.end_col))
                    .collect();
                cells.sort();
                cells
            })
            .collect()
    }

    #[test]
    fn the_blanks_ending_a_line_are_drawn() {
        let (ctx, env) = editor_with("abc  \nxyz\n \t\nend", 0);
        let views = drawn(&ctx, &env);
        // The Tab reaches column 4, the next multiple of the default width.
        assert_eq!(views[0], vec![(0, 3, 5), (2, 0, 4)]);
    }

    #[test]
    fn nothing_is_drawn_unless_asked_for() {
        let (ctx, env) = editor_with("abc  \n", 0);
        run("(setq show-trailing-whitespace nil)", &env, &ctx);
        assert!(drawn(&ctx, &env)[0].is_empty());
    }

    #[test]
    fn a_buffer_visiting_no_file_draws_nothing() {
        let (ctx, env) = editor_with("abc  \n", 0);
        ctx.with_buffer_mut("*scratch*", |b| b.file_path = None);
        assert!(drawn(&ctx, &env)[0].is_empty());
    }

    #[test]
    fn a_modes_own_property_wins_over_the_variable() {
        let (ctx, env) = editor_with("abc  \n", 0);
        run(
            "(put 'fundamental-mode 'show-trailing-whitespace nil)",
            &env,
            &ctx,
        );
        assert!(drawn(&ctx, &env)[0].is_empty());
    }

    #[test]
    fn the_blanks_point_is_among_are_not_drawn_where_it_is_typing() {
        let (ctx, env) = editor_with("abc  \nxyz \n", 5);
        assert_eq!(drawn(&ctx, &env)[0], vec![(1, 3, 4)], "only the other line");
        run("(split-window-below)", &env, &ctx);
        let views = drawn(&ctx, &env);
        assert_eq!(
            views[1],
            vec![(0, 3, 5), (1, 3, 4)],
            "a window without the keyboard draws them"
        );
    }

    #[test]
    fn point_before_the_blanks_leaves_them_drawn() {
        let (ctx, env) = editor_with("abc  \n", 1);
        assert_eq!(drawn(&ctx, &env)[0], vec![(0, 3, 5)]);
    }

    #[test]
    fn toggle_trailing_whitespace_turns_it_off_and_on() {
        let (ctx, env) = editor_with("abc  \n", 0);
        for source in [
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/base/common-keymaps.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        run("(toggle-trailing-whitespace)", &env, &ctx);
        assert!(drawn(&ctx, &env)[0].is_empty());
        run("(toggle-trailing-whitespace)", &env, &ctx);
        assert_eq!(drawn(&ctx, &env)[0], vec![(0, 3, 5)]);
    }
}
