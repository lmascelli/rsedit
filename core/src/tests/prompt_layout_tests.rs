//! The question in front of the minibuffer's answer: whole when it fits, and
//! shortened in the middle when it does not, so that the answer keeps room
//! to be typed in and the question its `(yes or no)`.
//!
//! Design: `doc/architettura-editor.typ, <margine>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::ui::RenderableWindowView;
    use crate::ui::width::cells;
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const H: usize = 24;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env must build")
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|err| panic!("{src}: {err:?}"))
    }

    /// Opens a plain prompt asking PROMPT.
    fn ask(prompt: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!("(minibuffer-read {prompt:?} nil nil nil)"),
            env,
            ctx,
        );
    }

    /// The minibuffer's window, in a frame WIDTH columns wide.
    fn minibuffer(ctx: &Ctx, env: &Arc<Env<Ctx>>, width: usize) -> RenderableWindowView {
        ctx.snapshot(env, width, H)
            .views
            .into_iter()
            .find(|view| view.buffer_name == "*Minibuffer*")
            .expect("a minibuffer must be open")
    }

    /// A question 100 columns wide, a long path in the middle of it.
    fn long_question() -> String {
        let path = format!("/home/user/{}/file.txt", "d".repeat(60));
        let question = format!("Delete {path}? (yes or no)");
        assert_eq!(cells(&question), 100);
        question
    }

    #[test]
    fn a_question_that_fits_is_shown_whole() {
        let (ctx, env) = editor();
        ask("Find file: ", &env, &ctx);
        let view = minibuffer(&ctx, &env, 80);
        assert_eq!(view.gutter_width, 11);
        assert_eq!(view.gutter[0].text, "Find file: ");
    }

    #[test]
    fn a_question_of_100_columns_leaves_20_to_the_answer_in_80() {
        let (ctx, env) = editor();
        ask(&long_question(), &env, &ctx);
        let view = minibuffer(&ctx, &env, 80);
        assert_eq!(view.gutter_width, 60);
        assert_eq!(view.rect.width, 20, "the answer's columns");
        assert_eq!(cells(&view.gutter[0].text), 60);
    }

    #[test]
    fn a_shortened_question_keeps_its_beginning_and_its_yes_or_no() {
        let (ctx, env) = editor();
        let question = long_question();
        ask(&question, &env, &ctx);
        let shown = minibuffer(&ctx, &env, 80).gutter[0].text.clone();
        // 59 of the 60 columns, and the blank before the answer.
        let (head, tail) = shown
            .split_once('…')
            .expect("an ellipsis where the middle was");
        assert_eq!(head, &question[..29]);
        assert_eq!(tail, format!("{} ", &question[100 - 29..]));
        assert!(tail.ends_with("/file.txt? (yes or no) "), "{shown:?}");
    }

    #[test]
    fn in_a_narrow_window_the_answer_keeps_half_of_it() {
        let (ctx, env) = editor();
        ask(&long_question(), &env, &ctx);
        let view = minibuffer(&ctx, &env, 30);
        assert_eq!(view.gutter_width, 15);
        assert_eq!(view.rect.width, 15);
        assert!(view.gutter[0].text.contains('…'));
        assert_eq!(cells(&view.gutter[0].text), 15);
    }

    #[test]
    fn a_question_ending_in_a_blank_is_shortened_to_the_whole_of_its_room() {
        let (ctx, env) = editor();
        let question = format!("{} ", long_question());
        ask(&question, &env, &ctx);
        let shown = minibuffer(&ctx, &env, 80).gutter[0].text.clone();
        assert_eq!(cells(&shown), 60);
        assert!(shown.ends_with("(yes or no) "), "{shown:?}");
        assert!(shown.starts_with("Delete /home/user/"), "{shown:?}");
    }
}
