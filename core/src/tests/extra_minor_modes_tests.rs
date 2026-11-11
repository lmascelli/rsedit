//! Logical-line highlighting, identifier motion and save-hook integration.
//! Design: `doc/architettura-editor.typ, <modi-minori>`.
use crate::ui::Face;
use crate::{BufferTrait, ELispExp, EditorState, Env, GapBuffer, Parser, create_global_env, eval};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
type Ctx = EditorState<GapBuffer>;

fn run(ctx: &Ctx, env: &Arc<Env<Ctx>>, source: &str) -> ELispExp<GapBuffer> {
    eval(
        &Parser::new(&format!("(progn {source})")).next().unwrap(),
        env.clone(),
        ctx,
    )
    .unwrap()
}
fn setup(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    for source in [
        include_str!("../../lisp/base/commands.lisp"),
        include_str!("../../lisp/base/debug.lisp"),
        include_str!("../../lisp/base/common-keymaps.lisp"),
        include_str!("../../lisp/features/hl-line.lisp"),
        include_str!("../../lisp/features/subword.lisp"),
        include_str!("../../lisp/features/visual-line.lisp"),
        include_str!("../../lisp/features/whitespace.lisp"),
    ] {
        run(&ctx, &env, source);
    }
    env.set_variable("frame-width".into(), ELispExp::number(40.));
    env.set_variable("frame-height".into(), ELispExp::number(12.));
    ctx.with_current_buffer_mut(|buf| buf.text = GapBuffer::from(text));
    (ctx, env)
}
fn point(ctx: &Ctx) -> usize {
    ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d())
}
fn text(ctx: &Ctx) -> String {
    ctx.with_current_buffer(|buf| buf.text.to_string())
}
fn settle(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
    super::file_io_tests::settle(ctx, env);
}
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rsedit-modes-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into()
    }
    fn editor(&self, contents: &str) -> (Ctx, Arc<Env<Ctx>>, String) {
        let (ctx, env) = setup("");
        let path = self.path("file.txt");
        std::fs::write(&path, contents).unwrap();
        env.set_variable(
            "auto-save-directory".into(),
            ELispExp::string(self.path("cache")),
        );
        run(&ctx, &env, &format!("(find-file {path:?})"));
        (ctx, env, path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn modes_are_per_buffer_and_disabled_initially() {
    let (ctx, env) = setup("camelCase");
    assert!(run(&ctx, &env, "(minor-mode-p 'subword-mode)").is_nil());
    assert!(run(&ctx, &env, "(minor-mode-p 'hl-line-mode)").is_nil());
    run(
        &ctx,
        &env,
        "(subword-mode 1) (hl-line-mode 1) (buffer-create \"other\") (switch-to-buffer \"other\")",
    );
    assert!(run(&ctx, &env, "(minor-mode-p 'subword-mode)").is_nil());
    assert!(run(&ctx, &env, "(minor-mode-p 'hl-line-mode)").is_nil());
}

#[test]
fn subwords_follow_camel_case_acronyms_and_numbers_in_both_directions() {
    let (ctx, env) = setup("HTTPServer2D camelCase");
    run(&ctx, &env, "(subword-mode 1)");
    for position in [4, 10, 11, 12, 18, 22] {
        run(&ctx, &env, "(forward-word)");
        assert_eq!(point(&ctx), position);
    }
    for position in [18, 13, 11, 10, 4, 0] {
        run(&ctx, &env, "(backward-word)");
        assert_eq!(point(&ctx), position);
    }
}

#[test]
fn separators_are_literal_configurable_and_can_override_one_buffer() {
    let (ctx, env) = setup("foo-bar_name2tail");
    run(&ctx, &env, "(subword-mode 1) (forward-word)");
    assert_eq!(point(&ctx), 7);
    run(
        &ctx,
        &env,
        "(goto-char 0) (setq subword-separators \"_-\") (forward-word)",
    );
    assert_eq!(point(&ctx), 3);
    run(&ctx, &env, "(forward-word)");
    assert_eq!(point(&ctx), 7);
    run(
        &ctx,
        &env,
        "(buffer-put 'subword-separators \"_-a\") (goto-char 4) (forward-word)",
    );
    assert_eq!(point(&ctx), 5);
    run(&ctx, &env, "(subword-mode -1) (goto-char 0) (forward-word)");
    assert_eq!(point(&ctx), 3);
}

#[test]
fn subword_prefixes_zero_negative_and_extreme_counts_are_bounded() {
    let (ctx, env) = setup("fooBarBaz");
    run(&ctx, &env, "(subword-mode 1) (forward-word 2)");
    assert_eq!(point(&ctx), 6);
    run(&ctx, &env, "(forward-word 0) (backward-word 0)");
    assert_eq!(point(&ctx), 6);
    run(&ctx, &env, "(forward-word -1)");
    assert_eq!(point(&ctx), 3);
    run(&ctx, &env, "(backward-word -1)");
    assert_eq!(point(&ctx), 6);
    run(&ctx, &env, "(forward-word 1000000000000)");
    assert_eq!(point(&ctx), 9);
    run(&ctx, &env, "(backward-word 1000000000000)");
    assert_eq!(point(&ctx), 0);
}

#[test]
fn subword_unicode_offsets_and_separator_runs_are_character_based() {
    let (ctx, env) = setup("écoleÜber__42βeta");
    run(&ctx, &env, "(subword-mode 1)");
    for position in [5, 9, 13, 17] {
        run(&ctx, &env, "(forward-word)");
        assert_eq!(point(&ctx), position);
    }
    run(&ctx, &env, "(backward-word 4)");
    assert_eq!(point(&ctx), 0);
}

#[test]
fn subword_killing_matches_motion_and_preserves_undo_and_kill_ring() {
    let (ctx, env) = setup("fooBar-baz");
    run(
        &ctx,
        &env,
        "(setq subword-separators \"_-\") (subword-mode 1) (kill-word)",
    );
    assert_eq!(text(&ctx), "Bar-baz");
    run(&ctx, &env, "(yank)");
    assert_eq!(text(&ctx), "fooBar-baz");
    run(&ctx, &env, "(end-of-buffer) (backward-kill-word)");
    assert_eq!(text(&ctx), "fooBar-");
    run(&ctx, &env, "(undo)");
    assert_eq!(text(&ctx), "fooBar-baz");
}

#[test]
fn logical_line_highlight_covers_all_wrapped_rows_and_padding() {
    let (ctx, env) = setup("abcdefgh ijklmnop qrst\nother");
    run(&ctx, &env, "(visual-line-mode 1) (hl-line-mode 1)");
    let version = ctx.with_current_buffer(|buf| buf.version);
    let frame = ctx.snapshot(&env, 10, 10);
    let view = frame.focused_view().unwrap();
    let highlights: Vec<_> = view
        .highlights
        .iter()
        .filter(|h| h.face == Face::HL_LINE)
        .collect();
    assert_eq!(
        highlights.iter().map(|h| h.row).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(
        highlights
            .iter()
            .all(|h| h.start_col == 0 && h.end_col == view.rect.width)
    );
    assert_eq!(ctx.with_current_buffer(|buf| buf.version), version);
    run(&ctx, &env, "(hl-line-mode -1)");
    assert!(
        !ctx.snapshot(&env, 10, 10)
            .focused_view()
            .unwrap()
            .highlights
            .iter()
            .any(|h| h.face == Face::HL_LINE)
    );
}

#[test]
fn only_the_selected_window_highlights_a_shared_buffer() {
    let (ctx, env) = setup("one\ntwo");
    run(
        &ctx,
        &env,
        "(hl-line-mode 1) (split-window-right) (other-window 1)",
    );
    let frame = ctx.snapshot(&env, 40, 12);
    for view in frame.views.iter().filter(|v| v.buffer_name == "*scratch*") {
        assert_eq!(
            view.highlights.iter().any(|h| h.face == Face::HL_LINE),
            view.is_focused
        );
    }
}

#[test]
fn line_background_is_below_the_region_and_covers_an_empty_line() {
    let (ctx, env) = setup("abc\n\nend");
    run(&ctx, &env, "(hl-line-mode 1) (set-mark) (forward-char 2)");
    let frame = ctx.snapshot(&env, 40, 12);
    let h = &frame.focused_view().unwrap().highlights;
    assert!(
        h.iter().position(|h| h.face == Face::HL_LINE).unwrap()
            < h.iter().position(|h| h.face == Face::REGION).unwrap()
    );
    run(&ctx, &env, "(deactivate-mark) (goto-char 4)");
    assert!(
        ctx.snapshot(&env, 40, 12)
            .focused_view()
            .unwrap()
            .highlights
            .iter()
            .any(|h| h.face == Face::HL_LINE && h.row == 1 && h.end_col == 40)
    );
}

#[test]
fn all_manual_save_paths_run_hooks_before_their_snapshot() {
    let files = Files::new();
    let (ctx, env, path) = files.editor("body");
    run(
        &ctx,
        &env,
        "(setq saves 0) (defun stamp-save () (setq saves (+ saves 1)) (end-of-buffer) (insert \"H\")) (add-hook nil \"before-save-hook\" 'stamp-save) (save-buffer)",
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "bodyH");
    run(&ctx, &env, "(save-buffer-async)");
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "bodyHH");
    let other = files.path("other.txt");
    run(&ctx, &env, &format!("(write-file {other:?})"));
    let last = files.path("last.txt");
    run(&ctx, &env, &format!("(write-file-async {last:?})"));
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(last).unwrap(), "bodyHHHH");
    assert_eq!(run(&ctx, &env, "saves"), ELispExp::number(4.));
}

#[test]
fn cleanup_uses_the_minor_hook_and_cleans_whole_buffer_with_active_region() {
    let files = Files::new();
    let (ctx, env, path) = files.editor("one  \ntwo \n\n");
    run(
        &ctx,
        &env,
        "(whitespace-cleanup-mode 1) (set-mark) (forward-char 3) (save-buffer)",
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
    run(
        &ctx,
        &env,
        "(remove-hook 'whitespace-cleanup-mode \"before-save-hook\" 'whitespace--cleanup) (deactivate-mark) (end-of-buffer) (insert \"tail  \") (save-buffer)",
    );
    assert!(std::fs::read_to_string(&path).unwrap().ends_with("tail  "));
}

#[test]
fn failing_hook_aborts_both_sync_and_async_saves_and_guard_is_released() {
    let files = Files::new();
    let (ctx, env, path) = files.editor("original");
    run(
        &ctx,
        &env,
        "(defun reject-save () (error \"reject\")) (add-hook nil \"before-save-hook\" 'reject-save) (insert \"mine\") (save-buffer)",
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    assert!(run(&ctx, &env, "(save-buffer-async)").is_nil());
    assert!(!ctx.file_writes_pending());
    assert!(ctx.get_echo_message().contains("save aborted"));
    run(
        &ctx,
        &env,
        "(remove-hook nil \"before-save-hook\" 'reject-save) (save-buffer-async)",
    );
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "mineoriginal");
}

#[test]
fn recursive_save_hook_cannot_start_overlapping_writes() {
    let files = Files::new();
    let (ctx, env, path) = files.editor("original");
    run(
        &ctx,
        &env,
        "(setq calls 0) (defun recursive-save () (setq calls (+ calls 1)) (save-buffer-async)) (add-hook nil \"before-save-hook\" 'recursive-save) (save-buffer-async)",
    );
    settle(&ctx, &env);
    assert_eq!(run(&ctx, &env, "calls"), ELispExp::number(1.));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
}

#[test]
fn removing_the_target_in_a_hook_aborts_without_writing_or_recreating_it() {
    let files = Files::new();
    let (ctx, env, path) = files.editor("original");
    run(
        &ctx,
        &env,
        "(defun remove-target () (kill-buffer-without-saving)) (add-hook nil \"before-save-hook\" 'remove-target) (insert \"mine\")",
    );
    assert!(run(&ctx, &env, "(save-buffer-async)").is_nil());
    assert!(!ctx.has_buffer("file.txt"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
    assert!(!ctx.file_writes_pending());
}

#[test]
fn hidden_buffer_hooks_run_in_the_target_context_and_restore_the_caller() {
    let files = Files::new();
    let (ctx, env, path) = files.editor("original");
    run(
        &ctx,
        &env,
        "(setq saved-in nil) (defun remember-target () (setq saved-in (current-buffer))) (add-hook nil \"before-save-hook\" 'remember-target) (buffer-create \"other\") (switch-to-buffer \"other\") (with-current-buffer \"file.txt\" (lambda () (save-buffer)))",
    );
    assert_eq!(
        run(&ctx, &env, "saved-in"),
        ELispExp::string("file.txt".into())
    );
    assert_eq!(ctx.get_current_buffer_name(), "other");
    assert!(PathBuf::from(path).exists());
}

#[test]
fn save_hooks_run_in_mode_order_and_each_receives_the_target_buffer() {
    let files = Files::new();
    let (ctx, env, path) = files.editor("");
    run(
        &ctx,
        &env,
        r#"
        (buffer-create "other")
        (defun major-save () (insert "M") (switch-to-buffer "other"))
        (defun minor-save () (insert "m"))
        (defun global-save () (insert "G"))
        (add-hook 'fundamental-mode "before-save-hook" 'major-save)
        (add-hook 'subword-mode "before-save-hook" 'minor-save)
        (add-hook nil "before-save-hook" 'global-save)
        (subword-mode 1)
        (save-buffer-async)
    "#,
    );
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "MmG");
    assert_eq!(
        ctx.with_buffer("other", |buf| buf.text.to_string())
            .unwrap(),
        ""
    );
    assert_eq!(ctx.get_current_buffer_name(), "file.txt");
}

#[test]
fn local_auto_save_keeps_original_text_and_never_runs_save_hooks() {
    let files = Files::new();
    let (ctx, env, path) = files.editor("one  \n");
    run(
        &ctx,
        &env,
        "(whitespace-cleanup-mode 1) (defun reject-save () (error \"manual only\")) (add-hook nil \"before-save-hook\" 'reject-save) (insert \"two \n\")",
    );
    let original = text(&ctx);
    ctx.auto_save_async_turn(0, crate::jobs::autosave::auto_save_directory(&env), None);
    settle(&ctx, &env);
    let copy = crate::jobs::autosave::auto_save_path(&path, Some(&files.path("cache")));
    assert_eq!(std::fs::read_to_string(copy).unwrap(), original);
    assert_eq!(text(&ctx), original);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "one  \n");
}
