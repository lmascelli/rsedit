//! The buffers the editor writes -- help, the log, a backtrace, a manual
//! page, the themes, a completion strip, a search's or a command's output --
//! hold nothing of yours to save, and are never shown as modified: no ** in
//! their status line, no * in the buffer list. Text you type is yours, and
//! still is.
//!
//! Design: `doc/architettura-editor.typ, <mod-buffer-list>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::tests::shipped::load_default_modules;
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// The editor as init.lisp makes it, working in a directory of its own
    /// with a file for the searches to find.
    fn editor(name: &str) -> (Ctx, Arc<Env<Ctx>>, std::path::PathBuf) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(100.0));
        env.set_variable("frame-height".into(), LispExp::number(30.0));
        load_default_modules(&env, &ctx);
        let dir = std::env::temp_dir().join(format!(
            "rsedit-listing-modified-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("sandbox");
        std::fs::write(dir.join("f.txt"), "needle one\nhay\nneedle two\n").expect("write");
        run(&format!(r#"(cd "{}")"#, dir.display()), &env, &ctx);
        (ctx, env, dir)
    }

    fn modified(name: &str, ctx: &Ctx) -> bool {
        ctx.with_buffer(name, |buf| buf.is_modified)
            .unwrap_or_else(|| panic!("{name} should exist"))
    }

    /// Lets the work in the background run until the buffer NAME holds
    /// WANTED, or a few seconds have gone.
    fn wait_for(name: &str, wanted: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for _ in 0..250 {
            ctx.tick(env);
            if ctx
                .with_buffer(name, |buf| buf.text.to_string().contains(wanted))
                .unwrap_or(false)
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("{name} never showed {wanted:?}");
    }

    #[test]
    fn what_the_editor_writes_in_lisp_is_never_modified() {
        let (ctx, env, dir) = editor("lisp");
        for (src, name) in [
            (r#"(help-function "car")"#, "*Help*"),
            (
                r#"(switch-to-buffer "*scratch*") (describe-bindings)"#,
                "*Help*",
            ),
            ("(switch-to-messages)", "*Messages*"),
            (r#"(manpage "dired")"#, "*Manual*"),
            ("(theme-list)", "*Themes*"),
            (r#"(backtrace-show "boom" '("f" "g"))"#, "*Backtrace*"),
            ("(risp-repl)", "*risp*"),
        ] {
            run(src, &env, &ctx);
            assert!(!modified(name, &ctx), "{name} after {src}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_completion_strip_is_never_modified() {
        let (ctx, env, dir) = editor("completion");
        run(
            r#"(minibuffer-read "Pick:" nil (lambda (input) (list "alpha" "beta")) nil)"#,
            &env,
            &ctx,
        );
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::default(),
            },
            &env,
        );
        assert!(!modified("*Completions*", &ctx));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_searchs_or_a_commands_output_is_never_modified() {
        let (ctx, env, dir) = editor("output");
        run(
            r#"(switch-to-buffer "*scratch*") (insert "needle here\n")"#,
            &env,
            &ctx,
        );
        run(r#"(occur "needle")"#, &env, &ctx);
        wait_for("*Occur*", "needle here", &env, &ctx);
        assert!(!modified("*Occur*", &ctx));

        run(r#"(grep "needle")"#, &env, &ctx);
        wait_for("*Grep*", "needle two", &env, &ctx);
        assert!(!modified("*Grep*", &ctx));

        run(r#"(shell-command "echo shell-said-this")"#, &env, &ctx);
        wait_for("*Shell Output*", "exited", &env, &ctx);
        assert!(!modified("*Shell Output*", &ctx));
        let status = ctx
            .snapshot(&env, 100, 30)
            .views
            .iter()
            .find(|view| view.buffer_name == "*Shell Output*")
            .and_then(|view| view.mode_line.clone())
            .unwrap_or_default();
        assert!(!status.contains("**"), "{status}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_you_type_still_makes_a_buffer_modified() {
        let (ctx, env, dir) = editor("typed");
        run(
            r#"(switch-to-buffer "*scratch*") (insert "mine")"#,
            &env,
            &ctx,
        );
        assert!(modified("*scratch*", &ctx));
        run(r#"(risp-repl) (insert "(+ 1 2)")"#, &env, &ctx);
        assert!(
            modified("*risp*", &ctx),
            "what you write in the REPL is yours"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
