//! Return and the minor mode that indents as you type: `newline`, which only
//! inserts, and `electric-indent-mode`, on `post-self-insert-hook`, which
//! tidies the line Return ends, indents the one it opens, and indents a line
//! when a character its mode names is typed first on it. Then `rust-mode`'s
//! rule, which the mode now has.
//!
//! Design: `doc/architettura-editor.typ, <mod-indent>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use risp::{Env, EvalError, LispExp, Parser, eval};
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

    /// The editor with indentation and the three language modes loaded, as
    /// init.lisp loads them.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/base/common-keymaps.lisp"),
            include_str!("../../lisp/base/indent.lisp"),
            include_str!("../../lisp/languages/rust-mode.lisp"),
            include_str!("../../lisp/languages/cc-mode.lisp"),
            include_str!("../../lisp/languages/python-mode.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A buffer called NAME in MODE, holding TEXT with point at AT, current.
    fn buffer(name: &str, mode: &str, text: &str, at: usize, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(r#"(buffer-create "{name}" '{mode}) (switch-to-buffer "{name}")"#),
            env,
            ctx,
        );
        ctx.with_current_buffer_mut(|b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(at);
            b.text.cursor_move(line, col);
        });
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|b| b.text.cursor_pos_1d())
    }

    fn press(code: KeyCode, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        ctx.handle_key_event(
            KeyEvent {
                code,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn enter(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        press(KeyCode::Enter, env, ctx);
    }

    fn type_str(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for c in text.chars() {
            press(KeyCode::Char(c), env, ctx);
        }
    }

    // ---------------- newline ----------------

    #[test]
    fn return_runs_newline_and_newline_only_inserts() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(car (nth 1 (key-binding "<ret>")))"#, &env, &ctx),
            LispExp::symbol("newline".into())
        );
        buffer("a.c", "c-mode", "int main() {", 12, &env, &ctx);
        // From Lisp no hook runs, whatever minor modes are on.
        run("(newline)", &env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n");
        run("(newline 2)", &env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n\n\n");
        assert!(eval_str("(newline -1)", &env, &ctx).is_err());
    }

    #[test]
    fn insert_newline_never_indents_even_from_a_key() {
        let (ctx, env) = editor();
        run(r#"(define-key nil "<ret>" 'insert-newline)"#, &env, &ctx);
        buffer("a.c", "c-mode", "int main() {", 12, &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n");
    }

    // ---------------- electric-indent-mode ----------------

    #[test]
    fn the_mode_is_on_in_every_buffer_old_and_new() {
        let (ctx, env) = editor();
        assert!(
            run(
                r#"(minor-mode-p 'electric-indent-mode "*scratch*")"#,
                &env,
                &ctx
            )
            .is_truthy()
        );
        buffer("a.rs", "rust-mode", "", 0, &env, &ctx);
        assert!(run("(minor-mode-p 'electric-indent-mode)", &env, &ctx).is_truthy());
        // Not a variable any more: turned off as a minor mode is.
        assert!(env.get_variable("electric-indent-mode").is_none());
    }

    #[test]
    fn return_indents_the_line_it_opens() {
        let (ctx, env) = editor();
        buffer("a.c", "c-mode", "int main() {", 12, &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n    ");
        assert_eq!(point(&ctx), 17, "at the end of the new indentation");
    }

    #[test]
    fn return_puts_the_line_it_ends_back_where_it_belongs() {
        let (ctx, env) = editor();
        // The closing brace was typed at the body's indentation, where an
        // electric character would have moved it, here inserted directly.
        let text = "int main() {\n    x;\n    }";
        buffer("a.c", "c-mode", text, text.len(), &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n    x;\n}\n");
    }

    #[test]
    fn return_leaves_no_blanks_at_the_end_of_the_line_it_ends() {
        let (ctx, env) = editor();
        let text = "int main() {\n    x;   ";
        buffer("a.c", "c-mode", text, text.len(), &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n    x;\n    ");
    }

    #[test]
    fn return_in_the_middle_of_a_line_indents_what_it_carries_down() {
        let (ctx, env) = editor();
        let text = "int main() {  return 0; }";
        buffer("a.c", "c-mode", text, 14, &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n    return 0; }");
    }

    #[test]
    fn return_from_a_blank_line_keeps_the_indentation_chosen_there() {
        let (ctx, env) = editor();
        // Twelve chosen by hand where the rule would say four.
        let text = "int main() {\n            ";
        buffer("a.c", "c-mode", text, text.len(), &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n\n            ");
    }

    #[test]
    fn python_leaves_the_line_it_ends_where_tab_put_it() {
        let (ctx, env) = editor();
        // Dedented by hand to end the block: indenting it again would undo
        // the choice, so python-mode inhibits that, as in Emacs.
        let text = "def a():\n    work()\nafter()";
        buffer("a.py", "python-mode", text, text.len(), &env, &ctx);
        enter(&env, &ctx);
        assert!(
            contents(&ctx).starts_with("def a():\n    work()\nafter()\n"),
            "{:?}",
            contents(&ctx)
        );
    }

    #[test]
    fn a_brace_typed_first_on_its_line_goes_to_its_place() {
        let (ctx, env) = editor();
        let text = "int main() {\n    x;\n    ";
        buffer("a.c", "c-mode", text, text.len(), &env, &ctx);
        type_str("}", &env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n    x;\n}");
        assert_eq!(point(&ctx), contents(&ctx).len(), "after the brace");
    }

    #[test]
    fn a_brace_after_other_text_leaves_the_line_alone() {
        let (ctx, env) = editor();
        let text = "int main() {\n        x = {";
        buffer("a.c", "c-mode", text, text.len(), &env, &ctx);
        type_str("}", &env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n        x = {}");
    }

    #[test]
    fn a_mode_that_names_no_characters_is_not_indented_as_you_type() {
        let (ctx, env) = editor();
        let text = "def a():\n            ";
        buffer("a.py", "python-mode", text, text.len(), &env, &ctx);
        type_str("}", &env, &ctx);
        assert_eq!(contents(&ctx), "def a():\n            }");
    }

    #[test]
    fn turned_off_return_is_a_plain_newline() {
        let (ctx, env) = editor();
        buffer("a.c", "c-mode", "int main() {", 12, &env, &ctx);
        run("(electric-indent-mode -1)", &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n");
        type_str("}", &env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n}");
    }

    #[test]
    fn a_read_only_buffer_refuses_return_and_nothing_else_happens() {
        let (ctx, env) = editor();
        let text = "int main() {\n    x;   ";
        buffer("a.c", "c-mode", text, text.len(), &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), text, "not even the blanks went");
    }

    // ---------------- rust-mode's rule ----------------

    /// Indents every line of TEXT with Tab, from the top, and returns it.
    fn rust_indented(text: &str) -> String {
        let (ctx, env) = editor();
        buffer("a.rs", "rust-mode", text, 0, &env, &ctx);
        let lines = text.lines().count();
        for line in 1..=lines {
            run(&format!("(goto-line {line}) (indent-line)"), &env, &ctx);
        }
        contents(&ctx)
    }

    #[test]
    fn rust_blocks_struct_fields_and_match_arms_are_one_width_in() {
        let flat = "fn main() {\nlet p = Point {\nx: 1,\ny: 2,\n};\nmatch v {\nSome(a) => a,\nNone => {\n0\n}\n}\n}";
        assert_eq!(
            rust_indented(flat),
            "fn main() {\n    let p = Point {\n        x: 1,\n        y: 2,\n    };\n    match v {\n        Some(a) => a,\n        None => {\n            0\n        }\n    }\n}"
        );
    }

    #[test]
    fn rust_a_method_chain_and_a_long_condition_continue_the_statement() {
        let flat = "fn main() {\nlet s = items\n.iter()\n.map(f)\n.collect();\nif a\n&& b {\ngo();\n}\nlet x =\n5;\n}";
        assert_eq!(
            rust_indented(flat),
            "fn main() {\n    let s = items\n        .iter()\n        .map(f)\n        .collect();\n    if a\n        && b {\n        go();\n    }\n    let x =\n        5;\n}"
        );
    }

    #[test]
    fn rust_arguments_line_up_inside_parentheses() {
        let flat = "fn main() {\nlet x = f(a,\nb);\n}";
        assert_eq!(
            rust_indented(flat),
            "fn main() {\n    let x = f(a,\n              b);\n}"
        );
    }

    #[test]
    fn rust_tab_no_longer_flattens_a_function_body() {
        let (ctx, env) = editor();
        let text = "fn main() {\n    let x = 1;\n}";
        buffer("a.rs", "rust-mode", text, 12, &env, &ctx);
        for _ in 0..3 {
            press(KeyCode::Tab, &env, &ctx);
            assert_eq!(contents(&ctx), text);
        }
    }

    #[test]
    fn rust_return_after_an_opening_brace_indents_the_body() {
        let (ctx, env) = editor();
        buffer("a.rs", "rust-mode", "fn main() {", 11, &env, &ctx);
        enter(&env, &ctx);
        type_str("let x = 1;", &env, &ctx);
        enter(&env, &ctx);
        type_str("}", &env, &ctx);
        assert_eq!(contents(&ctx), "fn main() {\n    let x = 1;\n}");
    }

    // ---------------- with electric-pair ----------------

    /// As [`editor`], with electric-pair loaded after indentation, as
    /// init.lisp loads it.
    fn editor_with_pairs() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor();
        eval_str(
            include_str!("../../lisp/features/electric-pair.lisp"),
            &env,
            &ctx,
        )
        .expect("loading electric-pair.lisp");
        (ctx, env)
    }

    #[test]
    fn return_inside_a_typed_pair_opens_an_indented_line_for_the_body() {
        let (ctx, env) = editor_with_pairs();
        buffer("a.rs", "rust-mode", "", 0, &env, &ctx);
        type_str("fn main() {", &env, &ctx);
        assert_eq!(contents(&ctx), "fn main() {}", "the partner typed itself");
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "fn main() {\n    \n}");
        type_str("let x = 1;", &env, &ctx);
        assert_eq!(contents(&ctx), "fn main() {\n    let x = 1;\n}");
    }

    /// The two minor modes' hooks run in the order the modes came on: the
    /// same lines come out whichever runs first.
    #[test]
    fn the_order_the_two_modes_came_on_in_does_not_matter() {
        let (ctx, env) = editor_with_pairs();
        buffer("a.c", "c-mode", "", 0, &env, &ctx);
        run("(electric-pair-mode 1)", &env, &ctx);
        // Off and on again: electric-indent now comes after electric-pair.
        run(
            "(electric-indent-mode -1) (electric-indent-mode 1)",
            &env,
            &ctx,
        );
        type_str("int main() {", &env, &ctx);
        enter(&env, &ctx);
        assert_eq!(contents(&ctx), "int main() {\n    \n}");
        assert_eq!(point(&ctx), 17);
    }

    #[test]
    fn nested_pairs_open_one_level_at_a_time() {
        let (ctx, env) = editor_with_pairs();
        buffer("a.rs", "rust-mode", "", 0, &env, &ctx);
        type_str("fn main() {", &env, &ctx);
        enter(&env, &ctx);
        type_str("if a {", &env, &ctx);
        enter(&env, &ctx);
        type_str("go();", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "fn main() {\n    if a {\n        go();\n    }\n}"
        );
    }
}
