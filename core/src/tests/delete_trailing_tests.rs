//! Deleting the blanks at the ends of lines: in the whole buffer with the
//! blank lines at its end, in the region, between two positions, with point
//! kept; and before a save, where Lisp asks.
//!
//! Design: `doc/architettura-editor.typ, <verbi>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn editor_with(text: &str, point: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(point);
            b.text.cursor_move(line, col);
            b.is_modified = false;
        });
        (ctx, env)
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|b| b.text.cursor_pos_1d())
    }

    /// A directory of the test's own, gone when it is dropped.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "rsedit-delete-trailing-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, contents: &str) -> String {
            let path = self.0.join("file.txt");
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // ----------------------------------------------------------------
    // The command
    // ----------------------------------------------------------------

    #[test]
    fn the_whole_buffer_loses_its_blanks_and_its_blank_lines_at_the_end() {
        let (ctx, env) = editor_with("a  \nb\t\nc\n \n\n", 0);
        assert_eq!(
            run("(delete-trailing-whitespace)", &env, &ctx),
            LispExp::number(3.0),
            "two lines, and the blank lines at the end"
        );
        assert_eq!(text(&ctx), "a\nb\nc\n");
        assert_eq!(
            ctx.get_echo_message(),
            "Trailing whitespace deleted on 3 lines"
        );
    }

    #[test]
    fn with_delete_trailing_lines_off_the_blank_lines_at_the_end_stay() {
        let (ctx, env) = editor_with("a \n\n\n", 0);
        run("(setq delete-trailing-lines nil)", &env, &ctx);
        run("(delete-trailing-whitespace)", &env, &ctx);
        assert_eq!(text(&ctx), "a\n\n\n");
    }

    #[test]
    fn a_last_line_with_no_newline_is_cleaned_too() {
        let (ctx, env) = editor_with("a\nb  ", 0);
        run("(delete-trailing-whitespace)", &env, &ctx);
        assert_eq!(text(&ctx), "a\nb");
    }

    #[test]
    fn with_the_region_active_only_its_lines_are_cleaned() {
        let (ctx, env) = editor_with("a  \nb  \n\n\n", 0);
        ctx.with_current_buffer_mut(|buf| {
            let mut mark = crate::buffer::Mark::new(2);
            mark.active = true;
            buf.mark = Some(mark);
        });
        run("(delete-trailing-whitespace)", &env, &ctx);
        assert_eq!(text(&ctx), "a\nb  \n\n\n");
    }

    #[test]
    fn two_positions_name_the_lines_to_clean() {
        let (ctx, env) = editor_with("a  \nb  \nc  \n", 0);
        run("(delete-trailing-whitespace 4 8)", &env, &ctx);
        assert_eq!(text(&ctx), "a  \nb\nc  \n");
    }

    #[test]
    fn point_among_what_goes_ends_up_at_the_end_of_its_line() {
        let (ctx, env) = editor_with("ab   \ncd  \nef", 3);
        run("(delete-trailing-whitespace)", &env, &ctx);
        assert_eq!(point(&ctx), 2);
        let (ctx, env) = editor_with("ab   \ncd  \nef", 12);
        run("(delete-trailing-whitespace)", &env, &ctx);
        assert_eq!(text(&ctx), "ab\ncd\nef");
        assert_eq!(point(&ctx), 7, "still between the e and the f");
    }

    #[test]
    fn a_clean_buffer_is_left_unmodified_and_says_so() {
        let (ctx, env) = editor_with("a\nb\n", 0);
        assert_eq!(
            run("(delete-trailing-whitespace)", &env, &ctx),
            LispExp::number(0.0)
        );
        assert!(!ctx.with_current_buffer(|b| b.is_modified));
        assert_eq!(ctx.get_echo_message(), "No trailing whitespace");
    }

    #[test]
    fn one_undo_puts_every_line_back() {
        let (ctx, env) = editor_with("a  \nb  \n", 0);
        run(
            "(undo-boundary) (delete-trailing-whitespace) (undo-boundary)",
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "a\nb\n");
        run("(undo)", &env, &ctx);
        assert_eq!(text(&ctx), "a  \nb  \n");
    }

    // ----------------------------------------------------------------
    // Before a save
    // ----------------------------------------------------------------

    #[test]
    fn a_save_leaves_the_blanks_unless_asked() {
        let sandbox = Sandbox::new("off");
        let path = sandbox.file("a  \n");
        let (ctx, env) = editor_with("", 0);
        run(&format!("(find-file {path:?})"), &env, &ctx);
        run("(insert \"x\") (save-buffer)", &env, &ctx);
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "xa  \n");
    }

    #[test]
    fn a_save_cleans_the_buffer_first_when_asked() {
        let sandbox = Sandbox::new("on");
        let path = sandbox.file("a  \nb\t\n\n\n");
        let (ctx, env) = editor_with("", 0);
        run("(setq delete-trailing-whitespace-on-save t)", &env, &ctx);
        run(&format!("(find-file {path:?})"), &env, &ctx);
        run("(insert \"x\") (save-buffer)", &env, &ctx);
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "xa\nb\n");
        assert_eq!(text(&ctx), "xa\nb\n");
        assert!(
            !ctx.with_current_buffer(|b| b.is_modified),
            "what was cleaned is what was written"
        );
    }

    #[test]
    fn a_modes_own_property_wins_over_the_variable() {
        let sandbox = Sandbox::new("mode");
        let path = sandbox.file("a  \n");
        let (ctx, env) = editor_with("", 0);
        run(
            "(setq delete-trailing-whitespace-on-save t)
             (put 'fundamental-mode 'delete-trailing-whitespace-on-save nil)",
            &env,
            &ctx,
        );
        run(&format!("(find-file {path:?})"), &env, &ctx);
        run("(insert \"x\") (save-buffer)", &env, &ctx);
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "xa  \n");
    }

    #[test]
    fn write_file_cleans_as_a_save_does() {
        let sandbox = Sandbox::new("write");
        let copy = sandbox.0.join("copy.txt").display().to_string();
        let (ctx, env) = editor_with("a  \n", 0);
        run("(setq delete-trailing-whitespace-on-save t)", &env, &ctx);
        run(&format!("(write-file {copy:?})"), &env, &ctx);
        assert_eq!(std::fs::read_to_string(&copy).expect("read"), "a\n");
    }
}
