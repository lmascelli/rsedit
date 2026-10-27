//! Loading a module once: `require` and `provide`, the cycle and the module
//! that provides nothing, a failure taking back what it loaded, a file of the
//! person's own in place of a shipped one, what waits with `eval-after-load`,
//! the plugins and their descriptors, `theme-path`, and the shipped modules
//! loaded the way the default init.lisp loads them.
//!
//! Design: `doc/architettura-editor.typ, <moduli-lisp>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::boot::DEFAULT_INIT_LISP;
    use crate::editor::{EditorState, MODULE_KINDS, create_global_env};
    use crate::tests::shipped::{default_modules, shipped_modules};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// A directory of the test's own, gone when it is.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsedit-loading-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        /// Writes CONTENTS to the file NAME, a path under the sandbox, making
        /// its directories; returns the directory it is in.
        fn file(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            let dir = path.parent().expect("a file has a directory").to_path_buf();
            std::fs::create_dir_all(&dir).expect("making the file's directory");
            std::fs::write(&path, contents).expect("writing a file");
            dir
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
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

    /// Sets the variable NAME to the list of DIRS.
    fn set_path(name: &str, dirs: &[&Path], env: &Arc<Env<Ctx>>) {
        env.set_variable(
            name.into(),
            LispExp::proper_list(
                dirs.iter()
                    .map(|dir| LispExp::string(dir.display().to_string()))
                    .collect(),
            ),
        );
    }

    fn number(exp: &LispExp<Ctx>) -> f64 {
        match exp {
            LispExp::Number(n) => *n,
            other => panic!("not a number: {other:?}"),
        }
    }

    /// Returns ITEM, a symbol or a string, as its text.
    fn text(item: &LispExp<Ctx>) -> String {
        match item {
            LispExp::Symbol(name) | LispExp::String(name) => name.to_string(),
            other => panic!("neither a symbol nor a string: {other:?}"),
        }
    }

    fn logged(ctx: &Ctx, part: &str) -> bool {
        ctx.get_logs().iter().any(|line| line.contains(part))
    }

    // -- once ----------------------------------------------------------------

    #[test]
    fn a_feature_required_twice_is_loaded_once() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("twice");
        let dir = sandbox.file(
            "counted.lisp",
            "(setq counted-loads (+ 1 (if (boundp 'counted-loads) counted-loads 0)))\n\
             (provide 'counted)",
        );
        set_path("lisp-path", &[&dir], &env);
        assert_eq!(
            run("(require 'counted)", &env, &ctx),
            LispExp::symbol("counted".into())
        );
        run("(require 'counted)", &env, &ctx);
        assert_eq!(number(&run("counted-loads", &env, &ctx)), 1.0);
        // `eval-file` is the one that loads again.
        run(r#"(eval-file "counted")"#, &env, &ctx);
        assert_eq!(number(&run("counted-loads", &env, &ctx)), 2.0);
    }

    #[test]
    fn what_is_loaded_is_listed_newest_first_with_its_file() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("listed");
        let dir = sandbox.file("first.lisp", "(provide 'first)");
        sandbox.file("second.lisp", "(require 'first) (provide 'second)");
        set_path("lisp-path", &[&dir], &env);
        run("(require 'second)", &env, &ctx);
        let features: Vec<String> = run("(features)", &env, &ctx)
            .iter()
            .map(|item| text(&item))
            .collect();
        let at = |name: &str| features.iter().position(|item| item == name);
        assert!(at("second") < at("first"), "{features:?}");
        assert_eq!(
            run("(feature-file 'first)", &env, &ctx),
            LispExp::string(dir.join("first.lisp").display().to_string())
        );
        assert_eq!(
            run(
                "(list (featurep 'second) (feature-status 'second))",
                &env,
                &ctx
            ),
            run("'(t loaded)", &env, &ctx)
        );
        assert!(run("(featurep 'third)", &env, &ctx).is_nil());
        assert!(run("(feature-status 'third)", &env, &ctx).is_nil());
    }

    #[test]
    fn a_feature_with_no_file_is_logged_and_answered_nil() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("missing");
        set_path("lisp-path", &[&sandbox.0], &env);
        assert!(run("(require 'nowhere)", &env, &ctx).is_nil());
        assert!(logged(&ctx, "require nowhere"));
        assert!(run("(feature-status 'nowhere)", &env, &ctx).is_nil());
    }

    #[test]
    fn a_file_named_outright_is_loaded_for_the_feature() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("named");
        let dir = sandbox.file("elsewhere.lisp", "(provide 'here)");
        let file = dir.join("elsewhere.lisp").display().to_string();
        run(&format!("(require 'here {file:?})"), &env, &ctx);
        assert!(!run("(featurep 'here)", &env, &ctx).is_nil());
    }

    // -- failures ------------------------------------------------------------

    #[test]
    fn a_file_that_requires_itself_is_an_error() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("cycle");
        let dir = sandbox.file("ping.lisp", "(require 'pong) (provide 'ping)");
        sandbox.file("pong.lisp", "(require 'ping) (provide 'pong)");
        set_path("lisp-path", &[&dir], &env);
        let why = format!("{:?}", try_run("(require 'ping)", &env, &ctx).unwrap_err());
        assert!(why.contains("ping -> pong -> ping"), "{why}");
        assert_eq!(
            run(
                "(list (feature-status 'ping) (feature-status 'pong))",
                &env,
                &ctx
            ),
            run("'(failed failed)", &env, &ctx)
        );
    }

    #[test]
    fn a_file_that_does_not_provide_its_feature_is_an_error() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("unprovided");
        let dir = sandbox.file("silent.lisp", "(setq silent-loaded t)");
        set_path("lisp-path", &[&dir], &env);
        let why = format!(
            "{:?}",
            try_run("(require 'silent)", &env, &ctx).unwrap_err()
        );
        assert!(why.contains("did not provide silent"), "{why}");
        assert_eq!(
            run("(feature-status 'silent)", &env, &ctx),
            LispExp::symbol("failed".into())
        );
    }

    #[test]
    fn a_file_that_does_not_parse_says_so() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("unparsed");
        let dir = sandbox.file("broken.lisp", "(provide 'broken");
        set_path("lisp-path", &[&dir], &env);
        let why = format!(
            "{:?}",
            try_run("(require 'broken)", &env, &ctx).unwrap_err()
        );
        assert!(why.contains("does not parse"), "{why}");
    }

    #[test]
    fn a_failed_load_takes_back_what_it_loaded_on_its_way() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("rollback");
        let dir = sandbox.file("helper.lisp", "(provide 'helper)");
        sandbox.file(
            "fragile.lisp",
            "(require 'helper) (no-such-function) (provide 'fragile)",
        );
        set_path("lisp-path", &[&dir], &env);
        assert!(try_run("(require 'fragile)", &env, &ctx).is_err());
        assert!(run("(featurep 'helper)", &env, &ctx).is_nil());
        assert!(run("(featurep 'fragile)", &env, &ctx).is_nil());
        // Fixed, it loads at the next asking.
        sandbox.file("fragile.lisp", "(require 'helper) (provide 'fragile)");
        run("(require 'fragile)", &env, &ctx);
        assert!(!run("(featurep 'helper)", &env, &ctx).is_nil());
    }

    // -- where -----------------------------------------------------------------

    #[test]
    fn a_file_in_an_earlier_directory_shadows_one_later_and_the_log_says_so() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("shadow");
        let mine = sandbox.file("mine/thing.lisp", "(setq thing 'mine) (provide 'thing)");
        let shipped = sandbox.file(
            "shipped/thing.lisp",
            "(setq thing 'shipped) (provide 'thing)",
        );
        set_path("lisp-path", &[&mine, &shipped], &env);
        run("(require 'thing)", &env, &ctx);
        assert_eq!(run("thing", &env, &ctx), LispExp::symbol("mine".into()));
        assert!(logged(&ctx, "shadows"));
    }

    #[test]
    fn eval_file_still_finds_a_module_by_its_bare_name() {
        // What an init.lisp written before `require` says.
        let (ctx, env) = editor();
        let shipped = crate::paths::installed_data_dir()
            .expect("the data beside the test binary")
            .join("lisp");
        let dirs: Vec<PathBuf> = MODULE_KINDS.iter().map(|kind| shipped.join(kind)).collect();
        let dirs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
        set_path("lisp-path", &dirs, &env);
        run(r#"(eval-file "commands") (eval-file "debug")"#, &env, &ctx);
        assert!(env.get_macro("defcommand").is_some());
        assert!(!run("(featurep 'debug)", &env, &ctx).is_nil());
    }

    // -- waiting -----------------------------------------------------------

    #[test]
    fn what_waits_for_a_feature_runs_when_it_is_provided_or_at_once() {
        let (ctx, env) = editor();
        run(
            "(setq waited nil)
             (eval-after-load 'later (lambda () (setq waited (cons 'first waited))))
             (eval-after-load 'later (lambda () (setq waited (cons 'second waited))))",
            &env,
            &ctx,
        );
        assert!(run("waited", &env, &ctx).is_nil(), "nothing until it is");
        run("(provide 'later)", &env, &ctx);
        assert_eq!(
            run("waited", &env, &ctx),
            run("'(second first)", &env, &ctx)
        );
        assert_eq!(
            run("(eval-after-load 'later (lambda () 'now))", &env, &ctx),
            LispExp::symbol("now".into())
        );
    }

    #[test]
    fn with_eval_after_load_waits_with_a_body() {
        let (ctx, env) = editor();
        run(include_str!("../../lisp/base/commands.lisp"), &env, &ctx);
        run(
            "(setq waited nil) (with-eval-after-load 'soon (setq waited 'done))",
            &env,
            &ctx,
        );
        run("(provide 'soon)", &env, &ctx);
        assert_eq!(run("waited", &env, &ctx), LispExp::symbol("done".into()));
    }

    #[test]
    fn a_failure_in_what_waited_is_logged_and_the_rest_still_run() {
        let (ctx, env) = editor();
        run(
            "(setq waited nil)
             (eval-after-load 'shaky (lambda () (no-such-function)))
             (eval-after-load 'shaky (lambda () (setq waited t)))
             (provide 'shaky)",
            &env,
            &ctx,
        );
        assert!(!run("waited", &env, &ctx).is_nil());
        assert!(logged(&ctx, "After loading shaky"));
    }

    // -- plugins -------------------------------------------------------------

    #[test]
    fn a_plugin_is_what_its_descriptor_says_and_nothing_of_it_runs() {
        let (ctx, env) = editor();
        let sandbox = Sandbox::new("plugins");
        sandbox.file(
            "hello/plugin.lisp",
            r#"(define-plugin hello :version "0.1" :requires (dired help)
                 :description "Says hello.")
               (setq ran t)"#,
        );
        sandbox.file("hello/hello.lisp", "(setq hello-loaded t) (provide 'hello)");
        sandbox.file("misnamed/plugin.lisp", "(define-plugin other)");
        sandbox.file("bare/readme.txt", "no descriptor");
        let plugins = ctx.find_plugins(&sandbox.0);
        assert_eq!(plugins.len(), 1, "{plugins:?}");
        let hello = &plugins[0];
        assert_eq!(hello.name, "hello");
        assert_eq!(hello.version.as_deref(), Some("0.1"));
        assert_eq!(hello.requires, ["dired", "help"]);
        assert_eq!(hello.description.as_deref(), Some("Says hello."));
        assert!(logged(&ctx, "misnamed"), "a plugin passed over says why");
        assert!(env.get_variable("ran").is_none(), "read, not evaluated");
        assert!(env.get_variable("hello-loaded").is_none(), "nothing loaded");

        ctx.loader_mut(|loader| loader.set_plugins(plugins.clone()));
        assert_eq!(run("(plugins)", &env, &ctx), run("'(hello)", &env, &ctx));
        assert_eq!(
            run("(cdr (assoc 'version (plugin-info 'hello)))", &env, &ctx),
            LispExp::string("0.1".into())
        );
        assert_eq!(
            run("(cdr (assoc 'requires (plugin-info 'hello)))", &env, &ctx),
            run("'(dired help)", &env, &ctx)
        );
        assert!(run("(plugin-info 'misnamed)", &env, &ctx).is_nil());
        set_path("lisp-path", &[&hello.directory], &env);
        run("(require 'hello)", &env, &ctx);
        assert!(env.get_variable("hello-loaded").is_some());
    }

    #[test]
    fn the_editor_looks_in_the_persons_folders_first_and_its_own_last() {
        let (ctx, env) = editor();
        let config = crate::paths::app_dir(crate::paths::DirOptions {
            dir: crate::paths::AppDir::Config,
            create: false,
        })
        .expect("the suite's own configuration");
        let strings = |name: &str| -> Vec<String> {
            run(name, &env, &ctx)
                .iter()
                .map(|item| match item {
                    LispExp::String(text) => text.to_string(),
                    other => panic!("{name} holds {other:?}"),
                })
                .collect()
        };
        let lisp = strings("lisp-path");
        assert_eq!(
            lisp.first(),
            Some(&config.join("lisp").display().to_string())
        );
        let tail: Vec<&str> = lisp
            .iter()
            .rev()
            .take(3)
            .map(|dir| Path::new(dir).file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(tail, ["languages", "features", "base"]);
        let themes = strings("theme-path");
        assert_eq!(
            themes.first(),
            Some(&config.join("themes").display().to_string())
        );
        assert!(themes.last().is_some_and(|dir| dir.ends_with("themes")));
    }

    // -- themes --------------------------------------------------------------

    #[test]
    fn a_theme_of_ones_own_shadows_a_shipped_one_of_the_same_name() {
        let (ctx, env) = editor();
        for module in ["commands", "debug", "theme"] {
            run(&crate::tests::shipped::module_source(module), &env, &ctx);
        }
        let sandbox = Sandbox::new("themes");
        let mine = sandbox.file(
            "mine/monochrome.lisp",
            "(put 'monochrome-theme 'faces '((keyword \"red\" nil nil)))",
        );
        sandbox.file(
            "mine/zzz-mine.lisp",
            "(put 'zzz-mine-theme 'faces '((keyword \"blue\" nil nil)))",
        );
        let shipped = crate::paths::installed_data_dir()
            .expect("the data beside the test binary")
            .join("themes");
        set_path("theme-path", &[&mine, &shipped], &env);
        let names: Vec<String> = run("(available-themes)", &env, &ctx)
            .iter()
            .map(|item| text(&item))
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names, sorted, "sorted, each once");
        assert!(names.contains(&"zzz-mine".to_string()), "{names:?}");
        assert!(names.contains(&"nord".to_string()), "{names:?}");
        run("(select-theme 'monochrome)", &env, &ctx);
        assert_eq!(
            run("(nth 1 (car (get 'monochrome-theme 'faces)))", &env, &ctx),
            LispExp::string("red".into())
        );
    }

    // -- the shipped modules ---------------------------------------------------

    #[test]
    fn the_default_init_loads_every_shipped_module_each_from_its_own_file() {
        let (ctx, env) = editor();
        let shipped = crate::paths::installed_data_dir()
            .expect("the data beside the test binary")
            .join("lisp");
        let dirs: Vec<PathBuf> = MODULE_KINDS.iter().map(|kind| shipped.join(kind)).collect();
        let dirs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
        set_path("lisp-path", &dirs, &env);
        run(DEFAULT_INIT_LISP, &env, &ctx);
        for (kind, name) in shipped_modules() {
            let file = run(&format!("(feature-file '{name})"), &env, &ctx);
            let LispExp::String(file) = &file else {
                panic!("{name} is not loaded: {file:?}");
            };
            assert!(
                Path::new(file.as_str()).ends_with(Path::new(&kind).join(format!("{name}.lisp"))),
                "{name} came from {file}"
            );
        }
    }

    #[test]
    fn every_shipped_module_is_required_by_the_default_init_and_kept_by_kind() {
        let mut required = default_modules();
        required.sort();
        let mut shipped: Vec<String> = shipped_modules()
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        shipped.sort();
        assert_eq!(required, shipped);

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("lisp");
        let mut kinds: Vec<String> = std::fs::read_dir(&root)
            .expect("core/lisp")
            .map(|entry| entry.expect("an entry").path())
            .filter(|path| path.is_dir())
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        kinds.sort();
        assert_eq!(
            kinds, MODULE_KINDS,
            "what lisp-path lists is what core/lisp holds"
        );
        let loose: Vec<PathBuf> = std::fs::read_dir(&root)
            .expect("core/lisp")
            .map(|entry| entry.expect("an entry").path())
            .filter(|path| path.is_file())
            .collect();
        assert!(loose.is_empty(), "a module belongs in a kind: {loose:?}");
    }
}
