//! Themes: what one is, applying it, and choosing it.
//!
//! The claim worth testing is that a theme can say "bold" where it cannot say
//! "blue" -- a monochrome theme is the whole reason the entry is a style
//! rather than a colour -- and that applying one really replaces the last
//! rather than layering on top of it.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
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

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/theme.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// `(face-style FACE)` as (fg, bg, attributes).
    fn style(
        face: &str,
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
    ) -> (Option<String>, Option<String>, Vec<String>) {
        let answer = run(&format!("(face-style '{face})"), env, ctx);
        let parts: Vec<LispExp<Ctx>> = answer.iter().collect();
        let text = |exp: &LispExp<Ctx>| match exp {
            LispExp::String(s) => Some(s.to_string()),
            other if other.is_nil() => None,
            other => panic!("expected a string or nil, got {other:?}"),
        };
        (
            text(&parts[0]),
            text(&parts[1]),
            parts[2..].iter().filter_map(text).collect(),
        )
    }

    /// Define a theme in the running editor, as a theme file would.
    fn define(name: &str, faces: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(&format!("(put '{name}-theme 'faces '({faces}))"), env, ctx);
    }

    // ----------------------------------------------------------------
    // Applying
    // ----------------------------------------------------------------

    #[test]
    fn a_theme_sets_the_faces_it_names() {
        let (ctx, env) = editor();
        define("toy", r#"(keyword "red" nil nil)"#, &env, &ctx);
        assert!(!run("(select-theme 'toy)", &env, &ctx).is_nil());
        let (fg, _, _) = style("keyword", &env, &ctx);
        assert!(fg.is_some(), "the keyword face should have a colour now");
    }

    #[test]
    fn a_theme_can_say_bold_where_it_cannot_say_blue() {
        // The whole reason an entry is a style rather than a colour: without
        // this a monochrome theme cannot exist.
        let (ctx, env) = editor();
        define("mono", r#"(keyword nil nil ("bold"))"#, &env, &ctx);
        run("(select-theme 'mono)", &env, &ctx);
        let (fg, bg, attributes) = style("keyword", &env, &ctx);
        assert_eq!(fg, None, "no foreground");
        assert_eq!(bg, None, "no background");
        assert!(
            attributes.contains(&"bold".to_string()),
            "got {attributes:?}"
        );
    }

    #[test]
    fn applying_a_theme_undoes_the_one_before_it() {
        // A sparse theme after a colourful one must not keep the colourful
        // one's leftovers -- the failure that makes themes feel broken.
        let (ctx, env) = editor();
        // Compared against how the face *ships*, not against nothing: several
        // faces have a colour out of the box, and "reset" means back to that.
        let shipped = style("string", &env, &ctx);
        define(
            "colourful",
            r#"(keyword "red" nil nil) (string "blue" nil nil)"#,
            &env,
            &ctx,
        );
        define("sparse", r#"(keyword nil nil ("bold"))"#, &env, &ctx);

        run("(select-theme 'colourful)", &env, &ctx);
        assert_ne!(
            style("string", &env, &ctx),
            shipped,
            "the theme took effect"
        );

        run("(select-theme 'sparse)", &env, &ctx);
        assert_eq!(
            style("string", &env, &ctx),
            shipped,
            "a face the new theme does not name goes back to shipped, \
             not to whatever the last theme left"
        );
    }

    #[test]
    fn a_theme_name_may_be_a_symbol_or_a_string() {
        // So a configuration file and the selector are the same call.
        let (ctx, env) = editor();
        define("toy", r#"(keyword "red" nil nil)"#, &env, &ctx);
        assert!(!run("(select-theme 'toy)", &env, &ctx).is_nil());
        assert!(!run(r#"(select-theme "toy")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn an_unknown_theme_is_reported_and_changes_nothing() {
        let (ctx, env) = editor();
        define("toy", r#"(keyword "red" nil nil)"#, &env, &ctx);
        run("(select-theme 'toy)", &env, &ctx);
        let before = style("keyword", &env, &ctx);

        assert!(run("(select-theme 'nonexistent)", &env, &ctx).is_nil());
        assert_eq!(style("keyword", &env, &ctx), before, "nothing was reset");
    }

    #[test]
    fn resetting_goes_back_to_the_shipped_appearance() {
        let (ctx, env) = editor();
        let shipped = style("keyword", &env, &ctx);
        define("toy", r#"(keyword "red" nil nil)"#, &env, &ctx);
        run("(select-theme 'toy)", &env, &ctx);
        assert_ne!(style("keyword", &env, &ctx), shipped);

        run("(reset-theme)", &env, &ctx);
        assert_eq!(style("keyword", &env, &ctx), shipped);
        assert!(run("current-theme", &env, &ctx).is_nil());
    }

    #[test]
    fn the_theme_in_force_is_remembered() {
        let (ctx, env) = editor();
        define("toy", r#"(keyword "red" nil nil)"#, &env, &ctx);
        run("(select-theme 'toy)", &env, &ctx);
        assert_eq!(
            run("current-theme", &env, &ctx),
            LispExp::string("toy".to_string())
        );
    }

    // ----------------------------------------------------------------
    // The shipped theme
    // ----------------------------------------------------------------

    #[test]
    fn the_monochrome_theme_loads_and_sets_no_colours_at_all() {
        // Read from the installed file, the way the selector reads it.
        let (ctx, env) = editor();
        let faces = run(
            &format!(
                "(progn (eval-string \"{}\") (get 'monochrome-theme 'faces))",
                include_str!("../../../themes/monochrome.lisp")
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
            ),
            &env,
            &ctx,
        );
        assert!(!faces.is_nil(), "the theme should define its faces");

        run("(select-theme 'monochrome)", &env, &ctx);
        for face in ["keyword", "comment", "string", "type"] {
            let (fg, bg, attributes) = style(face, &env, &ctx);
            assert_eq!(fg, None, "{face} should have no foreground");
            assert_eq!(bg, None, "{face} should have no background");
            assert!(
                !attributes.is_empty(),
                "{face} should be distinguished somehow"
            );
        }
    }

    // ----------------------------------------------------------------
    // The selector
    // ----------------------------------------------------------------

    fn listing(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run(
            r#"(with-current-buffer "*Themes*" (lambda () (buffer-string)))"#,
            env,
            ctx,
        ) {
            LispExp::String(s) => s.to_string(),
            other => panic!("expected the listing, got {other:?}"),
        }
    }

    #[test]
    fn the_listing_shows_what_is_installed() {
        let (ctx, env) = editor();
        run("(theme-list)", &env, &ctx);
        assert!(
            listing(&env, &ctx).contains("monochrome"),
            "got {:?}",
            listing(&env, &ctx)
        );
    }

    #[test]
    fn the_theme_in_force_is_marked() {
        let (ctx, env) = editor();
        run("(select-theme 'monochrome) (theme-list)", &env, &ctx);
        assert!(
            listing(&env, &ctx).contains("* monochrome"),
            "got {:?}",
            listing(&env, &ctx)
        );
    }

    #[test]
    fn return_applies_the_theme_on_the_line() {
        let (ctx, env) = editor();
        run("(theme-list)", &env, &ctx);
        let shown = listing(&env, &ctx);
        let line = shown
            .lines()
            .position(|l| l.contains("monochrome"))
            .expect("monochrome should be listed")
            + 1;
        run(
            &format!("(goto-line {line}) (theme-list-select)"),
            &env,
            &ctx,
        );
        assert_eq!(
            run("current-theme", &env, &ctx),
            LispExp::string("monochrome".to_string())
        );
    }

    #[test]
    fn choosing_moves_the_mark_to_the_chosen_line() {
        // The listing is the only thing that knows it has changed.
        let (ctx, env) = editor();
        run("(theme-list)", &env, &ctx);
        assert!(!listing(&env, &ctx).contains('*'));
        let shown = listing(&env, &ctx);
        let line = shown
            .lines()
            .position(|l| l.contains("monochrome"))
            .expect("listed")
            + 1;
        run(
            &format!("(goto-line {line}) (theme-list-select)"),
            &env,
            &ctx,
        );
        assert!(listing(&env, &ctx).contains("* monochrome"));
    }

    #[test]
    fn the_listing_is_read_only() {
        let (ctx, env) = editor();
        run("(theme-list)", &env, &ctx);
        let before = listing(&env, &ctx);
        run(r#"(goto-char 0) (insert "typed")"#, &env, &ctx);
        assert_eq!(listing(&env, &ctx), before);
    }

    #[test]
    fn a_line_naming_no_theme_is_refused() {
        let (ctx, env) = editor();
        run("(theme-list)", &env, &ctx);
        run(
            r#"(set-buffer-read-only nil) (end-of-buffer) (insert "  not-a-theme\n")
               (set-buffer-read-only t)"#,
            &env,
            &ctx,
        );
        let line = listing(&env, &ctx)
            .lines()
            .position(|l| l.contains("not-a-theme"))
            .expect("the line was added")
            + 1;
        run(
            &format!("(goto-line {line}) (theme-list-select)"),
            &env,
            &ctx,
        );
        assert!(run("current-theme", &env, &ctx).is_nil());
    }
}
