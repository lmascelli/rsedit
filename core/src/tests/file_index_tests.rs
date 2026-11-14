//! The index of a tree: the patterns that leave files out, the walk that
//! applies them, and what the editor keeps of a walk and gives out while it
//! goes on.
//!
//! Design: `doc/architettura-editor.typ, <indice-file>`.
use crate::editor::file_index::{self, INDEXES, Rules, Walked, walk};
use crate::editor::ignore::{self, Verdict, parse, parse_line};
use crate::{ELispExp, EditorState, Env, GapBuffer, Parser, create_global_env, eval};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

type Ctx = EditorState<GapBuffer>;

// ---------------------------------------------------------------------------
// The patterns
// ---------------------------------------------------------------------------

/// What the lines of TEXT say of PATH, relative to their directory.
fn says(text: &str, path: &str, directory: bool) -> Option<Verdict> {
    let name = path.rsplit('/').next().unwrap();
    ignore::verdict(&parse(text), path, name, directory)
}

fn excluded(text: &str, path: &str) -> bool {
    says(text, path, false) == Some(Verdict::Excluded)
}

fn excluded_directory(text: &str, path: &str) -> bool {
    says(text, path, true) == Some(Verdict::Excluded)
}

#[test]
fn a_name_without_a_slash_matches_at_any_depth() {
    assert!(excluded("build", "build"));
    assert!(excluded("build", "a/b/build"));
    assert!(excluded_directory("build", "a/build"));
    assert!(!excluded("build", "a/build.rs"));
}

#[test]
fn a_trailing_slash_matches_only_a_directory() {
    assert!(excluded_directory("out/", "out"));
    assert!(excluded_directory("out/", "deep/out"));
    assert!(!excluded("out/", "out"));
}

#[test]
fn a_slash_at_the_start_or_in_the_middle_anchors_the_pattern_to_its_directory() {
    assert!(excluded("/notes", "notes"));
    assert!(!excluded("/notes", "a/notes"));
    assert!(excluded("doc/frotz", "doc/frotz"));
    assert!(!excluded("doc/frotz", "a/doc/frotz"));
}

#[test]
fn a_star_and_a_question_mark_stay_within_one_part_of_the_path() {
    assert!(excluded("*.log", "deep/x.log"));
    assert!(excluded("doc/*.txt", "doc/a.txt"));
    assert!(!excluded("doc/*.txt", "doc/x/a.txt"));
    assert!(excluded("file?.txt", "file1.txt"));
    assert!(!excluded("file?.txt", "file12.txt"));
    assert!(excluded("a*b*c", "aXXbYYc"));
    assert!(!excluded("a*b*c", "aXXbYY"));
}

#[test]
fn two_stars_alone_cross_any_number_of_directories() {
    assert!(excluded("**/gen", "gen"));
    assert!(excluded("**/gen", "a/b/gen"));
    assert!(excluded("src/**/gen", "src/gen"));
    assert!(excluded("src/**/gen", "src/a/b/gen"));
    assert!(!excluded("src/**/gen", "lib/a/gen"));
    assert!(excluded("cache/**", "cache/x"));
    assert!(excluded("cache/**", "cache/x/y"));
    assert!(
        !excluded_directory("cache/**", "cache"),
        "what is inside, not the directory itself, so a `!` can bring a file back"
    );
}

#[test]
fn a_class_matches_one_character_of_its_set_or_outside_it() {
    assert!(excluded("[abc]x", "bx"));
    assert!(excluded("[a-c]x", "cx"));
    assert!(!excluded("[a-c]x", "dx"));
    assert!(excluded("[!a-c]x", "dx"));
    assert!(excluded("[^a-c]x", "dx"));
    assert!(!excluded("[!a-c]x", "ax"));
    assert!(excluded("[]]x", "]x"));
    assert!(
        excluded("[x", "[x"),
        "a class never closed is the bracket itself"
    );
}

#[test]
fn the_last_line_that_names_a_path_decides() {
    let lines = "*.log\n!keep.log\n";
    assert_eq!(says(lines, "keep.log", false), Some(Verdict::Included));
    assert_eq!(says(lines, "other.log", false), Some(Verdict::Excluded));
    assert_eq!(says(lines, "notes.txt", false), None);
    assert_eq!(
        says("!keep.log\n*.log", "keep.log", false),
        Some(Verdict::Excluded)
    );
}

#[test]
fn comments_blank_lines_and_line_endings_are_not_patterns() {
    assert!(parse("# a comment\n\n   \n").is_empty());
    assert_eq!(parse("build/\r\n*.log\r\n").len(), 2);
    assert!(excluded("build/\r\n*.log\r\n", "x.log"));
    assert!(
        excluded("x.log   ", "x.log"),
        "spaces after a pattern are dropped"
    );
}

#[test]
fn a_backslash_makes_the_next_character_itself() {
    assert!(excluded("\\#hash", "#hash"));
    assert!(excluded("\\!bang", "!bang"));
    assert!(excluded("star\\*", "star*"));
    assert!(!excluded("star\\*", "starry"));
    assert!(excluded("space\\ ", "space "));
}

#[test]
fn a_pattern_that_is_not_understood_names_nothing() {
    assert_eq!(parse_line("[[:digit:]]*.log"), None);
    assert_eq!(parse_line("/"), None);
    assert_eq!(parse_line("!"), None);
}

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

/// A tree of its own under the temporary directory, removed at the end.
struct Tree(PathBuf);

impl Tree {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rsedit-index-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn file(&self, relative: &str, contents: &str) -> &Self {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
        self
    }

    fn path(&self, relative: &str) -> String {
        self.0.join(relative).to_string_lossy().into()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rules(exclude: &[&str], ignore_files: &[&str], limit: usize) -> Rules {
    Rules {
        exclude: exclude.iter().map(|line| line.to_string()).collect(),
        ignore_files: ignore_files.iter().map(|name| name.to_string()).collect(),
        limit,
    }
}

/// The walk of TREE under RULES, to the end: its files and how it ended.
fn walked(tree: &Tree, rules: &Rules) -> (Vec<String>, Walked) {
    let (paths, how) = walk(&tree.0, rules, &|| true, &mut |_| {});
    (paths.iter().map(|path| path.to_string()).collect(), how)
}

#[test]
fn the_walk_finds_every_file_at_every_depth_sorted_and_relative() {
    let tree = Tree::new();
    tree.file("top.txt", "")
        .file("a/one.txt", "")
        .file("a/b/two.txt", "");
    assert_eq!(
        walked(&tree, &rules(&[], &[], 100)),
        (
            vec!["a/b/two.txt".into(), "a/one.txt".into(), "top.txt".into()],
            Walked::Complete
        )
    );
}

#[test]
fn the_given_lines_leave_out_files_and_directories_at_every_depth() {
    let tree = Tree::new();
    tree.file("keep.rs", "")
        .file("drop.o", "")
        .file("deep/also.o", "")
        .file("target/debug/thing", "")
        .file("deep/target/x", "");
    assert_eq!(
        walked(&tree, &rules(&["target/", "*.o"], &[], 100)).0,
        vec!["keep.rs".to_string()]
    );
}

#[test]
fn an_ignore_file_counts_in_its_directory_and_under_it_only() {
    let tree = Tree::new();
    tree.file("a.tmp", "")
        .file("sub/.gitignore", "*.tmp\n")
        .file("sub/b.tmp", "")
        .file("sub/deeper/c.tmp", "")
        .file("sub/keep.txt", "");
    assert_eq!(
        walked(&tree, &rules(&[], &[".gitignore"], 100)).0,
        vec!["a.tmp", "sub/.gitignore", "sub/keep.txt"]
    );
}

#[test]
fn an_anchored_line_in_a_deeper_ignore_file_is_read_from_its_own_directory() {
    let tree = Tree::new();
    tree.file("gen/top.txt", "")
        .file("sub/.gitignore", "/gen\n")
        .file("sub/gen/x.txt", "")
        .file("sub/deeper/gen/y.txt", "");
    assert_eq!(
        walked(&tree, &rules(&[], &[".gitignore"], 100)).0,
        vec!["gen/top.txt", "sub/.gitignore", "sub/deeper/gen/y.txt"]
    );
}

#[test]
fn a_deeper_ignore_file_brings_back_what_a_shallower_one_left_out() {
    let tree = Tree::new();
    tree.file(".gitignore", "*.log\n")
        .file("noise.log", "")
        .file("sub/.gitignore", "!keep.log\n")
        .file("sub/keep.log", "")
        .file("sub/other.log", "");
    assert_eq!(
        walked(&tree, &rules(&[], &[".gitignore"], 100)).0,
        vec![".gitignore", "sub/.gitignore", "sub/keep.log"]
    );
}

#[test]
fn the_ignore_files_win_over_the_given_lines() {
    let tree = Tree::new();
    tree.file(".gitignore", "!README.md\n")
        .file("README.md", "")
        .file("notes.md", "");
    assert_eq!(
        walked(&tree, &rules(&["*.md"], &[".gitignore"], 100)).0,
        vec![".gitignore", "README.md"]
    );
}

#[test]
fn a_directory_left_out_is_not_entered_and_nothing_under_it_comes_back() {
    let tree = Tree::new();
    tree.file(".gitignore", "build/\n!build/keep.txt\n")
        .file("build/keep.txt", "")
        .file("src.rs", "");
    assert_eq!(
        walked(&tree, &rules(&[], &[".gitignore"], 100)).0,
        vec![".gitignore", "src.rs"]
    );
}

#[test]
fn an_ignore_file_is_read_only_under_a_name_it_is_given() {
    let tree = Tree::new();
    tree.file(".gitignore", "*.log\n").file("x.log", "");
    assert_eq!(
        walked(&tree, &rules(&[], &[], 100)).0,
        vec![".gitignore", "x.log"]
    );
    tree.file(".ignore", "*.log\n");
    assert_eq!(
        walked(&tree, &rules(&[], &[".ignore"], 100)).0,
        vec![".gitignore", ".ignore"]
    );
}

#[test]
fn the_limit_stops_the_walk_and_files_left_out_do_not_count() {
    let tree = Tree::new();
    for n in 0..20 {
        tree.file(&format!("junk{n}.o"), "");
    }
    for n in 0..3 {
        tree.file(&format!("src{n}.rs"), "");
    }
    assert_eq!(
        walked(&tree, &rules(&["*.o"], &[], 3)),
        (
            vec!["src0.rs".into(), "src1.rs".into(), "src2.rs".into()],
            Walked::Complete
        )
    );
    let (paths, how) = walked(&tree, &rules(&["*.o"], &[], 2));
    assert_eq!((paths.len(), how), (2, Walked::Truncated));
}

#[test]
fn what_is_found_is_shown_in_batches_before_the_walk_ends() {
    let tree = Tree::new();
    for d in 0..30 {
        for f in 0..40 {
            tree.file(&format!("d{d:02}/f{f:02}.txt"), "");
        }
    }
    let mut batches: Vec<usize> = Vec::new();
    let (paths, how) = walk(&tree.0, &rules(&[], &[], 10_000), &|| true, &mut |batch| {
        batches.push(batch.len())
    });
    assert_eq!((paths.len(), how), (1200, Walked::Complete));
    assert!(
        batches.len() >= 2,
        "one batch is the whole walk: {batches:?}"
    );
    assert_eq!(batches.iter().sum::<usize>(), 1200);
}

#[test]
fn a_walk_asked_to_stop_stops_at_the_next_directory() {
    let tree = Tree::new();
    tree.file("a/1", "").file("b/2", "").file("c/3", "");
    let asked = AtomicUsize::new(0);
    let (_, how) = walk(
        &tree.0,
        &rules(&[], &[], 100),
        &|| asked.fetch_add(1, Ordering::Relaxed) < 2,
        &mut |_| {},
    );
    assert_eq!(how, Walked::Stopped);
    assert_eq!(asked.load(Ordering::Relaxed), 3);
}

#[test]
fn a_root_that_is_not_a_directory_is_said_to_be_one() {
    let tree = Tree::new();
    tree.file("plain", "");
    let (paths, how) = walk(
        &tree.0.join("plain"),
        &rules(&[], &[], 100),
        &|| true,
        &mut |_| {},
    );
    assert_eq!((paths.len(), how), (0, Walked::NotADirectory));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_listed_and_not_followed() {
    let tree = Tree::new();
    tree.file("a/real.txt", "");
    std::os::unix::fs::symlink(&tree.0, tree.0.join("a/loop")).unwrap();
    assert_eq!(
        walked(&tree, &rules(&[], &[], 100)).0,
        vec!["a/loop", "a/real.txt"]
    );
}

// ---------------------------------------------------------------------------
// What the editor keeps
// ---------------------------------------------------------------------------

fn run(ctx: &Ctx, env: &Arc<Env<Ctx>>, source: &str) -> ELispExp<GapBuffer> {
    eval(&Parser::new(source).next().unwrap(), env.clone(), ctx)
        .unwrap_or_else(|why| panic!("{source}: {why:?}"))
}

/// The STATE and the PATHS of a `file-index` answer.
fn answer(exp: &ELispExp<GapBuffer>) -> (String, Vec<String>) {
    let mut parts = exp.iter();
    let state = match parts.next() {
        Some(ELispExp::Symbol(state)) => state.to_string(),
        other => panic!("a state, not {other:?}"),
    };
    let paths = parts
        .next()
        .expect("a list of paths")
        .iter()
        .map(|path| match path {
            ELispExp::String(path) => path.to_string(),
            other => panic!("a path, not {other:?}"),
        })
        .collect();
    (state, paths)
}

/// Asks SOURCE again until no walk is out, and returns the last answer.
fn settled(ctx: &Ctx, env: &Arc<Env<Ctx>>, source: &str) -> (String, Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let found = answer(&run(ctx, env, source));
        if found.0 != "walking" && ctx.file_index_walks() == 0 {
            return found;
        }
        assert!(Instant::now() < deadline, "a walk did not come back");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn until_no_walk_is_out(ctx: &Ctx) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while ctx.file_index_walks() > 0 {
        assert!(Instant::now() < deadline, "a walk did not come back");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn editor() -> (Ctx, Arc<Env<Ctx>>) {
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    run(&ctx, &env, "(setq file-index-max-age nil)");
    (ctx, env)
}

#[test]
fn the_index_ends_complete_with_every_file_the_rules_keep() {
    let tree = Tree::new();
    tree.file(".gitignore", "*.log\n")
        .file("a/x.rs", "")
        .file("a/x.log", "")
        .file(".git/HEAD", "");
    let (ctx, env) = editor();
    let ask = format!(
        "(file-index {:?} '(\".git/\") '(\".gitignore\"))",
        tree.path("")
    );
    assert_eq!(
        settled(&ctx, &env, &ask),
        (
            "complete".into(),
            vec![".gitignore".into(), "a/x.rs".into()]
        )
    );
}

#[test]
fn an_index_stopped_at_its_limit_is_truncated() {
    let tree = Tree::new();
    for n in 0..5 {
        tree.file(&format!("f{n}"), "");
    }
    let (ctx, env) = editor();
    let ask = format!("(file-index {:?} nil nil 3)", tree.path(""));
    let (state, paths) = settled(&ctx, &env, &ask);
    assert_eq!((state.as_str(), paths.len()), ("truncated", 3));
}

#[test]
fn an_index_is_kept_and_does_not_see_a_new_file_until_it_is_forgotten() {
    let tree = Tree::new();
    tree.file("old", "");
    let (ctx, env) = editor();
    let ask = format!("(file-index {:?})", tree.path(""));
    assert_eq!(settled(&ctx, &env, &ask).1, vec!["old"]);
    tree.file("new", "");
    assert_eq!(
        answer(&run(&ctx, &env, &ask)),
        ("complete".into(), vec!["old".into()])
    );
    assert_eq!(ctx.file_index_walks(), 0, "a kept index starts no walk");
    assert_eq!(
        run(
            &ctx,
            &env,
            &format!("(file-index-forget {:?})", tree.path(""))
        ),
        ELispExp::number(1.0)
    );
    assert_eq!(settled(&ctx, &env, &ask).1, vec!["new", "old"]);
}

#[test]
fn an_index_older_than_the_max_age_is_walked_again_behind_the_old_list() {
    let tree = Tree::new();
    tree.file("old", "");
    let (ctx, env) = editor();
    let ask = format!("(file-index {:?})", tree.path(""));
    settled(&ctx, &env, &ask);
    tree.file("new", "");
    run(&ctx, &env, "(setq file-index-max-age 0)");
    let during = answer(&run(&ctx, &env, &ask));
    assert_eq!(during.0, "complete", "a walk again does not show itself");
    until_no_walk_is_out(&ctx);
    run(&ctx, &env, "(setq file-index-max-age nil)");
    assert_eq!(answer(&run(&ctx, &env, &ask)).1, vec!["new", "old"]);
}

#[test]
fn the_same_directory_under_other_rules_is_another_index() {
    let tree = Tree::new();
    tree.file("a.txt", "").file("b.rs", "");
    let (ctx, env) = editor();
    let all = format!("(file-index {:?})", tree.path(""));
    let some = format!("(file-index {:?} '(\"*.txt\"))", tree.path(""));
    assert_eq!(settled(&ctx, &env, &all).1, vec!["a.txt", "b.rs"]);
    assert_eq!(settled(&ctx, &env, &some).1, vec!["b.rs"]);
    assert_eq!(
        run(&ctx, &env, "(file-index-forget)"),
        ELispExp::number(2.0)
    );
}

#[test]
fn forgetting_a_directory_drops_only_the_indexes_in_it_or_under_it() {
    let tree = Tree::new();
    tree.file("in/deep/x", "").file("out/y", "");
    let (ctx, env) = editor();
    for relative in ["in", "in/deep", "out"] {
        settled(
            &ctx,
            &env,
            &format!("(file-index {:?})", tree.path(relative)),
        );
    }
    assert_eq!(
        run(
            &ctx,
            &env,
            &format!("(file-index-forget {:?})", tree.path("in"))
        ),
        ELispExp::number(2.0)
    );
    assert_eq!(
        run(&ctx, &env, "(file-index-forget)"),
        ELispExp::number(1.0)
    );
}

#[test]
fn no_more_indexes_than_the_limit_are_kept() {
    let tree = Tree::new();
    let (ctx, env) = editor();
    for n in 0..INDEXES + 1 {
        tree.file(&format!("d{n}/f"), "");
        settled(
            &ctx,
            &env,
            &format!("(file-index {:?})", tree.path(&format!("d{n}"))),
        );
    }
    assert_eq!(
        run(&ctx, &env, "(file-index-forget)"),
        ELispExp::number(INDEXES as f64)
    );
}

#[test]
fn a_nil_directory_is_the_working_directory() {
    let tree = Tree::new();
    tree.file("here/f", "");
    let (ctx, env) = editor();
    run(&ctx, &env, &format!("(cd {:?})", tree.path("here")));
    assert_eq!(settled(&ctx, &env, "(file-index nil)").1, vec!["f"]);
}

#[test]
fn a_directory_that_cannot_be_read_is_an_empty_complete_index_and_a_log_line() {
    let tree = Tree::new();
    let (ctx, env) = editor();
    let missing = tree.path("missing");
    assert_eq!(
        settled(&ctx, &env, &format!("(file-index {missing:?})")),
        ("complete".into(), vec![])
    );
    assert!(
        ctx.get_logs()
            .iter()
            .any(|line| line.contains("Cannot index") && line.contains("missing"))
    );
}

#[test]
fn arguments_of_the_wrong_shape_are_refused() {
    let (ctx, env) = editor();
    for form in [
        "(file-index)",
        "(file-index 7)",
        "(file-index \".\" \"*.o\")",
        "(file-index \".\" '(7))",
        "(file-index \".\" nil '(nil))",
        "(file-index \".\" nil nil \"many\")",
        "(file-index-forget 7)",
    ] {
        let parsed = Parser::new(form).next().unwrap();
        assert!(eval(&parsed, env.clone(), &ctx).is_err(), "{form}");
    }
}

#[test]
fn asking_costs_a_unit_of_fuel_for_each_path_handed_over_and_not_for_the_patterns() {
    let tree = Tree::new();
    for n in 0..600 {
        tree.file(&format!("d{}/f{n}.rs", n % 20), "");
    }
    let patterns: String = (0..40).map(|k| format!("*.ext{k}\n")).collect();
    tree.file(".gitignore", &patterns);
    let (ctx, env) = editor();
    let ask = format!("(file-index {:?} nil '(\".gitignore\"))", tree.path(""));
    assert_eq!(settled(&ctx, &env, &ask).1.len(), 601);
    let source = Parser::new(&ask).next().unwrap();
    let (_, spent) = risp::measure(&ctx.fuel_meter(), || eval(&source, env.clone(), &ctx));
    assert!(
        (601..1_000).contains(&spent),
        "asking for 601 paths cost {spent} units"
    );
}

#[test]
fn the_max_age_is_the_setting_in_seconds_nil_for_never_and_the_default_otherwise() {
    let (ctx, env) = create_global_env::<GapBuffer>().unwrap();
    assert_eq!(file_index::max_age(&env), Some(file_index::DEFAULT_MAX_AGE));
    run(&ctx, &env, "(setq file-index-max-age 2.5)");
    assert_eq!(file_index::max_age(&env), Some(Duration::from_millis(2500)));
    run(&ctx, &env, "(setq file-index-max-age nil)");
    assert_eq!(file_index::max_age(&env), None);
    run(&ctx, &env, "(setq file-index-max-age \"soon\")");
    assert_eq!(file_index::max_age(&env), Some(file_index::DEFAULT_MAX_AGE));
}
