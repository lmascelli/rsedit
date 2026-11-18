//! The `lsp` module against a server made for the tests: a shell script that
//! keeps a log of every message it is sent, answers initialize and shutdown,
//! reports one finding for each version of a document it hears of, falls
//! when a change brings the word "fall", and gives places in `target.txt`:
//! the definition at line 0 in a Location, at line 1 in a LocationLink, none
//! below; two references; code actions that change `a.fake` and `b.fake`,
//! one to be resolved, one a command that it carries out with applyEdit;
//! completions, on line 1 with changes elsewhere, on line 2 with a range.
//! It keeps the document's text as it is told it, rebuilt from the changes
//! it receives, with columns in bytes.
//!
//! Design: `doc/architettura-editor.typ, <mod-lsp>`.
#[cfg(all(test, unix))]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent};
    use risp::{Env, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    type Ctx = EditorState<GapBuffer>;

    /// The server. Its arguments are the file it logs to, the position
    /// encoding it chooses, and how it wants changes: 1, the default, as the
    /// whole text, 2 as the stretch changed.
    const SERVER: &str = r#"log="$1"
encoding="$2"
sync="${3:-1}"
dir="${log%/*}"
send() { printf 'Content-Length: %s\r\n\r\n%s' "${#1}" "$1"; }
at() { printf '{"end":{"character":0,"line":%s},"start":{"character":0,"line":%s}}' "$1" "$1"; }
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
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"capabilities\":{\"codeActionProvider\":{\"resolveProvider\":true},\"completionProvider\":{},\"definitionProvider\":true,\"positionEncoding\":\"$encoding\",\"textDocumentSync\":{\"change\":$sync,\"openClose\":true,\"save\":true},\"referencesProvider\":true}}}" ;;
        *'"method":"shutdown"'*)
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":null}" ;;
        *'"method":"exit"'*)
          exit 0 ;;
        *'"method":"textDocument/didChange"'*fall*)
          exit 3 ;;
        *'"method":"textDocument/definition"'*)
          target="file://${log%/*}/target.txt"
          case $(printf '%s' "$body" | sed -n 's/.*"line":\([0-9]*\).*/\1/p') in
            0) result="{\"range\":{\"end\":{\"character\":5,\"line\":1},\"start\":{\"character\":2,\"line\":1}},\"uri\":\"$target\"}" ;;
            1) result="[{\"targetRange\":{\"end\":{\"character\":0,\"line\":3},\"start\":{\"character\":0,\"line\":2}},\"targetSelectionRange\":{\"end\":{\"character\":3,\"line\":2},\"start\":{\"character\":2,\"line\":2}},\"targetUri\":\"$target\"}]" ;;
            *) result=null ;;
          esac
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":$result}" ;;
        *'"method":"textDocument/references"'*)
          target="file://${log%/*}/target.txt"
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":[{\"range\":{\"end\":{\"character\":2,\"line\":0},\"start\":{\"character\":1,\"line\":0}},\"uri\":\"$target\"},{\"range\":{\"end\":{\"character\":4,\"line\":2},\"start\":{\"character\":3,\"line\":2}},\"uri\":\"$target\"}]}" ;;
        *'"method":"textDocument/codeAction"'*)
          a="file://${log%/*}/a.fake"
          b="file://${log%/*}/b.fake"
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":[{\"disabled\":{\"reason\":\"no\"},\"title\":\"Off\"},{\"edit\":{\"changes\":{\"$a\":[{\"newText\":\"// \",\"range\":$(at 1)},{\"newText\":\"// \",\"range\":$(at 0)}],\"$b\":[{\"newText\":\"// \",\"range\":$(at 0)}]}},\"title\":\"Comment\"},{\"data\":7,\"title\":\"Later\"},{\"arguments\":[\"x\",1],\"command\":\"fake.run\",\"title\":\"Run\"},{\"isPreferred\":true,\"title\":\"Best\"}]}" ;;
        *'"method":"codeAction/resolve"'*)
          a="file://${log%/*}/a.fake"
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"edit\":{\"documentChanges\":[{\"edits\":[{\"newText\":\"later \",\"range\":$(at 0)}],\"textDocument\":{\"uri\":\"$a\",\"version\":null}}]},\"title\":\"Later\"}}" ;;
        *'"method":"workspace/executeCommand"'*)
          a="file://${log%/*}/a.fake"
          send "{\"id\":900,\"jsonrpc\":\"2.0\",\"method\":\"workspace/applyEdit\",\"params\":{\"edit\":{\"changes\":{\"$a\":[{\"newText\":\"run \",\"range\":$(at 0)}]}}}}"
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":null}" ;;
        *'"method":"textDocument/completion"'*)
          case $(printf '%s' "$body" | sed -n 's/.*"line":\([0-9]*\).*/\1/p') in
            1) result="[{\"additionalTextEdits\":[{\"newText\":\"use foo;\\n\",\"range\":$(at 0)},{\"newText\":\"// end\",\"range\":$(at 2)}],\"detail\":\"fn foo()\",\"label\":\"foo\"},{\"label\":\"bar\"}]" ;;
            2) result="{\"isIncomplete\":false,\"items\":[{\"insertText\":\"ignored\",\"label\":\"self\",\"textEdit\":{\"newText\":\"a self\",\"range\":{\"end\":{\"character\":4,\"line\":2},\"start\":{\"character\":0,\"line\":2}}}}]}" ;;
            *) result="[]" ;;
          esac
          send "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":$result}" ;;
        *'"method":"textDocument/didOpen"'*|*'"method":"textDocument/didChange"'*)
          printf '%s' "$body" > "$dir/body"
          LC_ALL=C awk -v bodyf="$dir/body" -v textf="$dir/served" -f "$dir/apply.awk"
          uri=$(printf '%s' "$body" | sed -n 's/.*"uri":"\([^"]*\)".*/\1/p')
          version=$(printf '%s' "$body" | sed -n 's/.*"version":\([0-9]*\).*/\1/p')
          send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{\"diagnostics\":[{\"code\":7,\"message\":\"version $version\",\"range\":{\"end\":{\"character\":5,\"line\":0},\"start\":{\"character\":3,\"line\":0}},\"severity\":2}],\"uri\":\"$uri\",\"version\":$version}}" ;;
      esac ;;
  esac
done
"#;

    /// What the server does with a didOpen or a didChange: the text it is
    /// given, whole or as a range, in UTF-8 bytes, put in place in the file
    /// `served`. Read in the C locale, a string counts bytes.
    const APPLY: &str = r#"function hex(h,  i, n) {
  n = 0
  for (i = 1; i <= length(h); i++) n = n * 16 + index("0123456789abcdef", tolower(substr(h, i, 1))) - 1
  return n
}
function utf8(n) {
  if (n < 128) return sprintf("%c", n)
  if (n < 2048) return sprintf("%c%c", 192 + int(n / 64), 128 + n % 64)
  if (n < 65536) return sprintf("%c%c%c", 224 + int(n / 4096), 128 + int(n / 64) % 64, 128 + n % 64)
  return sprintf("%c%c%c%c", 240 + int(n / 262144), 128 + int(n / 4096) % 64, 128 + int(n / 64) % 64, 128 + n % 64)
}
function decode(s, i,   out, c, n, m) {
  out = ""
  while (i <= length(s)) {
    c = substr(s, i, 1)
    if (c == "\"") break
    if (c != "\\") { out = out c; i++; continue }
    c = substr(s, i + 1, 1)
    i += 2
    if (c == "n") out = out "\n"
    else if (c == "t") out = out "\t"
    else if (c == "r") out = out "\r"
    else if (c == "b") out = out "\b"
    else if (c == "f") out = out "\f"
    else if (c == "u") {
      n = hex(substr(s, i, 4))
      i += 4
      if (n >= 55296 && n < 56320 && substr(s, i, 2) == "\\u") {
        m = hex(substr(s, i + 2, 4))
        i += 6
        n = 65536 + (n - 55296) * 1024 + (m - 56320)
      }
      out = out utf8(n)
    } else out = out c
  }
  return out
}
function num(s, key) {
  match(s, "\"" key "\":[0-9]+")
  return substr(s, RSTART + length(key) + 3, RLENGTH - length(key) - 3) + 0
}
function offset(t, line, column,   at, k) {
  at = 0
  for (; line > 0; line--) {
    k = index(substr(t, at + 1), "\n")
    if (k == 0) return length(t)
    at += k
  }
  return at + column
}
BEGIN {
  RS = "\001"
  if ((getline body < bodyf) <= 0) body = ""
  if ((getline text < textf) <= 0) text = ""
  new = decode(body, index(body, "\"text\":\"") + 8)
  if (match(body, /"range":[{]"end":[{][^}]*[}],"start":[{][^}]*[}][}]/)) {
    range = substr(body, RSTART, RLENGTH)
    end = substr(range, index(range, "\"end\""))
    start = substr(range, index(range, "\"start\""))
    from = offset(text, num(start, "line"), num(start, "character"))
    to = offset(text, num(end, "line"), num(end, "character"))
    new = substr(text, 1, from) new substr(text, to + 1)
  }
  printf "%s", new > textf
}
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
            std::fs::write(dir.join("apply.awk"), APPLY).expect("writing the server");
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
        editor_syncing(sandbox, encoding, "1", &[])
    }

    /// The editor of `editor`, with a server that wants changes as SYNC
    /// says, and the modules MORE loaded after the others.
    fn editor_syncing(
        sandbox: &Sandbox,
        encoding: &str,
        sync: &str,
        more: &[&str],
    ) -> (Ctx, Arc<Env<Ctx>>) {
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
        ]
        .into_iter()
        .chain(more.iter().copied())
        {
            run(source, &env, &ctx);
        }
        run(
            &format!(
                r#"(setq lsp-servers
                     (list (list 'fundamental-mode "fake"
                                 (list "sh" "{script}" "{log}" "{encoding}" "{sync}")
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

    /// Sends the server what the buffer NAME, visiting FILE, has changed,
    /// runs the host's ticks until the server has taken it in, checks that
    /// the text it rebuilt is the buffer's, and returns the didChange.
    fn server_has_the_text(
        sandbox: &Sandbox,
        name: &str,
        file: &str,
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
    ) -> String {
        run("(lsp--after-command)", env, ctx);
        let version = text(&run(
            &format!(r#"(format "%d" (buffer-get 'lsp-version "{name}"))"#),
            env,
            ctx,
        ));
        finding_says(file, &format!("version {version}"), env, ctx);
        let served = std::fs::read_to_string(sandbox.0.join("served")).unwrap_or_default();
        assert_eq!(
            served,
            contents(name, env, ctx),
            "the server's text at version {version}"
        );
        logged(
            sandbox,
            &[
                r#""method":"textDocument/didChange""#,
                &format!(r#""version":{version}}}"#),
            ],
            env,
            ctx,
        )
    }

    /// Evaluates SRC in the buffer NAME, and ends its undo group.
    fn edit(name: &str, src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(r#"(with-current-buffer "{name}" (lambda () {src} (undo-boundary)))"#),
            env,
            ctx,
        );
    }

    #[test]
    fn a_server_that_takes_stretches_is_sent_only_what_the_edits_changed() {
        let sandbox = Sandbox::new("stretches");
        let file = sandbox.file("a.fake", "uno\nè😀 due\ntre\n");
        let (ctx, env) = editor_syncing(&sandbox, "utf-8", "2", &[]);
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        // After the emoji, six bytes into its line.
        edit(&name, r#"(goto-char 6) (insert "X")"#, &env, &ctx);
        let change = server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        assert!(
            change.contains(r#""contentChanges":[{"range":{"end":{"character":6,"line":1},"start":{"character":6,"line":1}},"text":"X"}]"#),
            "{change}"
        );
        type_at_end(&name, "quattro\\ncinque", &env, &ctx);
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        // From inside the first line to inside the third.
        edit(&name, "(delete-region 2 13)", &env, &ctx);
        let change = server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        assert!(
            change.contains(r#"{"range":{"end":{"character":1,"line":2},"start":{"character":2,"line":0}},"text":""}"#),
            "{change}"
        );
        // Edits at both ends, and a word typed and deleted again, in one.
        edit(
            &name,
            r#"(goto-char 1) (insert "a\nb") (goto-char (point-max)) (delete-char -3)
               (insert "zz") (delete-char -2)"#,
            &env,
            &ctx,
        );
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        edit(&name, r#"(insert "zz") (delete-char -2)"#, &env, &ctx);
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        assert!(
            !sandbox
                .log()
                .lines()
                .any(|line| line.contains(r#""contentChanges":[{"text""#)),
            "the whole text was sent:\n{}",
            sandbox.log()
        );
    }

    #[test]
    fn the_range_of_a_stretch_counts_columns_in_the_unit_of_the_server() {
        let sandbox = Sandbox::new("stretch-unit");
        let file = sandbox.file("a.fake", "x😀\n😀y\n");
        let (ctx, env) = editor_syncing(&sandbox, "utf-16", "2", &[]);
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        // What is removed ends after an emoji, two UTF-16 units into its line.
        edit(&name, "(delete-region 1 4)", &env, &ctx);
        run("(lsp--after-command)", &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""contentChanges":[{"range":{"end":{"character":2,"line":1},"start":{"character":1,"line":0}},"text":""}]"#,
            ],
            &env,
            &ctx,
        );
    }

    #[test]
    fn an_undo_past_the_stretch_watched_sends_the_whole_text() {
        let sandbox = Sandbox::new("stretch-undo");
        let file = sandbox.file("a.fake", "one\ntwo\n");
        let (ctx, env) = editor_syncing(&sandbox, "utf-8", "2", &[]);
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        edit(
            &name,
            r#"(goto-char (point-max)) (insert "three\n")"#,
            &env,
            &ctx,
        );
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        edit(&name, "(undo)", &env, &ctx);
        let change = server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        assert!(
            change.contains(r#""contentChanges":[{"text":"one\ntwo\n"}]"#),
            "{change}"
        );
        // An undo of an edit made since the last change sent stays a stretch.
        edit(&name, r#"(goto-char 4) (insert " and")"#, &env, &ctx);
        edit(&name, r#"(insert " more")"#, &env, &ctx);
        edit(&name, "(undo)", &env, &ctx);
        let change = server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        assert!(change.contains(r#""range""#), "{change}");
        edit(&name, "(redo)", &env, &ctx);
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
    }

    #[test]
    fn a_file_read_again_is_sent_whole_and_the_edits_after_it_as_stretches() {
        let sandbox = Sandbox::new("stretch-revert");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor_syncing(&sandbox, "utf-8", "2", &[]);
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        type_at_end(&name, "two\\n", &env, &ctx);
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        sandbox.file("a.fake", "new\ntext\n");
        edit(&name, "(revert-buffer--reread)", &env, &ctx);
        let change = server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        assert!(
            change.contains(r#""contentChanges":[{"text":"new\ntext\n"}]"#),
            "{change}"
        );
        type_at_end(&name, "more\\n", &env, &ctx);
        let change = server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        assert!(change.contains(r#""range""#), "{change}");
    }

    #[test]
    fn a_vim_insert_session_in_the_buffer_reaches_the_server_and_dot_still_repeats_it() {
        let sandbox = Sandbox::new("stretch-vim");
        let file = sandbox.file("a.fake", "one\ntwo\n");
        let (ctx, env) = editor_syncing(
            &sandbox,
            "utf-8",
            "2",
            &[
                include_str!("../../lisp/base/common-keymaps.lisp"),
                include_str!("../../lisp/features/display-line-numbers.lisp"),
                include_str!("../../lisp/features/vim.lisp"),
            ],
        );
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        run("(vim-mode t)", &env, &ctx);
        let keys = |sequence: &str| {
            for c in sequence.chars() {
                ctx.handle_key_event(KeyEvent::new(KeyCode::Char(c)), &env);
            }
        };
        keys("Axy");
        // Halfway through the session, each key sent as it came.
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        keys("z");
        ctx.handle_key_event(KeyEvent::new(KeyCode::Esc), &env);
        keys("j.");
        assert_eq!(contents(&name, &env, &ctx), "onexyz\ntwoxyz\n");
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
        keys("u");
        assert_eq!(contents(&name, &env, &ctx), "onexyz\ntwo\n");
        server_has_the_text(&sandbox, &name, &file, &env, &ctx);
    }

    /// The whole number SRC, a Lisp form, gives.
    fn number(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> u64 {
        text(&run(&format!(r#"(format "%d" {src})"#), env, ctx))
            .parse()
            .expect("a whole number")
    }

    /// Where in the server's log the first message holding all of PARTS is.
    fn place_in_log(sandbox: &Sandbox, parts: &[&str]) -> usize {
        sandbox
            .log()
            .lines()
            .position(|line| parts.iter().all(|part| line.contains(part)))
            .unwrap_or_else(|| panic!("no message with {parts:?} in:\n{}", sandbox.log()))
    }

    #[test]
    fn with_a_delay_the_changes_wait_until_the_buffer_has_been_quiet_that_long() {
        let sandbox = Sandbox::new("delay");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        run("(setq lsp-send-delay 0.5)", &env, &ctx);
        type_at_end(&name, "two\\n", &env, &ctx);
        let first = number("lsp--now", &env, &ctx);
        // Another change a quarter of a second later starts the wait again.
        tick_until(&format!("(>= lsp--now {})", first + 250), &env, &ctx);
        type_at_end(&name, "three\\n", &env, &ctx);
        tick_until(&format!("(>= lsp--now {})", first + 500), &env, &ctx);
        let version = format!(r#"(buffer-get 'lsp-version "{name}")"#);
        assert_eq!(number(&version, &env, &ctx), 1);
        let change = logged(
            &sandbox,
            &[r#""method":"textDocument/didChange""#],
            &env,
            &ctx,
        );
        assert!(
            change.contains(r#""contentChanges":[{"text":"one\ntwo\nthree\n"}]"#)
                && change.contains(r#""version":2"#),
            "{change}"
        );
        assert!(number("lsp--now", &env, &ctx) >= first + 750);
    }

    #[test]
    fn with_no_delay_the_changes_wait_for_a_question_or_a_save() {
        let sandbox = Sandbox::new("no-delay");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        run("(setq lsp-send-delay nil)", &env, &ctx);
        type_at_end(&name, "two\\n", &env, &ctx);
        for _ in 0..60 {
            step(&env, &ctx);
        }
        let version = format!(r#"(buffer-get 'lsp-version "{name}")"#);
        assert_eq!(number(&version, &env, &ctx), 1);
        run("(goto-char 1) (lsp-find-definition)", &env, &ctx);
        logged(
            &sandbox,
            &[r#""method":"textDocument/definition""#],
            &env,
            &ctx,
        );
        assert!(
            place_in_log(
                &sandbox,
                &[r#""method":"textDocument/didChange""#, r#""version":2"#]
            ) < place_in_log(&sandbox, &[r#""method":"textDocument/definition""#])
        );
        type_at_end(&name, "three\\n", &env, &ctx);
        run("(save-buffer)", &env, &ctx);
        logged(
            &sandbox,
            &[r#""method":"textDocument/didSave""#],
            &env,
            &ctx,
        );
        assert!(
            place_in_log(
                &sandbox,
                &[r#""method":"textDocument/didChange""#, r#""version":3"#]
            ) < place_in_log(&sandbox, &[r#""method":"textDocument/didSave""#])
        );
    }

    #[test]
    fn findings_that_come_while_changes_wait_are_dropped() {
        let sandbox = Sandbox::new("waiting-findings");
        let file = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        run("(setq lsp-send-delay nil)", &env, &ctx);
        type_at_end(&name, "two\\n", &env, &ctx);
        // About the version the server has, but not the text the buffer has.
        let findings = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"file://{file}","version":1,"diagnostics":[]}}}}"#
        );
        run(
            &format!(
                r#"(lsp--deliver {} 'output {:?})"#,
                server(&sandbox),
                findings
            ),
            &env,
            &ctx,
        );
        finding_says(&file, "version 1", &env, &ctx);
    }

    #[test]
    fn every_request_leaves_through_the_one_function_that_sends_the_changes_first() {
        let source = include_str!("../../lisp/features/lsp.lisp");
        // A request is the one message with both an id and a method.
        let mut senders = Vec::new();
        for (at, _) in source.match_indices(r#"(make-map "jsonrpc""#) {
            let mut depth = 0;
            let end = source[at..]
                .char_indices()
                .find(|&(_, c)| {
                    depth += match c {
                        '(' => 1,
                        ')' => -1,
                        _ => 0,
                    };
                    depth == 0
                })
                .map(|(i, _)| at + i)
                .expect("a closed form");
            let message = &source[at..end];
            if message.contains(r#""id""#) && message.contains(r#""method""#) {
                let defun = source[..at].rfind("(defun ").expect("inside a function");
                let name = source[defun + 7..].split_whitespace().next().unwrap();
                senders.push((name, &source[defun..at]));
            }
        }
        assert_eq!(senders.len(), 1, "{senders:?}");
        let (name, before) = senders[0];
        assert_eq!(name, "lsp--request");
        assert!(before.contains("(lsp--flush server)"), "{before}");
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

    /// The file the server's places are in.
    const TARGET: &str = "zero\n  one\n\u{1F600}x two\n";

    /// The character at point in the current buffer, once it visits FILE.
    fn at_point_in(file: &str, wanted: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        tick_until(
            &format!(
                r#"(and (equal (buffer-file-name) "{file}")
                        (equal (buffer-substring (point) (+ (point) 1)) "{wanted}"))"#
            ),
            env,
            ctx,
        );
    }

    #[test]
    fn a_definition_in_one_place_is_gone_to_and_lsp_go_back_returns() {
        let sandbox = Sandbox::new("definition");
        let file = sandbox.file("a.fake", "\u{1F600}ab\nsecond\n");
        let target = sandbox.file("target.txt", TARGET);
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        run("(goto-char (point-min)) (forward-char 2)", &env, &ctx);
        let here = run("(point)", &env, &ctx);
        run("(lsp-find-definition)", &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""method":"textDocument/definition""#,
                r#""position":{"character":2,"line":0}"#,
                &format!(r#""uri":"file://{file}""#),
            ],
            &env,
            &ctx,
        );
        at_point_in(&target, "o", &env, &ctx);
        run("(lsp-go-back)", &env, &ctx);
        assert_eq!(text(&run("(current-buffer)", &env, &ctx)), name);
        assert_eq!(run("(point)", &env, &ctx), here);
    }

    #[test]
    fn a_definition_is_asked_and_gone_to_in_the_columns_of_the_server() {
        let sandbox = Sandbox::new("definition-utf16");
        let file = sandbox.file("a.fake", "first\n\u{1F600}ab\n");
        let target = sandbox.file("target.txt", TARGET);
        let (ctx, env) = editor(&sandbox, "utf-16");
        open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        run("(goto-line 2) (forward-char 2)", &env, &ctx);
        run("(lsp-find-definition)", &env, &ctx);
        logged(
            &sandbox,
            &[r#""position":{"character":3,"line":1}"#],
            &env,
            &ctx,
        );
        // The LocationLink's selection, two UTF-16 units into its line: the
        // character after the emoji.
        at_point_in(&target, "x", &env, &ctx);
    }

    #[test]
    fn references_in_more_than_one_place_are_listed_for_next_error() {
        let sandbox = Sandbox::new("references");
        let file = sandbox.file("a.fake", "abc\n");
        let target = sandbox.file("target.txt", TARGET);
        let (ctx, env) = editor(&sandbox, "utf-32");
        open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        run("(lsp-find-references)", &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""method":"textDocument/references""#,
                r#""context":{"includeDeclaration":true}"#,
            ],
            &env,
            &ctx,
        );
        tick_until(
            r#"(equal (results-count "*lsp locations*") 2)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            text(&run(
                r#"(with-current-buffer "*lsp locations*" (lambda () (buffer-string)))"#,
                &env,
                &ctx
            )),
            "--- 2 references ---\ntarget.txt:1:2\ntarget.txt:3:4\n"
        );
        run("(next-error)", &env, &ctx);
        at_point_in(&target, "e", &env, &ctx);
        run("(next-error)", &env, &ctx);
        at_point_in(&target, "t", &env, &ctx);
    }

    #[test]
    fn no_definition_moves_nothing_and_leaves_nothing_to_go_back_to() {
        let sandbox = Sandbox::new("no-definition");
        let file = sandbox.file("a.fake", "a\nb\nc\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        run("(goto-line 3)", &env, &ctx);
        run("(lsp-find-definition)", &env, &ctx);
        logged(&sandbox, &[r#""position":{"character":0,"line":2}"#], &env, &ctx);
        tick_until(
            &format!("(null (get {} 'requests))", server(&sandbox)),
            &env,
            &ctx,
        );
        assert_eq!(text(&run("(current-buffer)", &env, &ctx)), name);
        assert!(!run("lsp--places", &env, &ctx).is_truthy());
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

    /// The text of the buffer NAME.
    fn contents(name: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text(&run(
            &format!(r#"(with-current-buffer "{name}" (lambda () (buffer-string)))"#),
            env,
            ctx,
        ))
    }

    /// Asks for the code actions at point, and answers the prompt with
    /// TITLE once it is open.
    fn take_action(title: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run("(lsp-code-action)", env, ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ctx.minibuffer_is_open() {
            assert!(Instant::now() < deadline, "no prompt within ten seconds");
            step(env, ctx);
        }
        for c in title.chars() {
            run(&format!(r#"(self-insert "{c}")"#), env, ctx);
        }
        run("(minibuffer-confirm)", env, ctx);
    }

    /// Runs the host's ticks until the buffer NAME holds WANTED.
    fn holds(name: &str, wanted: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        tick_until(
            &format!(
                r#"(equal (with-current-buffer "{name}" (lambda () (buffer-string))) {wanted:?})"#
            ),
            env,
            ctx,
        );
    }

    #[test]
    fn a_code_action_is_asked_with_the_diagnostics_that_touch_point() {
        let sandbox = Sandbox::new("action-context");
        let file = sandbox.file("a.fake", "one two\nthree\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        finding_says(&file, "version 1", &env, &ctx);
        run("(goto-char (point-min)) (forward-char 4)", &env, &ctx);
        run("(lsp-code-action)", &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""method":"textDocument/codeAction""#,
                r#""diagnostics":[{"code":7,"message":"version 1","#,
                r#""range":{"end":{"character":4,"line":0},"start":{"character":4,"line":0}}"#,
            ],
            &env,
            &ctx,
        );
        let touching = |at: usize| {
            run(
                &format!("(length (lsp--touching {} (cons {at} {at})))", server(&sandbox)),
                &env,
                &ctx,
            )
        };
        assert_eq!(touching(5), LispExp::number(1.0));
        assert_eq!(touching(10), LispExp::number(0.0));
    }

    #[test]
    fn the_actions_offered_put_the_preferred_first_and_leave_out_the_disabled() {
        let sandbox = Sandbox::new("action-list");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let answer = r#"[{"title":"Off","disabled":{"reason":"no"}},{"title":"Fix"},{"title":"Best","isPreferred":true},{"title":"Fix"}]"#;
        let titles = run(
            &format!("(mapcar 'car (lsp--actions (json-parse-string {answer:?})))"),
            &env,
            &ctx,
        );
        assert_eq!(
            titles,
            run(r#"(list "Best" "Fix" "Fix <2>")"#, &env, &ctx)
        );
    }

    #[test]
    fn a_code_action_is_undone_with_one_undo_in_each_buffer_it_changes() {
        let sandbox = Sandbox::new("action-undo");
        let a = sandbox.file("a.fake", "one\ntwo\n");
        sandbox.file("b.fake", "three\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&a, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        type_at_end(&name, "x", &env, &ctx);
        run("(goto-line 2)", &env, &ctx);
        take_action("Comment", &env, &ctx);
        holds(&name, "// one\n// two\nx", &env, &ctx);
        // b.fake was not open: it is opened for the change, behind a.fake.
        holds("b.fake", "// three\n", &env, &ctx);
        assert_eq!(text(&run("(current-buffer)", &env, &ctx)), name);
        assert_eq!(
            text(&run("(buffer-substring (point) (+ (point) 3))", &env, &ctx)),
            "two"
        );
        for (buffer, before) in [(name.as_str(), "one\ntwo\nx"), ("b.fake", "three\n")] {
            run(
                &format!(r#"(with-current-buffer "{buffer}" (lambda () (undo)))"#),
                &env,
                &ctx,
            );
            assert_eq!(contents(buffer, &env, &ctx), before);
        }
    }

    #[test]
    fn an_action_without_its_changes_is_resolved_and_a_command_runs_on_the_server() {
        let sandbox = Sandbox::new("action-resolve");
        let a = sandbox.file("a.fake", "one\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&a, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        take_action("Later", &env, &ctx);
        logged(
            &sandbox,
            &[r#""method":"codeAction/resolve""#, r#""data":7"#],
            &env,
            &ctx,
        );
        holds(&name, "later one\n", &env, &ctx);
        take_action("Run", &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""method":"workspace/executeCommand""#,
                r#""arguments":["x",1],"command":"fake.run""#,
            ],
            &env,
            &ctx,
        );
        holds(&name, "run later one\n", &env, &ctx);
        logged(
            &sandbox,
            &[r#""id":900"#, r#""result":{"applied":true}"#],
            &env,
            &ctx,
        );
    }

    #[test]
    fn an_edit_for_another_version_or_with_changes_that_overlap_is_refused_whole() {
        let sandbox = Sandbox::new("action-refused");
        let a = sandbox.file("a.fake", "one\n");
        let b = sandbox.file("b.fake", "two\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&a, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        let at = |line: u32, from: u32, to: u32| {
            format!(r#"{{"end":{{"character":{to},"line":{line}}},"start":{{"character":{from},"line":{line}}}}}"#)
        };
        let old_version = format!(
            r#"{{"documentChanges":[{{"edits":[{{"newText":"x","range":{}}}],"textDocument":{{"uri":"file://{b}","version":null}}}},{{"edits":[{{"newText":"x","range":{}}}],"textDocument":{{"uri":"file://{a}","version":99}}}}]}}"#,
            at(0, 0, 0),
            at(0, 0, 0)
        );
        let overlapping = format!(
            r#"{{"changes":{{"file://{a}":[{{"newText":"x","range":{}}},{{"newText":"y","range":{}}}]}}}}"#,
            at(0, 0, 2),
            at(0, 1, 3)
        );
        for (edit, why) in [
            (old_version, "has changed since"),
            (overlapping, "overlap"),
        ] {
            let said = text(&run(
                &format!(
                    "(lsp--apply-edit {} (json-parse-string {edit:?}))",
                    server(&sandbox)
                ),
                &env,
                &ctx,
            ));
            assert!(said.contains(why), "{said}");
            assert_eq!(contents(&name, &env, &ctx), "one\n");
            assert_eq!(contents("b.fake", &env, &ctx), "two\n");
        }
    }

    #[test]
    fn a_completion_left_alone_is_put_in_place_with_the_changes_that_go_with_it() {
        let sandbox = Sandbox::new("complete");
        let file = sandbox.file("a.fake", "x\nfo\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        run("(goto-line 2) (forward-char 2) (lsp-complete)", &env, &ctx);
        logged(
            &sandbox,
            &[
                r#""method":"textDocument/completion""#,
                r#""position":{"character":2,"line":1}"#,
            ],
            &env,
            &ctx,
        );
        // "bar" does not go on from "fo"; the change after the completion
        // has moved with it.
        holds(&name, "use foo;\nx\nfoo\n// end", &env, &ctx);
        assert_eq!(
            text(&run("(buffer-substring (point) (+ (point) 1))", &env, &ctx)),
            "\n"
        );
    }

    #[test]
    fn a_completion_takes_the_place_of_the_range_the_server_gives() {
        let sandbox = Sandbox::new("complete-range");
        let file = sandbox.file("a.fake", "x\n\na se\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        run("(goto-line 3) (forward-char 4) (lsp-complete)", &env, &ctx);
        holds(&name, "x\n\na self\n", &env, &ctx);
    }

    #[test]
    fn a_completion_answered_after_point_has_moved_is_not_offered() {
        let sandbox = Sandbox::new("complete-moved");
        let file = sandbox.file("a.fake", "x\nfo\n");
        let (ctx, env) = editor(&sandbox, "utf-32");
        let name = open(&file, &env, &ctx);
        ready(&sandbox, &env, &ctx);
        // The answer is read between two commands: point moves first.
        run("(goto-line 2) (forward-char 2) (lsp-complete) (backward-char 1)", &env, &ctx);
        logged(&sandbox, &[r#""method":"textDocument/completion""#], &env, &ctx);
        tick_until(
            &format!("(null (get {} 'requests))", server(&sandbox)),
            &env,
            &ctx,
        );
        assert_eq!(contents(&name, &env, &ctx), "x\nfo\n");
    }
}
