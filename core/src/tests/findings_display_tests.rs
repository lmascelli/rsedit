//! The findings on the screen: each span drawn in the face of its severity
//! where `findings-mode` is on, following the text; a sign in the gutter; the
//! count in the status line; the message at point in an echo area with
//! nothing else to say; and the list `M-g n` walks, asked of the store again
//! before each step.
//!
//! Design: `doc/architettura-editor.typ, <segnalazioni-schermo>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::ui::{Face, FrameSnapshot, RenderableWindowView};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    /// A directory of this test's own, removed when it goes.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "rsedit-findings-display-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }

        fn path(&self) -> String {
            self.0.display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
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

    /// An editor with no module loaded.
    fn bare() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        (ctx, env)
    }

    /// An editor with the modules the findings need, and the one that draws
    /// them.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = bare();
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/findings.lisp"),
        ] {
            try_run(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// Opens FILE in the focused window.
    fn open(file: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(&format!("(find-file {file:?})"), env, ctx);
    }

    /// Returns the view showing BUFFER in FRAME.
    fn view<'a>(frame: &'a FrameSnapshot, buffer: &str) -> &'a RenderableWindowView {
        frame
            .views
            .iter()
            .find(|view| view.buffer_name == buffer)
            .unwrap_or_else(|| panic!("no window shows {buffer}"))
    }

    /// Returns (row, start, end, face) of what VIEW draws in one of the
    /// findings' faces.
    fn spans(view: &RenderableWindowView) -> Vec<(usize, usize, usize, Face)> {
        let faces = [
            Face::FINDING_ERROR,
            Face::FINDING_WARNING,
            Face::FINDING_INFO,
            Face::FINDING_HINT,
        ];
        view.highlights
            .iter()
            .filter(|h| faces.contains(&h.face))
            .map(|h| (h.row, h.start_col, h.end_col, h.face))
            .collect()
    }

    /// Returns the name of the buffer visiting FILE.
    fn buffer_of(file: &str, ctx: &Ctx) -> String {
        ctx.buffer_visiting(file).expect("a buffer visits the file")
    }

    const TEXT: &str = "alpha\nbeta gamma\ndelta\n";

    // ----------------------------------------------------------------
    // The spans
    // ----------------------------------------------------------------

    #[test]
    fn a_finding_is_drawn_over_its_span_in_the_face_of_its_severity() {
        let sandbox = Sandbox::new("span");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?}
                   '((:start (2 . 5) :end (2 . 10) :severity warning :message \"w\")))"
            ),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(
            spans(view(&frame, &buffer_of(&file, &ctx))),
            vec![(1, 5, 10, Face::FINDING_WARNING)]
        );
    }

    #[test]
    fn findings_made_before_the_file_is_opened_are_drawn_once_it_is() {
        let sandbox = Sandbox::new("before");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (1 . 0) :end (1 . 5) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        open(&file, &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(
            spans(view(&frame, &buffer_of(&file, &ctx))),
            vec![(0, 0, 5, Face::FINDING_ERROR)]
        );
    }

    #[test]
    fn a_span_follows_the_text_when_a_line_is_added_above_it() {
        let sandbox = Sandbox::new("follow");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (3 . 0) :end (3 . 5) :severity info :message \"i\")))"
            ),
            &env,
            &ctx,
        );
        run("(goto-char 1) (insert \"new\\n\")", &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(
            spans(view(&frame, &buffer_of(&file, &ctx))),
            vec![(3, 0, 5, Face::FINDING_INFO)]
        );
    }

    #[test]
    fn a_finding_with_no_span_takes_the_cell_at_its_start() {
        let sandbox = Sandbox::new("point");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        // The second at the end of its line: the cell after the text.
        run(
            &format!(
                "(finding-set 'lint {file:?}
                   '((:start (2 . 2) :severity hint :message \"h\")
                     (:start (1 . 5) :severity hint :message \"end\")))"
            ),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, W, H);
        let mut drawn = spans(view(&frame, &buffer_of(&file, &ctx)));
        drawn.sort();
        assert_eq!(
            drawn,
            vec![(0, 5, 6, Face::FINDING_HINT), (1, 2, 3, Face::FINDING_HINT)]
        );
    }

    #[test]
    fn the_more_severe_of_two_findings_over_the_same_text_is_drawn_last() {
        let sandbox = Sandbox::new("order");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'a {file:?} '((:start (1 . 0) :end (1 . 5) :severity error :message \"e\")))
                 (finding-set 'b {file:?} '((:start (1 . 0) :end (1 . 5) :severity hint :message \"h\")))"
            ),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, W, H);
        let faces: Vec<Face> = spans(view(&frame, &buffer_of(&file, &ctx)))
            .into_iter()
            .map(|(_, _, _, face)| face)
            .collect();
        assert_eq!(faces, vec![Face::FINDING_HINT, Face::FINDING_ERROR]);
    }

    #[test]
    fn nothing_is_drawn_where_findings_mode_is_off() {
        let sandbox = Sandbox::new("off");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (1 . 0) :end (1 . 5) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        run("(findings-mode -1)", &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        let shown = view(&frame, &buffer_of(&file, &ctx));
        assert!(spans(shown).is_empty());
        assert_eq!(shown.gutter_width, 0);
    }

    #[test]
    fn an_editor_without_the_module_draws_no_finding() {
        let sandbox = Sandbox::new("bare");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = bare();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (1 . 0) :end (1 . 5) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        run("(goto-char 1)", &env, &ctx);
        ctx.set_echo_message("");
        let frame = ctx.snapshot(&env, W, H);
        assert!(spans(view(&frame, &buffer_of(&file, &ctx))).is_empty());
        assert_eq!(frame.echo_message, "");
    }

    // ----------------------------------------------------------------
    // The gutter
    // ----------------------------------------------------------------

    #[test]
    fn the_line_a_finding_starts_on_has_its_sign_in_the_gutter() {
        let sandbox = Sandbox::new("sign");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?}
                   '((:start (2 . 0) :end (3 . 2) :severity warning :message \"w\")
                     (:start (2 . 6) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, W, H);
        let shown = view(&frame, &buffer_of(&file, &ctx));
        assert_eq!(shown.gutter_width, 2, "the sign and a space");
        assert_eq!(shown.gutter[0].text, "  ");
        assert_eq!(shown.gutter[0].sign, None);
        // The more severe of the two on the line.
        assert_eq!(shown.gutter[1].text, "E ");
        assert_eq!(shown.gutter[1].sign, Some(Face::FINDING_ERROR_SIGN));
        // Where the span goes on, but does not start, no sign.
        assert_eq!(shown.gutter[2].sign, None);
    }

    #[test]
    fn the_signs_come_before_the_line_numbers() {
        let sandbox = Sandbox::new("numbers");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            include_str!("../../lisp/features/display-line-numbers.lisp"),
            &env,
            &ctx,
        );
        run("(minor-mode-on 'display-line-numbers-mode)", &env, &ctx);
        let numbered = view(&ctx.snapshot(&env, W, H), &buffer_of(&file, &ctx)).gutter_width;
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (1 . 0) :severity info :message \"i\")))"
            ),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, W, H);
        let shown = view(&frame, &buffer_of(&file, &ctx));
        assert_eq!(shown.gutter_width, numbered + 2);
        assert_eq!(
            shown.gutter[0].text,
            format!("I {:>w$} ", 1, w = numbered - 1)
        );
        assert_eq!(shown.gutter[0].sign, Some(Face::FINDING_INFO_SIGN));
        assert_eq!(shown.gutter[0].face, Face::LINE_NUMBER_CURRENT);
        assert!(shown.gutter[1].text.starts_with("  "));
    }

    #[test]
    fn a_wrapped_line_has_its_sign_on_its_first_row_only() {
        let sandbox = Sandbox::new("wrap");
        let long = "word ".repeat(30);
        let file = sandbox.file("a.txt", &format!("{long}\nshort\n"));
        let (ctx, env) = editor();
        run(
            include_str!("../../lisp/features/visual-line.lisp"),
            &env,
            &ctx,
        );
        open(&file, &env, &ctx);
        run("(minor-mode-on 'visual-line-mode)", &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?}
                   '((:start (1 . 100) :severity warning :message \"w\")
                     (:start (2 . 0) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, 40, H);
        let shown = view(&frame, &buffer_of(&file, &ctx));
        let signs: Vec<Option<Face>> = shown.gutter.iter().map(|cell| cell.sign).collect();
        // The long line takes the first rows; the short one follows them.
        let short = shown
            .lines
            .iter()
            .position(|line| line.starts_with("short"))
            .expect("the short line is shown");
        assert!(short > 1, "the long line wraps: {:?}", shown.lines);
        assert_eq!(signs[0], Some(Face::FINDING_WARNING_SIGN));
        assert!(signs[1..short].iter().all(Option::is_none), "{signs:?}");
        assert_eq!(signs[short], Some(Face::FINDING_ERROR_SIGN));
    }

    #[test]
    fn a_buffer_without_findings_has_no_column_for_signs() {
        let sandbox = Sandbox::new("nosign");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (1 . 0) :severity info :message \"i\")))"
            ),
            &env,
            &ctx,
        );
        run(&format!("(finding-clear 'lint {file:?})"), &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(view(&frame, &buffer_of(&file, &ctx)).gutter_width, 0);
    }

    #[test]
    fn a_window_too_narrow_to_spare_the_columns_has_no_signs() {
        let sandbox = Sandbox::new("narrow");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (1 . 0) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, 9, H);
        assert_eq!(view(&frame, &buffer_of(&file, &ctx)).gutter_width, 0);
    }

    // ----------------------------------------------------------------
    // The status line
    // ----------------------------------------------------------------

    #[test]
    fn the_status_line_counts_the_findings_by_severity() {
        let sandbox = Sandbox::new("count");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run("(setq mode-line-format \"[%!]\")", &env, &ctx);
        let quiet = ctx.snapshot(&env, W, H);
        assert_eq!(
            view(&quiet, &buffer_of(&file, &ctx)).mode_line.as_deref(),
            Some("[]")
        );
        run(
            &format!(
                "(finding-set 'a {file:?}
                   '((:start (1 . 0) :severity warning :message \"w\")
                     (:start (2 . 0) :severity error :message \"e\")))
                 (finding-set 'b {file:?}
                   '((:start (3 . 0) :severity warning :message \"w\")))"
            ),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(
            view(&frame, &buffer_of(&file, &ctx)).mode_line.as_deref(),
            Some("[ E1 W2]")
        );
    }

    #[test]
    fn the_status_line_shows_the_count_where_it_is_told_to_by_default() {
        let sandbox = Sandbox::new("default-format");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!("(finding-set 'a {file:?} '((:start (1 . 0) :severity hint :message \"h\")))"),
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, W, H);
        let line = view(&frame, &buffer_of(&file, &ctx))
            .mode_line
            .clone()
            .expect("a status line");
        assert!(line.contains(" H1  L1 "), "{line:?}");
    }

    // ----------------------------------------------------------------
    // The message at point
    // ----------------------------------------------------------------

    #[test]
    fn the_echo_area_says_the_message_of_the_finding_at_point() {
        let sandbox = Sandbox::new("echo");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?}
                   '((:start (2 . 5) :end (2 . 10) :severity warning :message \"unused\" :code \"W1\")
                     (:start (2 . 5) :end (2 . 7) :severity error :message \"bad\nsecond line\")))
                 (message \"\")"
            ),
            &env,
            &ctx,
        );
        run("(goto-char 1)", &env, &ctx);
        assert_eq!(ctx.snapshot(&env, W, H).echo_message, "");
        // On both: the error, and that there is one more.
        run("(goto-char 12)", &env, &ctx);
        assert_eq!(
            ctx.snapshot(&env, W, H).echo_message,
            "error: bad (lint) (and 1 more)"
        );
        run("(goto-char 14)", &env, &ctx);
        assert_eq!(
            ctx.snapshot(&env, W, H).echo_message,
            "warning: unused [W1] (lint)"
        );
        // The end is not on it.
        run("(goto-char 17)", &env, &ctx);
        assert_eq!(ctx.snapshot(&env, W, H).echo_message, "");
    }

    #[test]
    fn a_message_of_its_own_in_the_echo_area_wins_over_the_finding_at_point() {
        let sandbox = Sandbox::new("echo-wins");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (1 . 0) :end (1 . 5) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        run("(goto-char 1) (message \"hello\")", &env, &ctx);
        assert_eq!(ctx.snapshot(&env, W, H).echo_message, "hello");
    }

    // ----------------------------------------------------------------
    // The list
    // ----------------------------------------------------------------

    #[test]
    fn the_list_shows_a_line_for_each_finding_about_the_file() {
        let sandbox = Sandbox::new("list");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?}
                   '((:start (2 . 5) :severity warning :message \"unused\" :code \"W1\")
                     (:start (3 . 0) :severity error :message \"bad\")))"
            ),
            &env,
            &ctx,
        );
        run("(findings-list)", &env, &ctx);
        let text = ctx
            .with_buffer("*Findings*", |buf| buf.text.to_string())
            .expect("the list");
        assert_eq!(
            text,
            format!(
                "--- 2 findings about {file} ---\n\
                 a.txt:2:6: warning: unused [W1] (lint)\n\
                 a.txt:3:1: error: bad (lint)\n"
            )
        );
        assert_eq!(
            run("(results-buffer)", &env, &ctx),
            LispExp::string("*Findings*".into())
        );
    }

    #[test]
    fn next_error_walks_the_findings_in_the_list() {
        let sandbox = Sandbox::new("walk");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        let source = buffer_of(&file, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?}
                   '((:start (2 . 5) :severity warning :message \"w\")
                     (:start (3 . 1) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        run("(findings-list)", &env, &ctx);
        run("(next-error)", &env, &ctx);
        let point = |ctx: &Ctx| {
            ctx.with_buffer(&source, |buf| buf.text.cursor_pos())
                .unwrap()
        };
        assert_eq!(point(&ctx), (1, 5));
        run("(next-error)", &env, &ctx);
        assert_eq!(point(&ctx), (2, 1));
    }

    #[test]
    fn the_list_is_asked_of_the_store_again_before_each_step() {
        let sandbox = Sandbox::new("refresh");
        let file = sandbox.file("a.txt", TEXT);
        let (ctx, env) = editor();
        open(&file, &env, &ctx);
        let source = buffer_of(&file, &ctx);
        run(
            &format!(
                "(finding-set 'lint {file:?} '((:start (1 . 1) :severity error :message \"e\")))"
            ),
            &env,
            &ctx,
        );
        run("(findings-list)", &env, &ctx);
        run("(next-error)", &env, &ctx);
        // The producer speaks again: a second finding, after the first.
        run(
            &format!(
                "(finding-set 'lint {file:?}
                   '((:start (1 . 1) :severity error :message \"e\")
                     (:start (3 . 2) :severity error :message \"new\")))"
            ),
            &env,
            &ctx,
        );
        run("(next-error)", &env, &ctx);
        assert_eq!(
            ctx.with_buffer(&source, |buf| buf.text.cursor_pos())
                .unwrap(),
            (2, 2)
        );
        let text = ctx
            .with_buffer("*Findings*", |buf| buf.text.to_string())
            .unwrap();
        assert!(text.contains("a.txt:3:3: error: new (lint)"), "{text}");
    }

    #[test]
    fn the_list_under_a_directory_names_the_files_from_it() {
        let sandbox = Sandbox::new("under");
        let a = sandbox.file("a.txt", TEXT);
        let b = sandbox.file("b.txt", TEXT);
        let (ctx, env) = editor();
        run(
            &format!(
                "(finding-set 'lint {a:?} '((:start (1 . 0) :severity error :message \"in a\")))
                 (finding-set 'lint {b:?} '((:start (2 . 0) :severity hint :message \"in b\")))"
            ),
            &env,
            &ctx,
        );
        run(
            &format!("(findings-list-under {:?})", sandbox.path()),
            &env,
            &ctx,
        );
        let text = ctx
            .with_buffer("*Findings*", |buf| buf.text.to_string())
            .unwrap();
        assert_eq!(
            text,
            format!(
                "--- 2 findings under {} ---\n\
                 a.txt:1:1: error: in a (lint)\n\
                 b.txt:2:1: hint: in b (lint)\n",
                sandbox.path()
            )
        );
    }

    // ----------------------------------------------------------------
    // The faces
    // ----------------------------------------------------------------

    #[test]
    fn every_finding_face_has_a_look_as_the_editor_ships() {
        let theme = crate::ui::Theme::default();
        for face in [
            Face::FINDING_ERROR,
            Face::FINDING_WARNING,
            Face::FINDING_INFO,
            Face::FINDING_HINT,
            Face::FINDING_ERROR_SIGN,
            Face::FINDING_WARNING_SIGN,
            Face::FINDING_INFO_SIGN,
            Face::FINDING_HINT_SIGN,
        ] {
            assert!(!theme.style(face).is_plain(), "{}", face.name());
        }
    }
}
