//! The shipped language modes as evaluated Lisp, including their real grammars.
use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
use crate::editor::{EditorState, create_global_env};
use crate::modes::{Grammar, SyntaxState, highlight_line};
use risp::{Env, LispExp, Parser, eval};
use std::sync::Arc;

type Ctx = EditorState<GapBuffer>;

fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
    let form = Parser::new(&format!("(progn {src})"))
        .next()
        .expect("test must parse");
    eval(&form, env.clone(), ctx).unwrap_or_else(|why| panic!("{src}: {why:?}"))
}

fn editor() -> (Ctx, Arc<Env<Ctx>>) {
    let (ctx, env) = create_global_env::<GapBuffer>().expect("editor");
    env.set_variable("frame-width".into(), LispExp::number(80.0));
    env.set_variable("frame-height".into(), LispExp::number(24.0));
    for source in [
        include_str!("../../lisp/base/commands.lisp"),
        include_str!("../../lisp/base/debug.lisp"),
        include_str!("../../lisp/base/indent.lisp"),
        include_str!("../../lisp/languages/cc-mode.lisp"),
        include_str!("../../lisp/languages/python-mode.lisp"),
        include_str!("../../lisp/languages/toml-mode.lisp"),
        include_str!("../../lisp/languages/bash-mode.lisp"),
    ] {
        run(source, &env, &ctx);
    }
    assert!(
        !ctx.get_logs()
            .iter()
            .any(|s| s.contains("Invalid") || s.contains("invalid regex")),
        "{:?}",
        ctx.get_logs()
    );
    (ctx, env)
}

fn text(ctx: &Ctx, mode: &str, source: &str) {
    ctx.with_current_buffer_mut(|b| {
        b.current_mode = mode.into();
        b.text = GapBuffer::from(source);
        let (line, column) = b.text.cursor_1d_to_2d(source.chars().count());
        b.text.cursor_move(line, column);
    });
}

fn columns(mode: &str, cases: &[(&str, usize)]) {
    let (ctx, env) = editor();
    for (source, expected) in cases {
        text(&ctx, mode, source);
        run("(indent-line)", &env, &ctx);
        assert_eq!(
            run("(current-indentation)", &env, &ctx),
            LispExp::number(*expected as f64),
            "{mode}: {source:?}"
        );
    }
}

fn grammar(ctx: &Ctx, mode: &str) -> Grammar {
    ctx.modes(|m| m.get(mode).expect("registered mode").grammar.clone())
}

fn face_at(grammar: &Grammar, line: &str, at: usize, entering: &SyntaxState) -> Option<String> {
    highlight_line(grammar, line, entering)
        .0
        .iter()
        .find(|span| span.start <= at && span.end > at)
        .map(|span| span.face.name().to_string())
}

#[test]
fn filenames_select_the_specific_languages_and_do_not_claim_unrelated_files() {
    let (ctx, _) = editor();
    for (path, mode) in [
        ("src/main.c", "c-mode"),
        ("include/main.h", "c-mode"),
        ("src/main.cpp", "cpp-mode"),
        ("src/main.C", "cpp-mode"),
        ("include/main.hh", "cpp-mode"),
        ("module.cppm", "cpp-mode"),
        ("test.py", "python-mode"),
        ("types.pyi", "python-mode"),
        ("Cargo.toml", "toml-mode"),
        ("Cargo.lock", "toml-mode"),
        ("build.sh", "bash-mode"),
        ("build.bash", "bash-mode"),
        ("/home/user/.bashrc", "bash-mode"),
        (r"C:\Users\user\.bash_profile", "bash-mode"),
    ] {
        assert_eq!(ctx.auto_mode_for(path).as_deref(), Some(mode), "{path}");
    }
    for path in [
        "test.py.bak",
        "notbashrc",
        "src/main.cpp.old",
        "Cargo.lock.bak",
    ] {
        assert_eq!(ctx.auto_mode_for(path), None, "{path}");
    }
}

#[test]
fn completion_vocabulary_matches_each_language() {
    let (ctx, env) = editor();
    for (mode, word) in [
        ("python-mode", "async"),
        ("python-mode", "print"),
        ("toml-mode", "false"),
        ("bash-mode", "esac"),
        ("bash-mode", "declare"),
        ("c-mode", "_Generic"),
        ("cpp-mode", "co_await"),
        ("cpp-mode", "static_cast"),
    ] {
        assert!(
            run(
                &format!("(member \"{word}\" (get '{mode} 'keywords))"),
                &env,
                &ctx
            )
            .is_truthy(),
            "{mode}: {word}"
        );
    }
    assert!(run("(member \"class\" (get 'c-mode 'keywords))", &env, &ctx).is_nil());
    assert!(
        run(
            "(member \"_Generic\" (get 'cpp-mode 'keywords))",
            &env,
            &ctx
        )
        .is_nil()
    );
}

#[test]
fn python_indents_blocks_clauses_and_explicit_continuations() {
    columns(
        "python-mode",
        &[
            ("if ready:\nwork()", 4),
            ("if ready: # not a string\n", 4),
            ("def f():\n    if ready:\nwork()", 8),
            ("if ready:\n    work()\nelse:", 0),
            ("if a:\n    if b:\n        work()\n    elif c:", 4),
            ("try:\n    work()\nexcept ValueError:", 0),
            ("try:\n    work()\nexcept:\n    recover()\nfinally:", 0),
            ("match value:\ncase 1:", 4),
            ("match value:\n    case 1:\n        work()\ncase 2:", 4),
            ("def f():\n    return 42\n", 0),
            ("def f():\n    work()\nnext_statement()", 0),
            ("text = 'not a block:'\n", 0),
            ("total = one + \\\n", 4),
            ("total = one + \\\n    two + \\\n", 4),
        ],
    );
}

#[test]
fn python_continuations_follow_delimiters_and_preserve_multiline_strings() {
    columns(
        "python-mode",
        &[
            ("values = [\nitem,", 4),
            ("call(first,\nsecond)", 5),
            ("values = [\n    item,\n]", 0),
            ("if (ready\n    and available):\nwork()", 4),
            ("result = call(\n    first)\n", 0),
            ("if ready:\n    text = '''hello\n  preserve this", 2),
            ("if ready:\n    text = \"\"\"hello\n   preserve this", 3),
        ],
    );
}

#[test]
fn toml_indents_containers_but_not_table_keys() {
    columns(
        "toml-mode",
        &[
            ("[package]\n    name = 'editor'", 0),
            ("[[bin]]\n    name = 'editor'", 0),
            ("values = [\nitem,", 2),
            ("values = [one,\ntwo,", 10),
            ("values = [\n  [\nvalue", 4),
            ("values = [\n  one,\n]", 0),
            ("settings = {\nkey = true", 2),
            ("key = '''hello\n   literal text", 3),
            ("key = \"\"\"hello\n  literal text", 2),
            ("key = '# [ is text'\n  other = 1", 0),
        ],
    );
}

#[test]
fn bash_indents_nested_control_flow_without_treating_arguments_as_keywords() {
    columns(
        "bash-mode",
        &[
            ("if test -f file; then\necho yes", 4),
            ("if true\nthen", 0),
            ("if true; then\n  echo yes\nelse", 0),
            ("if true; then\n  echo yes\nfi", 0),
            ("for f in *; do\necho \"$f\"", 4),
            ("for f in *\ndo", 0),
            ("while true; do\nif true; then\necho ok", 8),
            ("while true; do\nif true; then\ntrue\nfi\ndone", 0),
            ("echo if then done\nnext", 0),
            ("echo ${#items[@]}\nnext", 0),
            (
                "if true; then\nvalue=$(if false; then echo no; fi)\nnext",
                4,
            ),
            ("printf '%s' '# if then' # else\nnext", 0),
            ("if true; then echo yes; fi\nnext", 0),
            ("f() {\necho ok", 4),
            ("f() {\n  echo ok\n}", 0),
            ("printf foo |\ncat", 4),
            ("echo foo \\\n  bar \\\nbaz", 4),
        ],
    );
}

#[test]
fn bash_case_patterns_and_bodies_have_distinct_columns() {
    columns(
        "bash-mode",
        &[
            ("case $x in\none)", 4),
            ("case $x in\none)\necho yes", 8),
            ("case $x in\none)\necho yes\n;;\ntwo)", 4),
            ("case $x in\none) echo yes ;;\nesac", 0),
            ("if true; then\ncase $x in\none)\necho yes", 12),
        ],
    );
}

#[test]
fn bash_leaves_heredoc_contents_and_multiline_quotes_untouched() {
    columns(
        "bash-mode",
        &[
            ("if true; then\ncat <<EOF\n  if text ' done", 2),
            (
                "if true; then\ncat <<'EOF'\n  if text ' done\nEOF\necho after",
                4,
            ),
            ("cat <<-EOF\n\tfi\n\tEOF\necho after", 0),
            (
                "cat <<ONE <<'TWO'\nif ignored\nONE\ndone ignored\nTWO\necho after",
                0,
            ),
            ("cat <<<EOF\necho after", 0),
            ("if true; then\nprintf '%s' \"hello\n  preserve this", 2),
        ],
    );
}

#[test]
fn bash_cache_is_invalidated_by_edits() {
    let (ctx, env) = editor();
    text(&ctx, "bash-mode", "if true; then\nbody");
    run("(indent-line)", &env, &ctx);
    run(
        "(goto-char 0) (delete-region 0 13) (insert \"echo changed\") (goto-char (point-max)) (indent-line)",
        &env,
        &ctx,
    );
    assert_eq!(
        run("(current-indentation)", &env, &ctx),
        LispExp::number(0.0)
    );
}

#[test]
fn c_and_cpp_indent_braces_parameters_cases_and_labels() {
    for mode in ["c-mode", "cpp-mode"] {
        columns(
            mode,
            &[
                ("int main() {\nreturn 0;", 4),
                ("int main() {\n    if (ok) {\ncall();", 8),
                ("int main() {\n    call();\n}", 0),
                ("int main() {\n    if (ok)\ncall();", 8),
                ("int main() {\n    if (ok)\n{", 4),
                ("int main() {\n    if (ok &&\n        more)\ncall();", 8),
                ("int main() {\n    if (ok)\n        call();\n    next();", 4),
                ("int main() {\n    if (ok)\n        call();\nelse", 4),
                ("call(first,\nsecond);", 5),
                ("int main() {\n    call(\nargument);", 8),
                ("int main() {\n    call(\n        argument\n);", 4),
                ("switch (value) {\ncase 1:", 4),
                ("switch (value) {\n    case 1:\nwork();", 8),
                ("switch (value)\n{\n    case 1:\nwork();", 8),
                (
                    "switch (value +\n        offset)\n{\n    case 1:\nwork();",
                    8,
                ),
                (
                    "switch (value) {\n    case 1:\n        work();\ndefault:",
                    4,
                ),
                ("int main() {\n  label:", 0),
                ("int main() {\n    #define VALUE 1", 0),
                ("const char *s = \"{ //\";\n    next;", 0),
                ("char c = '}';\nnext;", 0),
                ("/* comment\n* continued", 1),
            ],
        );
    }
    columns(
        "cpp-mode",
        &[
            ("class Widget {\npublic:", 0),
            ("class Widget {\npublic:\nvoid f();", 4),
        ],
    );
}

#[test]
fn region_indentation_is_repeatable_for_each_language() {
    let (ctx, env) = editor();
    for (mode, source, expected) in [
        ("python-mode", "if yes:\ncall()\n", "if yes:\n    call()\n"),
        ("toml-mode", "items = [\n1,\n]\n", "items = [\n  1,\n]\n"),
        (
            "bash-mode",
            "if true; then\necho ok\nfi\n",
            "if true; then\n    echo ok\nfi\n",
        ),
        (
            "c-mode",
            "int main() {\nreturn 0;\n}\n",
            "int main() {\n    return 0;\n}\n",
        ),
        (
            "cpp-mode",
            "class C {\npublic:\nvoid f();\n};\n",
            "class C {\npublic:\n    void f();\n};\n",
        ),
    ] {
        text(&ctx, mode, source);
        run(
            "(goto-char 0) (set-mark) (goto-char (point-max)) (indent-region)",
            &env,
            &ctx,
        );
        assert_eq!(
            ctx.with_current_buffer(|b| b.text.to_string()),
            expected,
            "{mode}"
        );
        run("(indent-region)", &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|b| b.text.to_string()),
            expected,
            "second pass: {mode}"
        );
    }
}

#[test]
fn python_grammar_faces_keywords_calls_numbers_and_comments() {
    let (ctx, _) = editor();
    let g = grammar(&ctx, "python-mode");
    for (line, needle, expected) in [
        ("async def f():", "async", "keyword"),
        ("print(True)", "print", "builtin"),
        ("print(True)", "True", "builtin"),
        ("int(value)", "int", "type"),
        ("class Widget:", "Widget", "type"),
        ("@package.decorator", "@", "attribute"),
        ("value = 0xFF", "0x", "builtin"),
        ("value = 1.5e-3j", "1.5", "builtin"),
        ("# ' not a string", "#", "comment"),
        ("f'# not a comment'", "#", "string"),
    ] {
        assert_eq!(
            face_at(
                &g,
                line,
                line[..line.find(needle).unwrap()].chars().count(),
                &vec![]
            )
            .as_deref(),
            Some(expected),
            "{line}"
        );
    }
    let state = highlight_line(&g, "text = r'''hello", &vec![]).1;
    assert_eq!(
        face_at(&g, "if True: # still string", 0, &state).as_deref(),
        Some("string")
    );
    assert!(highlight_line(&g, "end'''", &state).1.is_empty());
}

#[test]
fn toml_grammar_faces_tables_keys_dates_and_quoted_hashes() {
    let (ctx, _) = editor();
    let g = grammar(&ctx, "toml-mode");
    for (line, at, expected) in [
        ("[[bin]]", 0, "type"),
        ("package.name = 'hi'", 0, "variable"),
        ("date = 2026-10-19T10:00:00Z", 7, "builtin"),
        ("enabled = false", 10, "builtin"),
        ("value = 1.5e-3", 9, "builtin"),
        ("value = '# text'", 9, "string"),
        ("# comment", 0, "comment"),
    ] {
        assert_eq!(
            face_at(&g, line, at, &vec![]).as_deref(),
            Some(expected),
            "{line}"
        );
    }
    let state = highlight_line(&g, "text = '''literal", &vec![]).1;
    assert_eq!(
        face_at(&g, "[not_a_table]", 0, &state).as_deref(),
        Some("string")
    );
    assert!(highlight_line(&g, "end'''", &state).1.is_empty());
}

#[test]
fn bash_grammar_distinguishes_hashes_in_words_parameters_and_comments() {
    let (ctx, _) = editor();
    let g = grammar(&ctx, "bash-mode");
    for (line, at, expected) in [
        ("if true; then", 0, Some("keyword")),
        ("echo hi", 0, Some("builtin")),
        ("value=$HOME", 6, Some("variable")),
        ("echo ${#items[@]}", 7, Some("variable")),
        ("echo foo#bar", 8, None),
        ("echo hi # comment", 8, Some("comment")),
        ("echo '# not a comment'", 6, Some("string")),
    ] {
        assert_eq!(
            face_at(&g, line, at, &vec![]).as_deref(),
            expected,
            "{line}"
        );
    }
}

#[test]
fn c_and_cpp_grammars_include_modern_keywords_and_literals() {
    let (ctx, _) = editor();
    for mode in ["c-mode", "cpp-mode"] {
        let g = grammar(&ctx, mode);
        for (line, at, expected) in [
            ("const int value;", 0, "keyword"),
            ("uint64_t value;", 0, "type"),
            ("#include <stdio.h>", 0, "preprocessor"),
            ("double x = 0x1.fp+3;", 11, "builtin"),
            ("float x = 1.0e-3f;", 10, "builtin"),
            ("char c = '\"'; int next;", 9, "string"),
            ("char c = '\"'; int next;", 14, "type"),
            ("// \" not a string", 3, "comment"),
        ] {
            assert_eq!(
                face_at(&g, line, at, &vec![]).as_deref(),
                Some(expected),
                "{mode}: {line}"
            );
        }
    }
    let cpp = grammar(&ctx, "cpp-mode");
    assert_eq!(
        face_at(&grammar(&ctx, "c-mode"), "_Atomic int value;", 0, &vec![]).as_deref(),
        Some("keyword")
    );
    for (line, at) in [
        ("static_cast<int>(n)", 0),
        ("co_await task;", 0),
        ("a and b", 2),
    ] {
        assert_eq!(face_at(&cpp, line, at, &vec![]).as_deref(), Some("keyword"));
    }
    assert_eq!(
        face_at(&cpp, "[[nodiscard]] int f();", 0, &vec![]).as_deref(),
        Some("attribute")
    );
}

#[test]
fn cpp_raw_string_delimiters_configure_both_highlighting_and_scanning() {
    let (ctx, env) = editor();
    run("(cpp-add-raw-string-delimiter \"custom\")", &env, &ctx);
    for source in [
        "auto s = R\"(\" } //\n    literal)\";",
        "auto s = u8R\"json(\" } //\n    literal)json\";",
        "auto s = R\"custom(\" } //\n    literal)custom\";",
    ] {
        text(&ctx, "cpp-mode", source);
        let g = grammar(&ctx, "cpp-mode");
        let mut state = vec![];
        for line in source.lines() {
            state = highlight_line(&g, line, &state).1;
        }
        assert!(state.is_empty(), "{source}");
        let offset = source.find("literal").unwrap();
        assert!(
            run(&format!("(nth 2 (syntax-ppss {offset}))"), &env, &ctx).is_truthy(),
            "{source}"
        );
        run("(indent-line)", &env, &ctx);
        assert_eq!(ctx.with_current_buffer(|b| b.text.to_string()), source);
        assert!(run("(nth 2 (syntax-ppss (point-max)))", &env, &ctx).is_nil());
    }
}

#[test]
fn escaped_triple_quotes_do_not_close_python_or_toml_basic_strings() {
    let (ctx, env) = editor();
    for mode in ["python-mode", "toml-mode"] {
        let source = "value = \"\"\"open \\\"\"\" still a string\n  { preserve }\n\"\"\"";
        text(&ctx, mode, source);
        let at = source.find("preserve").unwrap();
        assert!(
            run(&format!("(nth 2 (syntax-ppss {at}))"), &env, &ctx).is_truthy(),
            "{mode}"
        );
        assert_eq!(
            run(&format!("(nth 0 (syntax-ppss {at}))"), &env, &ctx),
            LispExp::number(0.0)
        );
        assert!(run("(nth 2 (syntax-ppss (point-max)))", &env, &ctx).is_nil());
        let g = grammar(&ctx, mode);
        let state = highlight_line(&g, source.lines().next().unwrap(), &vec![]).1;
        assert_eq!(
            face_at(&g, "  { preserve }", 4, &state).as_deref(),
            Some("string")
        );
    }
    // TOML literal strings and C++ raw strings keep the previous API's
    // default: a backslash immediately before the closer does not escape it.
    for (mode, source) in [
        ("toml-mode", "value = '''literal\\'''\nnext = 1"),
        ("cpp-mode", "auto s = R\"(literal\\)\";\nint next;"),
    ] {
        text(&ctx, mode, source);
        assert!(
            run("(nth 2 (syntax-ppss (point-max)))", &env, &ctx).is_nil(),
            "{source}"
        );
    }
}

#[test]
fn registered_bash_heredocs_colour_contents_until_the_exact_terminator() {
    let (ctx, env) = editor();
    run("(bash-add-heredoc-delimiter \"CUSTOM\")", &env, &ctx);
    let g = grammar(&ctx, "bash-mode");
    for opener in ["cat <<EOF", "cat <<'EOF'", "cat <<\"EOF\"", "cat <<CUSTOM"] {
        let state = highlight_line(&g, opener, &vec![]).1;
        assert!(!state.is_empty(), "{opener}");
        assert_eq!(
            face_at(&g, "if ' not shell code", 0, &state).as_deref(),
            Some("string")
        );
        assert!(!highlight_line(&g, "EOF suffix", &state).1.is_empty());
        let end = if opener.contains("CUSTOM") {
            "CUSTOM"
        } else {
            "EOF"
        };
        assert!(highlight_line(&g, end, &state).1.is_empty());
    }
    let tabbed = highlight_line(&g, "cat <<-EOF", &vec![]).1;
    assert!(highlight_line(&g, "\tEOF", &tabbed).1.is_empty());
    assert!(!highlight_line(&g, " EOF", &tabbed).1.is_empty());
}

#[test]
fn reloading_modes_restores_default_raw_string_and_heredoc_regions() {
    let (ctx, env) = editor();
    let cpp_count = grammar(&ctx, "cpp-mode").regions.len();
    let bash_count = grammar(&ctx, "bash-mode").regions.len();
    for _ in 0..2 {
        run(
            include_str!("../../lisp/languages/cc-mode.lisp"),
            &env,
            &ctx,
        );
        run(
            include_str!("../../lisp/languages/bash-mode.lisp"),
            &env,
            &ctx,
        );
        let cpp = grammar(&ctx, "cpp-mode");
        let bash = grammar(&ctx, "bash-mode");
        assert_eq!(cpp.regions.len(), cpp_count);
        assert_eq!(bash.regions.len(), bash_count);
        assert!(
            !highlight_line(&cpp, "auto s = R\"json(open", &vec![])
                .1
                .is_empty()
        );
        assert!(!highlight_line(&bash, "cat <<EOF", &vec![]).1.is_empty());
    }
}

#[test]
fn bash_cache_tracks_a_changed_indent_width_and_undo_of_a_prefix_edit() {
    let (ctx, env) = editor();
    text(&ctx, "bash-mode", "if true; then\nif true; then\nbody");
    run("(bash-indent-line)", &env, &ctx);
    run("(put 'bash-mode 'tab-width 2)", &env, &ctx);
    assert_eq!(run("(bash-indent-line)", &env, &ctx), LispExp::number(4.0));
    text(&ctx, "bash-mode", "if true; then\nbody");
    run(
        "(goto-char 0) (insert \"#\") (goto-char (point-max))",
        &env,
        &ctx,
    );
    assert_eq!(run("(bash-indent-line)", &env, &ctx), LispExp::number(0.0));
    run("(undo) (goto-char (point-max))", &env, &ctx);
    assert_eq!(run("(bash-indent-line)", &env, &ctx), LispExp::number(2.0));
}

#[test]
fn bash_region_indentation_preserves_literal_heredoc_text_and_tabs() {
    let (ctx, env) = editor();
    let source = "if true; then\ncat <<-EOF\n\tif this is text\n\tEOF\necho after\nfi\n";
    text(&ctx, "bash-mode", source);
    run(
        "(goto-char 0) (set-mark) (goto-char (point-max)) (indent-region)",
        &env,
        &ctx,
    );
    assert_eq!(
        ctx.with_current_buffer(|b| b.text.to_string()),
        "if true; then\n    cat <<-EOF\n\tif this is text\n\tEOF\n    echo after\nfi\n"
    );
}

#[test]
fn return_uses_the_registered_indenter_in_each_new_mode() {
    let (ctx, env) = editor();
    for (mode, source, suffix) in [
        ("python-mode", "if ready:", "\n    "),
        ("toml-mode", "values = [", "\n  "),
        ("bash-mode", "if true; then", "\n    "),
        ("c-mode", "int main() {", "\n    "),
        ("cpp-mode", "class Widget {", "\n    "),
    ] {
        text(&ctx, mode, source);
        run("(newline-and-indent)", &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|b| b.text.to_string()),
            format!("{source}{suffix}")
        );
    }
}
