//! The `lsp` module against a server made for the tests: a shell script that
//! keeps a log of every message it is sent, answers initialize and shutdown,
//! reports one finding for each version of a document it hears of, and falls
//! when a change brings the word "fall".
//!
//! Design: `doc/architettura-editor.typ, <mod-lsp>`.
#[cfg(all(test, unix))]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    type Ctx = EditorState<GapBuffer>;

    /// The server. Its arguments are the file it logs to and the position
    /// encoding it chooses.
    const SERVER: &str = r#"log="$1"
encoding="$2"
send() { printf 'Content-Length: %s\r\n\r\n%s' "${#1}" "$1"; }
len=0
while IFS= read -r line; do
  line=$(printf '%s' "$line" | tr -d '\r')
  case "$line" in
    Content-Length:*) len=${line#Content-Length: } ;;
    "")
      body=$(dd bs=1 count="$len" 2>/dev/null)
      printf '%s\n' "$body" >> "$log"
      id=$(printf '%s' "$body" | sed -n 's/^{"id":\([0-9]*\),.*/\1/p')
      case "$body" in
        *'"method":"initialize"'*)
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"capabilities\":{\"positionEncoding\":\"$encoding\",\"textDocumentSync\":{\"change\":1,\"openClose\":true,\"save\":true}}}}" ;;
        *'"method":"shutdown"'*)
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":null}" ;;
        *'"method":"exit"'*)
          exit 0 ;;
        *'"method":"textDocument/didChange"'*fall*)
          exit 3 ;;
        *'"method":"textDocument/didOpen"'*|*'"method":"textDocument/didChange"'*)
          uri=$(printf '%s' "$body" | sed -n 's/.*"uri":"\([^"]*\)".*/\1/p')
          version=$(printf '%s' "$body" | sed -n 's/.*"version":\([0-9]*\).*/\1/p')
          send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{\"diagnostics\":[{\"code\":7,\"message\":\"version $version\",\"range\":{\"end\":{\"character\":5,\"line\":0},\"start\":{\"character\":3,\"line\":0}},\"severity\":2}],\"uri\":\"$uri\",\"version\":$version}}" ;;
      esac ;;
  esac
done
"#;

    /// A directory of this test's own, with the server's script and log,
    /// removed when it goes.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsedit-lsp-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            std::fs::write(dir.join("server.sh"), SERVER).expect("writing the server");
            Sandbox(dir)
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).display().to_string()
        }

        /// The directory, ending in a separator, as a root is.
        fn root(&self) -> String {
            format!("{}/", self.0.display())
        }

        fn file(&self, name: &str, contents: &str) -> String {
            std::fs::write(self.0.join(name), contents).expect("writing a file");
            self.path(name)
        }

        /// The messages the server has been sent, one per line.
        fn log(&self) -> String {
            std::fs::read_to_string(self.0.join("log")).unwrap_or_default()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::String(s) => s.to_string(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    /// An editor with the module loaded, and the test server as the one for
    /// fundamental-mode, choosing ENCODING, with its root in SANDBOX.
    fn editor(sandbox: &Sandbox, encoding: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/dired.lisp"),
            include_str!("../../lisp/features/occur.lisp"),
            include_str!("../../lisp/features/shell.lisp"),
            include_str!("../../lisp/features/compile.lisp"),
            include_str!("../../lisp/features/project.lisp"),
            include_str!("../../lisp/features/lsp.lisp"),
        ] {
            run(source, &env, &ctx);
        }
        run(
            &format!(
                r#"(setq lsp-servers
                     (list (list 'fundamental-mode "fake"
                                 (list "sh" "{script}" "{log}" "{encoding}")
                                 :language "rust"
                                 :settings (make-map "fake" (make-map "depth" 2)))))
                   (defun test-root (file) "{root}")
                   (setq lsp-root-function 'test-root)
                   (minor-mode-enable 'lsp-mode 'fundamental-mode)"#,
                script = sandbox.path("server.sh"),
                log = sandbox.path("log"),
                root = sandbox.root(),
            ),
            &env,
            &ctx,
        );
        (ctx, env)
    }

    /// The name of the server for SANDBOX, as a Lisp string.
    fn server(sandbox: &Sandbox) -> String {
        format!("\"lsp fake {}\"", sandbox.root())
    }

    /// Does what the host does between two keys: lets 20 ms of the
    /// scheduler's time pass, which the clock of the module counts by, then
    /// makes the deliveries and calls the callbacks waiting.
    fn step(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        ctx.work().run_for(ctx, Duration::from_millis(20));
        if !ctx.tick(env) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Runs the host's ticks until CONDITION, a Lisp form, is true, or gives
    /// up.
    fn tick_until(condition: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !run(condition, env, ctx).is_truthy() {
            assert!(
                Instant::now() < deadline,
                "{condition} did not come true within ten seconds"
            );
            step(env, ctx);
        }
    }

    /// Runs the host's ticks until the server's log has a line holding all
    /// of PARTS, and returns that line.
    fn logged(sandbox: &Sandbox, parts: &[&str], env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(line) = sandbox
                .log()
                .lines()
                .find(|line| parts.iter().all(|part| line.contains(part)))
            {
                return line.to_string();
            }
            assert!(
                Instant::now() < deadline,
                "no message with {parts:?} in ten seconds; the server was sent:\n{}",
                sandbox.log()
            );
            step(env, ctx);
        }
    }

    /// Opens PATH, and returns its buffer's name.
    fn open(path: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text(&run(&format!(r#"(find-file "{path}")"#), env, ctx))
    }

    /// Runs the host's ticks until the server is ready.
    fn ready(sandbox: &Sandbox, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        tick_until(
            &format!("(eq (get {} 'state) 'ready)", server(sandbox)),
            env,
            ctx,
        );
    }

    /// The message of the one finding about FILE, once it says WANTED.
    fn finding_says(file: &str, wanted: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        tick_until(
            &format!(
                r#"(let ((found (findings-for-file "{file}")))
                     (and found (null (cdr found))
                          (equal (nth 1 (memq :message (car found))) "{wanted}")))"#
            ),
            env,
            ctx,
        );
    }

    /// Inserts TEXT at the end of the buffer NAME, and runs what follows a
    /// command.
    fn type_at_end(name: &str, text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(
                r#"(with-current-buffer "{name}"
                     (lambda () (goto-char (point-max)) (insert "{text}")))
                   (lsp--after-command)"#
            ),
            env,
            ctx,
        );
    }

    #[test]
    fn the_first_buffer_starts_the_server_which_then_gets_its_whole_text() {
        let sandbox = Sandbox::new("open");
        let file = sandbox.file("a.fake", "fn main() {}\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        open(&file, &env, &ctx);
        let initialize = logged(&sandbox, &[r#""method":"initialize""#], &env, &ctx);
        for part in [
            r#""positionEncodings":["utf-32","utf-16"]"#,
            r#""processId":null"#,
            &format!(r#""rootUri":"file://{}""#, sandbox.0.display()),
            r#""versionSupport":true"#,
        ] {
            assert!(initialize.contains(part), "{part} not in {initialize}");
        }
        logged(&sandbox, &[r#""method":"initialized""#], &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""method":"textDocument/didOpen""#,
                r#""languageId":"rust""#,
                r#""text":"fn main() {}\n""#,
                &format!(r#""uri":"file://{file}""#),
                r#""version":1"#,
            ],
            &env,
            &ctx,
        );
    }

    #[test]
    fn a_change_sends_the_whole_text_as_the_next_version() {
        let sandbox = Sandbox::new("change");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        type_at_end(&name, "two\\n", &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""method":"textDocument/didChange""#,
                r#""contentChanges":[{"text":"one\ntwo\n"}]"#,
                r#""version":2"#,
            ],
            &env,
            &ctx,
        );
        // Nothing changed, nothing sent.
        run("(lsp--after-command)", &env, &ctx);
        type_at_end(&name, "three\\n", &env, &ctx);
        let third = logged(&sandbox, &[r#""version":3"#], &env, &ctx);
        assert!(third.contains(r#""text":"one\ntwo\nthree\n""#), "{third}");
    }

    #[test]
    fn what_the_server_reports_becomes_the_findings_of_the_file() {
        let sandbox = Sandbox::new("findings");
        let file = sandbox.file("a.fake", "let x = 1;\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        let finding = run(
            &format!(r#"(car (findings-for-file "{file}"))"#),
            &env,
            &ctx,
        );
        let shown = format!("{finding:?}");
        for part in ["warning", "\"7\"", "lsp fake"] {
            assert!(shown.contains(part), "{part} not in {shown}");
        }
        assert!(
            run(
                &format!(
                    r#"(let ((f (car (findings-for-file "{file}"))))
                         (and (equal (nth 1 (memq :start f)) '(1 . 3))
                              (equal (nth 1 (memq :end f)) '(1 . 5))))"#
                ),
                &env,
                &ctx
            )
            .is_truthy(),
            "{shown}"
        );
    }

    #[test]
    fn columns_in_utf_16_are_read_in_the_text_of_the_buffer() {
        let sandbox = Sandbox::new("utf16");
        // The face is two UTF-16 units and one character: unit 3 is the a.
        let file = sandbox.file("a.fake", "\u{1F600} abcd\n");
        let (ctx, env) = editor(&sandbox, "utf-16");
        open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        assert!(
            run(
                &format!(
                    r#"(let ((f (car (findings-for-file "{file}"))))
                         (and (equal (nth 1 (memq :start f)) '(1 . 2))
                              (equal (nth 1 (memq :end f)) '(1 . 4))))"#
                ),
                &env,
                &ctx
            )
            .is_truthy(),
            "{:?}",
            run(&format!(r#"(findings-for-file "{file}")"#), &env, &ctx)
        );
    }

    #[test]
    fn findings_about_an_older_version_are_dropped() {
        let sandbox = Sandbox::new("stale");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        type_at_end(&name, "two\\n", &env, &ctx);
        finding_says(&file, "version 2", &env, &ctx);
        // What a slow server says late about the first version.
        let late = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"file://{file}","version":1,"diagnostics":[]}}}}"#
        );
        run(
            &format!(r#"(lsp--deliver {} 'output {:?})"#, server(&sandbox), late),
            &env,
            &ctx,
        );
        finding_says(&file, "version 2", &env, &ctx);
        // Without a version it is taken: here, no findings at all.
        let unversioned = late.replace(r#""version":1,"#, "");
        run(
            &format!(
                r#"(lsp--deliver {} 'output {:?})"#,
                server(&sandbox),
                unversioned
            ),
            &env,
            &ctx,
        );
        assert!(run(&format!(r#"(findings-for-file "{file}")"#), &env, &ctx).is_nil());
    }

    #[test]
    fn a_server_that_falls_is_started_again_and_given_its_documents() {
        let sandbox = Sandbox::new("fall");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        type_at_end(&name, "fall\\n", &env, &ctx);
        let initializes = || sandbox.log().matches(r#""method":"initialize""#).count();
        let deadline = Instant::now() + Duration::from_secs(10);
        while initializes() < 2 {
            assert!(
                Instant::now() < deadline,
                "not started again:\n{}",
                sandbox.log()
            );
            step(&env, &ctx);
        }
        ready(&sandbox, &env, &ctx);
        // The new server has the text of now, as its version 1.
        logged(
            &sandbox,
            &[
                r#""method":"textDocument/didOpen""#,
                r#""text":"one\nfall\n""#,
                r#""version":1"#,
            ],
            &env,
            &ctx,
        );
        finding_says(&file, "version 1", &env, &ctx);
    }

    #[test]
    fn a_server_that_falls_too_often_stays_down_until_restarted() {
        let sandbox = Sandbox::new("down");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        run("(setq lsp-restart-limit 0)", &env, &ctx);
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        type_at_end(&name, "fall\\n", &env, &ctx);
        tick_until(
            &format!("(eq (get {} 'state) 'dead)", server(&sandbox)),
            &env,
            &ctx,
        );
        // What it said went with it.
        assert!(run(&format!(r#"(findings-for-file "{file}")"#), &env, &ctx).is_nil());
        assert_eq!(sandbox.log().matches(r#""method":"initialize""#).count(), 1);
        run(
            &format!(r#"(with-current-buffer "{name}" (lambda () (lsp-restart)))"#),
            &env,
            &ctx,
        );
        ready(&sandbox, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
    }

    #[test]
    fn shutting_a_server_down_asks_it_first_and_takes_its_findings() {
        let sandbox = Sandbox::new("shutdown");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        run(
            &format!(r#"(with-current-buffer "{name}" (lambda () (lsp-shutdown)))"#),
            &env,
            &ctx,
        );
        tick_until(
            &format!("(eq (get {} 'state) 'dead)", server(&sandbox)),
            &env,
            &ctx,
        );
        logged(&sandbox, &[r#""method":"shutdown""#], &env, &ctx);
        logged(&sandbox, &[r#""method":"exit""#], &env, &ctx);
        assert!(run(&format!(r#"(findings-for-file "{file}")"#), &env, &ctx).is_nil());
    }

    #[test]
    fn the_server_is_answered_its_questions_even_those_not_known() {
        let sandbox = Sandbox::new("questions");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        let asked = r#"{"id":41,"jsonrpc":"2.0","method":"workspace/configuration","params":{"items":[{"section":"fake.depth"},{"section":"other"}]}}"#;
        let unknown = r#"{"id":42,"jsonrpc":"2.0","method":"made/up"}"#;
        for question in [asked, unknown] {
            run(
                &format!(
                    r#"(lsp--deliver {} 'output {:?})"#,
                    server(&sandbox),
                    question
                ),
                &env,
                &ctx,
            );
        }
        logged(
            &sandbox,
            &[r#""id":41"#, r#""result":[2,null]"#],
            &env,
            &ctx,
        );
        logged(&sandbox, &[r#""id":42"#, r#""code":-32601"#], &env, &ctx);
    }

    #[test]
    fn lsp_mode_where_there_is_no_file_or_no_server_turns_itself_off() {
        let sandbox = Sandbox::new("off");
        let (ctx, env) = editor(&sandbox, "utf-32");
        run(
            r#"(buffer-create "plain")
               (with-current-buffer "plain" (lambda () (lsp-mode 1)))"#,
            &env,
            &ctx,
        );
        assert!(run(r#"(minor-mode-p 'lsp-mode "plain")"#, &env, &ctx).is_nil());
        run("(setq lsp-servers nil)", &env, &ctx);
        let file = sandbox.file("b.fake", "two\n");
        let name = open(&file, &env, &ctx);
        assert!(run(&format!(r#"(minor-mode-p 'lsp-mode "{name}")"#), &env, &ctx).is_nil());
        assert!(run("lsp--servers", &env, &ctx).is_nil());
    }

    #[test]
    fn saving_and_closing_a_buffer_are_told_to_the_server() {
        let sandbox = Sandbox::new("save");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        type_at_end(&name, "two\\n", &env, &ctx);
        run(
            &format!(r#"(with-current-buffer "{name}" (lambda () (save-buffer)))"#),
            &env,
            &ctx,
        );
        let saved = logged(
            &sandbox,
            &[r#""method":"textDocument/didSave""#],
            &env,
            &ctx,
        );
        // The change went before the save.
        let log = sandbox.log();
        assert!(log.find(r#""version":2"#) < log.find(&saved), "{log}");
        run(&format!(r#"(close-buffer "{name}")"#), &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""method":"textDocument/didClose""#,
                &format!(r#""uri":"file://{file}""#),
            ],
            &env,
            &ctx,
        );
        assert!(
            run(
                &format!("(get {} 'documents)", server(&sandbox)),
                &env,
                &ctx
            )
            .is_nil()
        );
    }

    #[test]
    fn a_question_without_an_answer_is_dropped_after_the_timeout() {
        let sandbox = Sandbox::new("timeout");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        run(
            &format!(
                r#"(setq lsp-request-timeout 0.5)
                   (lsp--request {} "made/up" nil 'ignore :timed t)"#,
                server(&sandbox)
            ),
            &env,
            &ctx,
        );
        assert!(run(&format!("(get {} 'requests)", server(&sandbox)), &env, &ctx).is_truthy());
        tick_until(
            &format!("(null (get {} 'requests))", server(&sandbox)),
            &env,
            &ctx,
        );
    }

    #[test]
    fn a_root_not_known_yet_is_asked_for_again_after_the_next_command() {
        let sandbox = Sandbox::new("later");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        run(
            &format!(
                r#"(defvar test-asked 0)
                   (defun test-slow-root (file)
                     (setq test-asked (+ test-asked 1))
                     (if (< test-asked 2) 'timeout "{}"))
                   (setq lsp-root-function 'test-slow-root)"#,
                sandbox.root()
            ),
            &env,
            &ctx,
        );
        let name = open(&file, &env, &ctx);
        assert_eq!(
            run("lsp--waiting", &env, &ctx),
            LispExp::proper_list(vec![LispExp::string(name)])
        );
        assert!(run("lsp--servers", &env, &ctx).is_nil());
        run("(lsp--after-command)", &env, &ctx);
        assert!(run("lsp--waiting", &env, &ctx).is_nil());
        finding_says(&file, "version 1", &env, &ctx);
    }

    #[test]
    fn a_file_and_its_uri_are_read_both_ways_escapes_included() {
        let sandbox = Sandbox::new("uri");
        let (ctx, env) = editor(&sandbox, "utf-32");
        for (file, uri) in [
            ("/src/a b/#1.rs", "file:///src/a%20b/%231.rs"),
            ("/src/caffè.rs", "file:///src/caffè.rs"),
            ("C:\\src\\a.rs", "file:///C:/src/a.rs"),
        ] {
            assert_eq!(
                text(&run(&format!(r#"(lsp--uri "{file}")"#), &env, &ctx)),
                uri
            );
        }
        for (uri, file) in [
            ("file:///src/a%20b/%231.rs", "/src/a b/#1.rs"),
            ("file:///src/caff%C3%A8.rs", "/src/caffè.rs"),
            ("file:///src/%F0%9F%98%80.rs", "/src/\u{1F600}.rs"),
            ("file:///C:/src/a.rs", "C:/src/a.rs"),
        ] {
            assert_eq!(
                text(&run(&format!(r#"(lsp--file "{uri}")"#), &env, &ctx)),
                file
            );
        }
        assert!(run(r#"(lsp--file "untitled:1")"#, &env, &ctx).is_nil());
    }
}
