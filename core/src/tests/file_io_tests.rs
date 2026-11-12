//! File I/O under stalled calls; command-thread and recovery progress stay independent.
//! Design: doc/architettura-editor.typ, <file-su-disco>.
use crate::{EditorState, Env, GapBuffer};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) fn settle(ctx: &EditorState<GapBuffer>, env: &Arc<Env<EditorState<GapBuffer>>>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ctx.file_io_idle() {
        ctx.tick(env);
        assert!(Instant::now() < deadline, "file I/O did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

use crate::buffer::{
    FileStamp,
    disk::{self, WriteFailure},
};
use crate::editor::file_io::{self, Lane};
use crate::jobs::autosave::{auto_save_directory, auto_save_path};
use crate::{ELispExp, KeyCode, KeyEvent, KeyModifiers, Parser, create_global_env, eval};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

type Ctx = EditorState<GapBuffer>;
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rsedit-io-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into()
    }
    fn file(&self) -> String {
        let path = self.path("file.txt");
        std::fs::write(&path, "old").unwrap();
        path
    }
    fn editor(&self) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env().unwrap();
        env.set_variable(
            "auto-save-directory".into(),
            ELispExp::string(self.path("cache")),
        );
        env.set_variable("use-short-answers".into(), ELispExp::t());
        crate::tests::shipped::load_auto_revert(&env, &ctx);
        (ctx, env)
    }
    fn copy(&self, path: &str) -> PathBuf {
        auto_save_path(path, Some(&self.path("cache")))
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn call(
    ctx: &Ctx,
    env: &Arc<Env<Ctx>>,
    name: &str,
    args: Vec<ELispExp<GapBuffer>>,
) -> ELispExp<GapBuffer> {
    let mut form = vec![ELispExp::symbol(name.into())];
    form.extend(args);
    eval(&ELispExp::form(form), env.clone(), ctx).unwrap()
}
fn run(ctx: &Ctx, env: &Arc<Env<Ctx>>, source: &str) -> ELispExp<GapBuffer> {
    eval(
        &Parser::new(&format!("(progn {source})")).next().unwrap(),
        env.clone(),
        ctx,
    )
    .unwrap()
}
fn open(ctx: &Ctx, env: &Arc<Env<Ctx>>, path: &str) {
    assert!(matches!(
        call(ctx, env, "find-file", vec![ELispExp::string(path.into())]),
        ELispExp::String(_)
    ));
}
fn yes(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
    ctx.handle_key_event(
        KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::default(),
        },
        env,
    );
}

#[test]
fn default_recovery_directory_uses_the_platform_cache_utility() {
    let (_, env) = create_global_env::<GapBuffer>().unwrap();
    let cache = crate::paths::app_dir_path(crate::AppDir::Cache).unwrap();
    assert_eq!(
        auto_save_directory(&env).unwrap(),
        Some(cache.join("autosave").to_string_lossy().into())
    );
    assert_eq!(file_io::timeout(&env), Some(Duration::from_secs(10)));
    env.set_variable("file-io-timeout".into(), ELispExp::number(2.5));
    assert_eq!(file_io::timeout(&env), Some(Duration::from_millis(2500)));
    env.set_variable("file-io-timeout".into(), ELispExp::nil());
    assert_eq!(file_io::timeout(&env), None);
}

#[test]
fn timeout_retains_reservation_while_commands_and_local_recovery_progress() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    let handle = ctx.file_handle(&ctx.get_current_buffer_name()).unwrap();
    let (release, blocked) = mpsc::channel();
    assert!(
        ctx.start_file_io(
            Lane::Write,
            format!("file:{path}"),
            "Saving",
            Some(handle.clone()),
            Some(Duration::ZERO),
            move || blocked.recv().unwrap(),
            |_, _, ()| {}
        )
        .is_some()
    );
    ctx.tick(&env);
    assert!(ctx.get_echo_message().contains("not cancelled"));
    assert!(handle.read().unwrap().file_io_status().contains("Saving?"));
    assert!(ctx.reserve_file_write(handle.clone(), &path).is_none());
    assert!(
        ctx.reserve_file_write(handle.clone(), &files.path("elsewhere"))
            .is_some()
    );
    assert!(call(&ctx, &env, "save-buffer-async", vec![]).is_nil());
    run(&ctx, &env, "(insert \"new\")");
    assert!(
        ctx.auto_save_async_turn(0, auto_save_directory(&env), file_io::timeout(&env))
            .1
            > 0
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while !files.copy(&path).exists() {
        ctx.tick(&env);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        std::fs::read_to_string(files.copy(&path)).unwrap(),
        "newold"
    );
    assert!(!ctx.close_buffer(&ctx.get_current_buffer_name(), &env));
    call(&ctx, &env, "quit--confirm", vec![]);
    assert!(ctx.is_running());
    release.send(()).unwrap();
    settle(&ctx, &env);
    assert!(!ctx.file_writes_pending());
}

#[test]
fn lanes_are_bounded_and_a_blocked_watcher_does_not_use_read_or_recovery_slots() {
    let files = Files::new();
    let (ctx, env) = files.editor();
    let mut releases = Vec::new();
    for lane in [Lane::Read, Lane::Write, Lane::Watch, Lane::Recovery] {
        let limit = match lane {
            Lane::Read | Lane::Watch => 2,
            Lane::Write => 4,
            Lane::Recovery => 1,
        };
        for n in 0..limit {
            let (tx, rx) = mpsc::channel();
            releases.push(tx);
            assert!(
                ctx.start_file_io(
                    lane,
                    format!("{lane:?}:{n}"),
                    "Test",
                    None,
                    None,
                    move || rx.recv().unwrap(),
                    |_, _, ()| {}
                )
                .is_some()
            );
        }
        assert!(
            ctx.start_file_io(
                lane,
                format!("{lane:?}:extra"),
                "Test",
                None,
                None,
                || (),
                |_, _, ()| {}
            )
            .is_none()
        );
    }
    for tx in releases {
        tx.send(()).unwrap();
    }
    settle(&ctx, &env);
}

#[test]
fn late_save_keeps_new_edits_and_the_newest_recovery_copy() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"A\")");
    assert!(call(&ctx, &env, "save-buffer-async", vec![]).is_truthy());
    run(&ctx, &env, "(insert \"B\")");
    let name = ctx.get_current_buffer_name();
    let handle = ctx.file_handle(&name).unwrap();
    let version = handle.read().unwrap().version;
    ctx.write_recovery(&handle, &files.copy(&path), version, "ABold")
        .unwrap();
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Aold");
    assert_eq!(
        ctx.with_buffer(&name, |buf| (buf.text.to_string(), buf.is_modified)),
        Some(("ABold".into(), true))
    );
    assert_eq!(std::fs::read_to_string(files.copy(&path)).unwrap(), "ABold");
    call(&ctx, &env, "save-buffer-async", vec![]);
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ABold");
    assert!(!files.copy(&path).exists());
}

#[test]
fn failed_write_preserves_buffer_visited_path_and_recovery() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"mine\")");
    let parent_file = files.path("not-a-directory");
    std::fs::write(&parent_file, "x").unwrap();
    call(
        &ctx,
        &env,
        "write-file-async",
        vec![ELispExp::string(format!("{parent_file}/new"))],
    );
    settle(&ctx, &env);
    assert_eq!(
        ctx.with_current_buffer(|buf| (buf.file_path.clone(), buf.is_modified)),
        (Some(path.clone()), true)
    );
    assert_eq!(
        std::fs::read_to_string(files.copy(&path)).unwrap(),
        "mineold"
    );
    assert!(ctx.get_echo_message().contains("Cannot save"));
}

#[test]
fn cache_errors_are_visible_and_never_fall_back_to_the_target_directory() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    let bad_cache = files.path("cache");
    std::fs::write(&bad_cache, "not a directory").unwrap();
    run(&ctx, &env, "(insert \"mine\")");
    ctx.auto_save_async_turn(0, auto_save_directory(&env), file_io::timeout(&env));
    settle(&ctx, &env);
    assert!(ctx.with_current_buffer(|buf| buf.auto_save_failing));
    assert!(!auto_save_path(&path, None).exists());
    call(&ctx, &env, "save-buffer-async", vec![]);
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mineold");
    assert!(ctx.with_current_buffer(|buf| buf.auto_save_failing));
    assert!(!auto_save_path(&path, None).exists());
}

#[test]
fn save_detects_external_changes_without_waiting_for_watcher_and_rechecks_confirmation() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"mine\")");
    std::fs::write(&path, "other person's longer text").unwrap();
    call(&ctx, &env, "save-buffer-async", vec![]);
    settle(&ctx, &env);
    assert!(ctx.with_current_buffer(|buf| buf.stale));
    std::fs::write(&path, "a second external update").unwrap();
    yes(&ctx, &env);
    settle(&ctx, &env);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a second external update"
    );
    yes(&ctx, &env);
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mineold");
    assert!(!files.copy(&path).exists());
}

#[test]
fn an_edit_while_confirmation_is_open_invalidates_the_save() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"mine\")");
    std::fs::write(&path, "external update").unwrap();
    call(&ctx, &env, "save-buffer-async", vec![]);
    settle(&ctx, &env);
    run(&ctx, &env, "(insert \"newer\")");
    yes(&ctx, &env);
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external update");
    assert!(ctx.with_current_buffer(|buf| buf.is_modified));
}

#[test]
fn a_new_edit_prevents_late_revert_from_replacing_text() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    std::fs::write(&path, "updated on disk").unwrap();
    assert!(call(&ctx, &env, "revert-buffer-async", vec![]).is_truthy());
    run(&ctx, &env, "(insert \"new\")");
    settle(&ctx, &env);
    assert_eq!(
        ctx.with_current_buffer(|buf| buf.text.to_string()),
        "newold"
    );
    assert!(ctx.with_current_buffer(|buf| buf.is_modified));
}

#[test]
fn late_open_never_steals_an_edited_or_recreated_origin() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    call(
        &ctx,
        &env,
        "find-file-async",
        vec![ELispExp::string(path.clone())],
    );
    run(&ctx, &env, "(insert \"keep me\")");
    settle(&ctx, &env);
    assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    assert_eq!(
        ctx.with_current_buffer(|buf| buf.text.to_string()),
        "keep me"
    );
    assert!(ctx.has_buffer("file.txt"));
    let second = files.path("other.txt");
    std::fs::write(&second, "another").unwrap();
    call(
        &ctx,
        &env,
        "find-file-async",
        vec![ELispExp::string(second)],
    );
    ctx.close_buffer("*scratch*", &env);
    run(&ctx, &env, "(buffer-create \"*scratch*\")");
    settle(&ctx, &env);
    assert_ne!(ctx.get_current_buffer_name(), "other.txt");
}

#[test]
fn existing_lisp_file_calls_remain_synchronous() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    assert_eq!(ctx.with_current_buffer(|buf| buf.text.to_string()), "old");
    run(&ctx, &env, "(insert \"sync\") (save-buffer)");
    call(&ctx, &env, "save-buffer", vec![]);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "syncold");
    settle(&ctx, &env);
}

#[test]
fn checked_write_refuses_a_missing_or_changed_known_file() {
    let files = Files::new();
    let path = files.file();
    let stamp = FileStamp::try_of(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        disk::write_checked(&path, b"mine", stamp),
        Err(WriteFailure::Conflict(None))
    ));
    assert!(!std::path::Path::new(&path).exists());
    std::fs::write(&path, "external text").unwrap();
    assert!(matches!(
        disk::write_checked(&path, b"mine", stamp),
        Err(WriteFailure::Conflict(Some(_)))
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external text");
}

#[cfg(unix)]
#[test]
fn a_symlink_loop_is_an_access_error_and_cannot_create_an_empty_buffer() {
    let files = Files::new();
    let (ctx, env) = files.editor();
    let path = files.path("loop");
    std::os::unix::fs::symlink("loop", &path).unwrap();
    assert!(FileStamp::try_of(&path).is_err());
    assert!(
        call(
            &ctx,
            &env,
            "find-file",
            vec![ELispExp::string(path.clone())]
        )
        .is_nil()
    );
    call(&ctx, &env, "find-file-async", vec![ELispExp::string(path)]);
    settle(&ctx, &env);
    assert!(!ctx.has_buffer("loop"));
    assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
}

#[test]
fn save_as_to_a_local_file_is_possible_while_a_remote_save_is_pending() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"A\")");
    assert!(call(&ctx, &env, "save-buffer-async", vec![]).is_truthy());
    run(&ctx, &env, "(insert \"B\")");
    let local = files.path("local-copy.txt");
    assert!(
        call(
            &ctx,
            &env,
            "write-file-async",
            vec![ELispExp::string(local.clone())]
        )
        .is_truthy()
    );
    settle(&ctx, &env);
    assert_eq!(std::fs::read_to_string(&local).unwrap(), "ABold");
    assert_eq!(
        ctx.with_current_buffer(|buf| (buf.file_path.clone(), buf.is_modified)),
        (Some(local), false)
    );
}

#[test]
fn interactive_quit_waits_for_save_completion() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"saved\")");
    run(&ctx, &env, "(call-interactively 'quit)");
    yes(&ctx, &env);
    assert!(ctx.is_running(), "the confirmation starts I/O, not exit");
    settle(&ctx, &env);
    assert!(!ctx.is_running());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "savedold");
}

#[test]
fn an_access_error_and_reconnect_preserve_modified_text_and_detect_conflicts() {
    let files = Files::new();
    let mount = files.path("mount");
    std::fs::create_dir(&mount).unwrap();
    let path = format!("{mount}/remote.txt");
    std::fs::write(&path, "remote").unwrap();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"mine\")");
    let name = ctx.get_current_buffer_name();
    let held = files.path("held");
    std::fs::rename(&mount, &held).unwrap();
    std::fs::write(&mount, "unavailable directory").unwrap();
    ctx.store_file_check(ctx.file_check_for(&name).expect("unavailable"));
    assert!(ctx.with_current_buffer(|buf| buf.file_io_status().contains("Invalid path")));
    assert_eq!(
        ctx.with_current_buffer(|buf| buf.text.to_string()),
        "mineremote"
    );
    std::fs::remove_file(&mount).unwrap();
    std::fs::rename(&held, &mount).unwrap();
    std::fs::write(&path, "different after reconnect").unwrap();
    ctx.store_file_check(ctx.file_check_for(&name).expect("conflict"));
    assert!(ctx.with_current_buffer(|buf| buf.stale));
    assert_eq!(
        ctx.with_current_buffer(|buf| buf.text.to_string()),
        "mineremote"
    );
}

#[test]
fn async_open_runs_find_file_hooks_after_showing_the_new_buffer() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    run(
        &ctx,
        &env,
        "(setq hook-window nil) (defun io-test-hook () (setq hook-window (window-buffer))) (add-hook nil \"find-file-hook\" 'io-test-hook)",
    );
    call(&ctx, &env, "find-file-async", vec![ELispExp::string(path)]);
    settle(&ctx, &env);
    assert_eq!(
        run(&ctx, &env, "hook-window"),
        ELispExp::string("file.txt".into())
    );
}

use crate::buffer::io_status::{
    AUTO_SAVE_REPORT, AutoSaveReport, FileIssue, IoCause, is_missing, utc,
};
use std::{
    io::{Error, ErrorKind},
    time::{SystemTime, UNIX_EPOCH},
};

fn status_text(value: ELispExp<GapBuffer>) -> String {
    match value {
        ELispExp::String(text) => text.to_string(),
        other => panic!("expected status text, got {other:?}"),
    }
}

#[test]
fn typed_causes_keep_native_errors_and_do_not_guess_from_messages() {
    for (kind, cause) in [
        (ErrorKind::NotFound, IoCause::Missing),
        (ErrorKind::PermissionDenied, IoCause::Denied),
        (ErrorKind::ConnectionReset, IoCause::Network),
        (ErrorKind::NetworkUnreachable, IoCause::Network),
        (ErrorKind::TimedOut, IoCause::Timeout),
        (ErrorKind::StorageFull, IoCause::Full),
        (ErrorKind::ReadOnlyFilesystem, IoCause::ReadOnly),
        (ErrorKind::WouldBlock, IoCause::Busy),
        (ErrorKind::NotADirectory, IoCause::InvalidPath),
        (ErrorKind::Other, IoCause::Other),
    ] {
        let error = Error::new(kind, "original OS explanation");
        let issue = FileIssue::from_io(&error);
        assert_eq!(issue.cause, cause);
        assert_eq!(issue.kind, Some(kind));
        assert_eq!(issue.message, "original OS explanation");
    }
    assert_eq!(
        FileIssue::from_io(&Error::other("Network unreachable")).cause,
        IoCause::Other
    );
    let native = Error::from_raw_os_error(if cfg!(windows) { 5 } else { 13 });
    assert_eq!(FileIssue::from_io(&native).os_code, native.raw_os_error());
    assert_eq!(FileIssue::from_io(&native).message, native.to_string());
    assert!(is_missing(&Error::from(ErrorKind::NotFound)));
    assert!(!is_missing(&Error::from(ErrorKind::PermissionDenied)));
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn native_network_errors_are_not_mistaken_for_a_missing_file() {
    let error = Error::from_raw_os_error(if cfg!(windows) { 53 } else { 116 });
    assert_eq!(FileIssue::from_io(&error).cause, IoCause::Network);
    assert!(!is_missing(&error));
}

#[test]
fn utc_dates_handle_epoch_boundaries_and_gregorian_leap_days() {
    for (seconds, text) in [
        (0, "1970-01-01 00:00:00 UTC"),
        (951782400, "2000-02-29 00:00:00 UTC"),
        (1709164800, "2024-02-29 00:00:00 UTC"),
        (4107542400, "2100-03-01 00:00:00 UTC"),
    ] {
        assert_eq!(utc(UNIX_EPOCH + Duration::from_secs(seconds)), text);
    }
    assert_eq!(
        utc(UNIX_EPOCH - Duration::from_millis(1)),
        "1969-12-31 23:59:59 UTC"
    );
}

#[test]
fn recovery_history_records_failure_and_repair_without_losing_last_success() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"A\")");
    ctx.auto_save_async_turn(0, auto_save_directory(&env), None);
    settle(&ctx, &env);
    let first = ctx.with_current_buffer(|buf| {
        buf.data::<AutoSaveReport>(AUTO_SAVE_REPORT)
            .unwrap()
            .clone()
    });
    assert!(first.outcome.is_ok());
    assert_eq!(first.copy.as_ref(), Some(&files.copy(&path)));
    assert_eq!(first.last_success, Some(first.at));
    assert!(first.at <= SystemTime::now());
    std::fs::remove_dir_all(files.path("cache")).unwrap();
    std::fs::write(files.path("cache"), "obstruction").unwrap();
    run(&ctx, &env, "(insert \"B\")");
    ctx.auto_save_async_turn(0, auto_save_directory(&env), None);
    settle(&ctx, &env);
    let failed = ctx.with_current_buffer(|buf| {
        buf.data::<AutoSaveReport>(AUTO_SAVE_REPORT)
            .unwrap()
            .clone()
    });
    let expected =
        FileIssue::from_io(&std::fs::create_dir_all(files.path("cache")).unwrap_err()).cause;
    assert_eq!(failed.outcome.unwrap_err().cause, expected);
    assert_eq!(failed.last_success, first.last_success);
    assert!(ctx.with_current_buffer(|buf| {
        buf.file_io_status()
            .contains(&format!("AS {}", expected.short()))
    }));
    assert!(status_text(call(&ctx, &env, "auto-save-status", vec![])).contains(&utc(first.at)));
    std::fs::remove_file(files.path("cache")).unwrap();
    ctx.auto_save_async_turn(0, auto_save_directory(&env), None);
    settle(&ctx, &env);
    assert!(!ctx.with_current_buffer(|buf| buf.auto_save_failing));
    assert!(ctx.with_current_buffer(|buf| {
        buf.data::<AutoSaveReport>(AUTO_SAVE_REPORT)
            .unwrap()
            .outcome
            .is_ok()
    }));
}

#[test]
fn history_survives_successful_save_and_recovery_copy_removal() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"A\")");
    call(&ctx, &env, "save-buffer-async", vec![]);
    settle(&ctx, &env);
    assert!(!files.copy(&path).exists());
    assert!(ctx.with_current_buffer(|buf| {
        let report = buf.data::<AutoSaveReport>(AUTO_SAVE_REPORT).unwrap();
        report.removed && report.outcome.is_ok()
    }));
    assert!(
        status_text(call(&ctx, &env, "auto-save-status", vec![]))
            .contains("removed after successful save")
    );
    assert!(ctx.with_current_buffer(|buf| buf.file_io_status().contains("AS ok")));
}

#[test]
fn an_older_failure_callback_cannot_replace_a_new_recovery_success() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(&ctx, &env, "(insert \"A\")");
    std::fs::write(files.path("cache"), "obstruction").unwrap();
    ctx.auto_save_async_turn(0, auto_save_directory(&env), None);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ctx.file_io_ready() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    std::fs::remove_file(files.path("cache")).unwrap();
    let handle = ctx.file_handle(&ctx.get_current_buffer_name()).unwrap();
    let version = handle.read().unwrap().version;
    ctx.write_recovery(&handle, &files.copy(&path), version, "Aold")
        .unwrap();
    settle(&ctx, &env);
    assert!(!ctx.with_current_buffer(|buf| buf.auto_save_failing));
    assert!(ctx.with_current_buffer(|buf| {
        buf.data::<AutoSaveReport>(AUTO_SAVE_REPORT)
            .unwrap()
            .outcome
            .is_ok()
    }));
    assert!(!ctx.get_echo_message().contains("Auto-save failed"));
}

#[test]
fn skipped_older_snapshot_does_not_forge_another_auto_save_time() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    let handle = ctx.file_handle(&ctx.get_current_buffer_name()).unwrap();
    ctx.write_recovery(&handle, &files.copy(&path), 20, "newest")
        .unwrap();
    let first = handle
        .read()
        .unwrap()
        .data::<AutoSaveReport>(AUTO_SAVE_REPORT)
        .unwrap()
        .at;
    ctx.write_recovery(&handle, &files.copy(&path), 19, "older")
        .unwrap();
    assert_eq!(
        handle
            .read()
            .unwrap()
            .data::<AutoSaveReport>(AUTO_SAVE_REPORT)
            .unwrap()
            .at,
        first
    );
    assert_eq!(
        std::fs::read_to_string(files.copy(&path)).unwrap(),
        "newest"
    );
}

#[test]
fn missing_files_have_a_distinct_state_and_reconnection_clears_it() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    let name = ctx.get_current_buffer_name();
    std::fs::remove_file(&path).unwrap();
    ctx.store_file_check(ctx.file_check_for(&name).unwrap());
    assert_eq!(
        ctx.with_current_buffer(|buf| buf.data::<FileIssue>(file_io::ERROR).unwrap().cause),
        IoCause::Missing
    );
    std::fs::write(&path, "longer reconnected text").unwrap();
    ctx.store_file_check(ctx.file_check_for(&name).unwrap());
    assert!(ctx.with_current_buffer(|buf| buf.data::<FileIssue>(file_io::ERROR).is_none()));
    assert_eq!(
        ctx.with_current_buffer(|buf| buf.text.to_string()),
        "longer reconnected text"
    );
}

#[test]
fn configuration_failure_records_a_completed_attempt_with_no_network_fallback() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    run(
        &ctx,
        &env,
        "(insert \"A\") (setq auto-save-directory 'unavailable)",
    );
    ctx.auto_save_async_turn(0, auto_save_directory(&env), None);
    assert!(ctx.with_current_buffer(|buf| {
        let report = buf.data::<AutoSaveReport>(AUTO_SAVE_REPORT).unwrap();
        report.copy.is_none()
            && report.outcome.as_ref().unwrap_err().cause == IoCause::Configuration
            && report.last_success.is_none()
    }));
    assert!(!auto_save_path(&path, None).exists());
}

#[test]
fn status_commands_work_while_io_is_pending_without_starting_another_operation() {
    let files = Files::new();
    let path = files.file();
    let (ctx, env) = files.editor();
    open(&ctx, &env, &path);
    let (tx, rx) = mpsc::channel();
    let handle = ctx.file_handle(&ctx.get_current_buffer_name()).unwrap();
    ctx.start_file_io(
        Lane::Write,
        format!("file:{path}"),
        "Saving",
        Some(handle),
        None,
        move || rx.recv().unwrap(),
        |_, _, ()| {},
    );
    assert!(status_text(call(&ctx, &env, "file-io-status", vec![])).contains("Saving"));
    assert!(status_text(call(&ctx, &env, "auto-save-status", vec![])).contains("never attempted"));
    assert!(ctx.file_writes_pending());
    tx.send(()).unwrap();
    settle(&ctx, &env);
}
