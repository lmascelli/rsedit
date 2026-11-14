//! Looking upwards for a name: which directory answers, what is remembered
//! and forgotten, and what a disk that does not answer costs the caller.
//!
//! Design: `doc/architettura-editor.typ, <risalita>`.
use crate::editor::locate::{self, CLIMBS, Located, Question};
use crate::{ELispExp, EditorState, Env, GapBuffer, Parser, create_global_env, eval};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

type Ctx = EditorState<GapBuffer>;

/// A tree of its own under the temporary directory, removed at the end:
/// `root/outer/inner/leaf`.
struct Tree(PathBuf);

impl Tree {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rsedit-locate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("outer/inner/leaf")).unwrap();
        Self(root)
    }

    fn path(&self, relative: &str) -> String {
        self.0.join(relative).to_string_lossy().into()
    }

    /// The directory RELATIVE names, as the primitive answers it.
    fn directory(&self, relative: &str) -> ELispExp<GapBuffer> {
        ELispExp::string(crate::paths::as_directory(&self.path(relative)))
    }

    fn touch(&self, relative: &str) {
        std::fs::write(self.0.join(relative), "").unwrap();
    }

    fn question(&self, relative: &str, name: &str) -> Question {
        Question {
            start: self.path(relative),
            names: vec![name.into()],
            topmost: false,
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(ctx: &Ctx, env: &Arc<Env<Ctx>>, source: &str) -> ELispExp<GapBuffer> {
    eval(&Parser::new(source).next().unwrap(), env.clone(), ctx).unwrap()
}

/// What a disk that answers says.
fn there(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// A disk that answers nothing until the sender it is paired with is
/// dropped, and tells the truth from then on.
fn stalled() -> (mpsc::Sender<()>, impl Fn(&Path) -> bool + Send + 'static) {
    let (release, gate) = mpsc::channel::<()>();
    (release, move |path: &Path| {
        let _ = gate.recv();
        there(path)
    })
}

fn until_no_climb_is_out(ctx: &Ctx) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while ctx.locate_climbs() > 0 {
        assert!(Instant::now() < deadline, "a climb did not come back");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn the_nearest_directory_that_holds_one_of_the_names_answers() {
    let tree = Tree::new();
    tree.touch("m-far");
    tree.touch("outer/m-near");
    tree.touch("outer/m-far");
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    let from = tree.path("outer/inner/leaf");
    assert_eq!(
        run(
            &ctx,
            &env,
            &format!("(locate-dominating-file {from:?} \"m-far\")")
        ),
        tree.directory("outer"),
    );
    assert_eq!(
        run(
            &ctx,
            &env,
            &format!("(locate-dominating-file {from:?} '(\"m-none\" \"m-near\") 'nearest)")
        ),
        tree.directory("outer"),
    );
}

#[test]
fn topmost_answers_the_highest_directory_that_holds_a_name() {
    let tree = Tree::new();
    tree.touch("m-top");
    tree.touch("outer/inner/m-top");
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    let from = tree.path("outer/inner/leaf");
    assert_eq!(
        run(
            &ctx,
            &env,
            &format!("(locate-dominating-file {from:?} '(\"m-top\") 'topmost)")
        ),
        tree.directory(""),
    );
}

#[test]
fn the_starting_directory_itself_can_answer_and_a_file_starts_in_its_directory() {
    let tree = Tree::new();
    tree.touch("outer/inner/leaf/m-here");
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    let directory = tree.path("outer/inner/leaf");
    let file = tree.path("outer/inner/leaf/m-here");
    for from in [directory, file] {
        assert_eq!(
            run(
                &ctx,
                &env,
                &format!("(locate-dominating-file {from:?} \"m-here\")")
            ),
            tree.directory("outer/inner/leaf"),
        );
    }
}

#[test]
fn no_directory_holding_a_name_is_nil_and_so_is_an_empty_list_of_names() {
    let tree = Tree::new();
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    let from = tree.path("outer");
    assert!(
        run(
            &ctx,
            &env,
            &format!("(locate-dominating-file {from:?} \"m-nowhere\")")
        )
        .is_nil()
    );
    assert!(
        run(
            &ctx,
            &env,
            &format!("(locate-dominating-file {from:?} nil)")
        )
        .is_nil()
    );
}

#[test]
fn a_nil_directory_is_the_working_directory() {
    let tree = Tree::new();
    tree.touch("outer/m-cwd");
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    run(&ctx, &env, &format!("(cd {:?})", tree.path("outer/inner")));
    assert_eq!(
        run(&ctx, &env, "(locate-dominating-file nil \"m-cwd\")"),
        tree.directory("outer"),
    );
}

#[test]
fn an_answer_is_remembered_until_it_is_forgotten() {
    let tree = Tree::new();
    tree.touch("m-kept");
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    let ask = format!(
        "(locate-dominating-file {:?} \"m-kept\")",
        tree.path("outer/inner")
    );
    assert_eq!(run(&ctx, &env, &ask), tree.directory(""));
    tree.touch("outer/m-kept");
    assert_eq!(run(&ctx, &env, &ask), tree.directory(""));
    assert_eq!(
        run(&ctx, &env, "(locate-dominating-file-forget)"),
        ELispExp::number(1.0)
    );
    assert_eq!(run(&ctx, &env, &ask), tree.directory("outer"));
}

#[test]
fn forgetting_a_directory_forgets_only_the_searches_that_started_under_it() {
    let tree = Tree::new();
    tree.touch("m-part");
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    let inside = format!(
        "(locate-dominating-file {:?} \"m-part\")",
        tree.path("outer/inner")
    );
    let outside = format!("(locate-dominating-file {:?} \"m-part\")", tree.path(""));
    run(&ctx, &env, &inside);
    run(&ctx, &env, &outside);
    tree.touch("outer/m-part");
    let forget = format!("(locate-dominating-file-forget {:?})", tree.path("outer"));
    assert_eq!(run(&ctx, &env, &forget), ELispExp::number(1.0));
    assert_eq!(run(&ctx, &env, &inside), tree.directory("outer"));
    assert_eq!(run(&ctx, &env, &forget), ELispExp::number(1.0));
    assert_eq!(
        run(&ctx, &env, "(locate-dominating-file-forget)"),
        ELispExp::number(1.0)
    );
}

#[test]
fn an_absolute_or_empty_name_and_an_unknown_which_are_refused() {
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    for form in [
        "(locate-dominating-file \".\" \"\")",
        "(locate-dominating-file \".\" '(\"/etc\"))",
        "(locate-dominating-file \".\" 7)",
        "(locate-dominating-file \".\" \"m\" 'lowest)",
    ] {
        let parsed = Parser::new(form).next().unwrap();
        assert!(eval(&parsed, env.clone(), &ctx).is_err(), "{form}");
    }
}

#[test]
fn a_disk_that_does_not_answer_costs_one_wait_and_its_answer_is_kept_when_it_comes() {
    let tree = Tree::new();
    tree.touch("m-slow");
    let (ctx, _env) = create_global_env::<GapBuffer>().unwrap();
    let question = tree.question("outer", "m-slow");
    let (release, disk) = stalled();
    assert_eq!(
        ctx.locate(question.clone(), Duration::from_millis(20), disk),
        Located::Unanswered
    );
    // The same question again neither waits nor asks the disk.
    let asked = Instant::now();
    assert_eq!(
        ctx.locate(question.clone(), Duration::from_secs(60), |_: &Path| {
            panic!("a second climb was started")
        }),
        Located::Unanswered
    );
    assert!(asked.elapsed() < Duration::from_secs(30));
    drop(release);
    until_no_climb_is_out(&ctx);
    assert_eq!(
        ctx.locate(question, Duration::ZERO, |_: &Path| panic!(
            "it was remembered"
        )),
        Located::Known(Some(crate::paths::as_directory(&tree.path("")))),
    );
}

#[test]
fn no_more_climbs_than_the_limit_are_out_at_once() {
    let tree = Tree::new();
    let (ctx, _env) = create_global_env::<GapBuffer>().unwrap();
    let mut releases = Vec::new();
    for n in 0..CLIMBS {
        let (release, disk) = stalled();
        releases.push(release);
        assert_eq!(
            ctx.locate(
                tree.question("outer", &format!("m-{n}")),
                Duration::from_millis(5),
                disk
            ),
            Located::Unanswered
        );
    }
    assert_eq!(ctx.locate_climbs(), CLIMBS);
    assert_eq!(
        ctx.locate(
            tree.question("outer", "m-one-more"),
            Duration::from_secs(60),
            |_: &Path| { panic!("a climb over the limit was started") }
        ),
        Located::Unanswered
    );
    drop(releases);
    until_no_climb_is_out(&ctx);
    assert_eq!(
        ctx.locate(
            tree.question("outer", "m-one-more"),
            Duration::from_secs(5),
            there
        ),
        Located::Known(None)
    );
}

#[test]
fn the_primitive_answers_the_symbol_timeout_when_the_disk_has_not_answered() {
    let tree = Tree::new();
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    // Every climb is out, so the next question is refused as a slow one is.
    let mut releases = Vec::new();
    for n in 0..CLIMBS {
        let (release, disk) = stalled();
        releases.push(release);
        ctx.locate(
            tree.question("outer", &format!("m-{n}")),
            Duration::from_millis(5),
            disk,
        );
    }
    assert_eq!(
        run(
            &ctx,
            &env,
            &format!("(locate-dominating-file {:?} \"m\")", tree.path("outer"))
        ),
        ELispExp::symbol("timeout".into()),
    );
    drop(releases);
    until_no_climb_is_out(&ctx);
}

#[test]
fn a_climb_that_was_out_during_a_forgetting_does_not_record_its_answer() {
    let tree = Tree::new();
    tree.touch("m-stale");
    let (ctx, _env) = create_global_env::<GapBuffer>().unwrap();
    let question = tree.question("outer", "m-stale");
    let (release, disk) = stalled();
    ctx.locate(question.clone(), Duration::from_millis(5), disk);
    ctx.forget_located(None);
    drop(release);
    until_no_climb_is_out(&ctx);
    tree.touch("outer/m-stale");
    assert_eq!(
        ctx.locate(question, Duration::from_secs(5), there),
        Located::Known(Some(crate::paths::as_directory(&tree.path("outer")))),
    );
}

#[test]
fn the_wait_is_the_setting_in_seconds_and_the_default_for_anything_else() {
    let (_, env) = create_global_env::<GapBuffer>().unwrap();
    assert_eq!(locate::timeout(&env), locate::DEFAULT_TIMEOUT);
    env.set_variable(locate::TIMEOUT.into(), ELispExp::number(1.5));
    assert_eq!(locate::timeout(&env), Duration::from_millis(1500));
    for wrong in [
        ELispExp::nil(),
        ELispExp::number(0.0),
        ELispExp::number(-1.0),
    ] {
        env.set_variable(locate::TIMEOUT.into(), wrong);
        assert_eq!(locate::timeout(&env), locate::DEFAULT_TIMEOUT);
    }
}
