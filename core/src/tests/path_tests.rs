//! Shaping a file name: making it absolute, and taking it apart.
//!
//! # What these can and cannot check
//!
//! The functions under test are written with `std::path`, so what counts as a
//! separator and what counts as a root come from the platform rather than from
//! a character literal in this repository. That is what makes them right on
//! Windows -- and it is also why the Windows behaviour cannot be *asserted*
//! from here: on a Linux host, `std::path` has Unix semantics and a backslash
//! is an ordinary character in a file name.
//!
//! So these tests check two things. The first is the whole of the logic on the
//! host platform. The second is that nothing has crept back in that only works
//! on one platform: the tests below phrase their expectations with
//! `MAIN_SEPARATOR` where the answer depends on it, so a hard-coded `/` in the
//! implementation would fail them on Windows rather than passing everywhere and
//! being wrong on half.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::primitives::io::{
        as_directory, directory_part, expand_path, expand_path_in, last_component,
        without_trailing_separator,
    };
    use crate::tests::home_guard::guard::HomeAs;
    use std::path::{MAIN_SEPARATOR, Path, PathBuf};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn setup() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// A path written the way the host platform writes one, so a test can name
    /// an absolute path without hard-coding a separator.
    fn abs(parts: &[&str]) -> String {
        let mut path = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
        for part in parts {
            path.push(part);
        }
        path.to_string_lossy().to_string()
    }

    // ---------------- making a path absolute ----------------

    /// The bug in one line. A relative path has fewer components than it looks
    /// like it has, and everything downstream that counts components was
    /// counting the wrong number.
    #[test]
    fn a_relative_path_is_resolved_against_the_working_directory() {
        let here = std::env::current_dir().expect("a working directory");

        assert_eq!(
            expand_path("notes.txt"),
            here.join("notes.txt").to_string_lossy()
        );
        assert_eq!(
            expand_path("src/main.rs"),
            here.join("src/main.rs").to_string_lossy()
        );
    }

    #[test]
    fn a_lone_dot_is_the_directory_it_was_resolved_against() {
        let base = PathBuf::from(abs(&["a", "b"]));
        assert_eq!(expand_path_in(".", &base), abs(&["a", "b"]));
        assert_eq!(expand_path_in("./", &base), abs(&["a", "b"]));
    }

    #[test]
    fn dot_dot_drops_the_component_before_it() {
        let base = PathBuf::from(abs(&["a", "b", "c"]));
        assert_eq!(expand_path_in("..", &base), abs(&["a", "b"]));
        assert_eq!(expand_path_in("../..", &base), abs(&["a"]));
        assert_eq!(expand_path_in("../d", &base), abs(&["a", "b", "d"]));
    }

    #[test]
    fn dots_in_the_middle_of_a_path_are_collapsed_too() {
        assert_eq!(
            expand_path(&abs(&["a", "b", "..", "c", ".", "d"])),
            abs(&["a", "c", "d"])
        );
    }

    /// Above a root there is nothing, so the root is its own parent. Without
    /// this, holding `^` down would build a path made of `..`.
    #[test]
    fn dot_dot_above_the_root_stays_at_the_root() {
        let root = std::path::MAIN_SEPARATOR_STR.to_string();
        assert_eq!(expand_path_in("..", Path::new(&root)), root);
        assert_eq!(expand_path_in("../../..", Path::new(&root)), root);
        assert_eq!(expand_path(&abs(&["a", "..", ".."])), root);
    }

    #[test]
    fn an_absolute_path_ignores_the_base_it_is_given() {
        let base = PathBuf::from(abs(&["somewhere", "else"]));
        let given = abs(&["a", "b"]);
        assert_eq!(expand_path_in(&given, &base), given);
    }

    /// Lexical, not `canonicalize`: the answer is the same whether or not the
    /// path exists, which is what the destination of a rename needs.
    #[test]
    fn a_path_that_does_not_exist_expands_like_any_other() {
        let base = PathBuf::from(abs(&["a", "b"]));
        assert_eq!(
            expand_path_in("no/such/file.txt", &base),
            abs(&["a", "b", "no", "such", "file.txt"])
        );
    }

    // ---------------- taking a path apart ----------------

    #[test]
    fn as_directory_adds_a_separator_only_when_there_is_none() {
        let with = format!("{}{MAIN_SEPARATOR}", abs(&["a", "b"]));
        assert_eq!(as_directory(&abs(&["a", "b"])), with);
        assert_eq!(as_directory(&with), with);
    }

    #[test]
    fn directory_file_name_removes_a_trailing_separator() {
        let with = format!("{}{MAIN_SEPARATOR}", abs(&["a", "b"]));
        assert_eq!(without_trailing_separator(&with), abs(&["a", "b"]));
        assert_eq!(
            without_trailing_separator(&abs(&["a", "b"])),
            abs(&["a", "b"])
        );
    }

    /// A root has nothing to remove: stripping its separator leaves the empty
    /// string, which names nothing at all.
    #[test]
    fn a_root_keeps_its_separator() {
        let root = std::path::MAIN_SEPARATOR_STR.to_string();
        assert_eq!(without_trailing_separator(&root), root);
    }

    #[test]
    fn the_directory_part_is_everything_up_to_the_last_separator() {
        let with = format!("{}{MAIN_SEPARATOR}", abs(&["a", "b"]));
        assert_eq!(directory_part(&abs(&["a", "b", "c"])), Some(with.clone()));
        // A path already naming a directory is its own directory part: this is
        // about the *name*, not about what is on disk.
        assert_eq!(directory_part(&with), Some(with));
    }

    #[test]
    fn a_bare_name_has_no_directory_part() {
        assert_eq!(directory_part("notes.txt"), None);
    }

    #[test]
    fn the_nondirectory_part_is_the_last_component() {
        assert_eq!(last_component(&abs(&["a", "b", "notes.txt"])), "notes.txt");
        assert_eq!(last_component("notes.txt"), "notes.txt");
        assert_eq!(
            last_component(&format!("{}{MAIN_SEPARATOR}", abs(&["a", "b"]))),
            "",
            "a path ending in a separator names no file"
        );
    }

    /// The two compose into "the directory above this one", which is the whole
    /// reason both exist and is what `^` in a listing is built out of.
    #[test]
    fn the_two_compose_into_the_parent_of_a_directory() {
        let child = format!("{}{MAIN_SEPARATOR}", abs(&["a", "b", "c"]));
        let parent = format!("{}{MAIN_SEPARATOR}", abs(&["a", "b"]));

        assert_eq!(
            directory_part(&without_trailing_separator(&child)),
            Some(parent)
        );
    }

    #[test]
    fn the_parent_of_the_root_is_the_root() {
        let root = std::path::MAIN_SEPARATOR_STR.to_string();
        assert_eq!(
            directory_part(&without_trailing_separator(&root)),
            Some(root)
        );
    }

    // ---------------- as Lisp sees them ----------------

    #[test]
    fn expand_file_name_resolves_against_the_directory_it_is_given() {
        let (ctx, env) = setup();
        let base = format!("{}{MAIN_SEPARATOR}", abs(&["a", "b"]));

        assert_eq!(
            run(
                &format!(r#"(expand-file-name "draft.md" "{}")"#, escaped(&base)),
                &env,
                &ctx
            ),
            LispExp::string(abs(&["a", "b", "draft.md"]))
        );
        assert_eq!(
            run(
                &format!(r#"(expand-file-name ".." "{}")"#, escaped(&base)),
                &env,
                &ctx
            ),
            LispExp::string(abs(&["a"]))
        );
    }

    /// Without a second argument it resolves against the working directory,
    /// which is the behaviour every other caller depends on.
    #[test]
    fn expand_file_name_without_a_directory_uses_the_working_directory() {
        let (ctx, env) = setup();
        let here = std::env::current_dir().expect("a working directory");

        assert_eq!(
            run(r#"(expand-file-name "notes.txt")"#, &env, &ctx),
            LispExp::string(here.join("notes.txt").to_string_lossy().to_string())
        );
    }

    #[test]
    fn the_name_shaping_primitives_are_reachable_from_lisp() {
        let (ctx, env) = setup();
        let dir = format!("{}{MAIN_SEPARATOR}", abs(&["a", "b"]));
        let file = abs(&["a", "b", "notes.txt"]);

        assert_eq!(
            run(
                &format!(
                    r#"(file-name-as-directory "{}")"#,
                    escaped(&abs(&["a", "b"]))
                ),
                &env,
                &ctx
            ),
            LispExp::string(dir.clone())
        );
        assert_eq!(
            run(
                &format!(r#"(directory-file-name "{}")"#, escaped(&dir)),
                &env,
                &ctx
            ),
            LispExp::string(abs(&["a", "b"]))
        );
        assert_eq!(
            run(
                &format!(r#"(file-name-directory "{}")"#, escaped(&file)),
                &env,
                &ctx
            ),
            LispExp::string(dir)
        );
        assert_eq!(
            run(
                &format!(r#"(file-name-nondirectory "{}")"#, escaped(&file)),
                &env,
                &ctx
            ),
            LispExp::string("notes.txt".into())
        );
    }

    #[test]
    fn a_name_with_no_directory_part_answers_nil_in_lisp() {
        let (ctx, env) = setup();
        assert_eq!(
            run(r#"(file-name-directory "notes.txt")"#, &env, &ctx),
            LispExp::nil()
        );
    }

    /// These shape a name and must not quietly make it absolute -- a caller
    /// asking for the last component of "a/b" is asking about that string.
    #[test]
    fn the_name_shaping_primitives_leave_a_relative_name_relative() {
        let (ctx, env) = setup();
        assert_eq!(
            run(r#"(file-name-nondirectory "src/main.rs")"#, &env, &ctx),
            LispExp::string("main.rs".into())
        );
        assert_eq!(
            run(r#"(file-name-directory "src/main.rs")"#, &env, &ctx),
            LispExp::string("src/".into())
        );
    }

    /// A base that is itself relative is made absolute before anything is
    /// resolved against it. Otherwise a relative base hands back a relative
    /// answer -- from the one function whose whole job is that what comes out
    /// is absolute -- and the caller is no better off than before.
    #[test]
    fn a_relative_base_is_made_absolute_before_it_is_used() {
        let here = std::env::current_dir().expect("a working directory");

        assert_eq!(
            expand_path_in("notes.txt", Path::new("project")),
            here.join("project").join("notes.txt").to_string_lossy()
        );
        assert_eq!(
            expand_path_in("notes.txt", Path::new("./project")),
            here.join("project").join("notes.txt").to_string_lossy()
        );
    }

    /// Windows sets `USERPROFILE` and frequently not `HOME`. A `~` that stayed
    /// a `~` would make every path built from it name a directory called `~`.
    #[test]
    fn the_home_directory_is_found_through_userprofile_when_home_is_unset() {
        let _guard = HomeAs::vars(None, Some("/windows/home"));

        assert_eq!(
            expand_path("~/notes.txt"),
            abs(&["windows", "home", "notes.txt"])
        );
    }

    /// And `HOME` wins when both are set, so a Unix box with a stray
    /// `USERPROFILE` behaves as it always did.
    #[test]
    fn home_is_preferred_when_both_are_set() {
        let _guard = HomeAs::vars(Some("/unix/home"), Some("/windows/home"));

        assert_eq!(
            expand_path("~/notes.txt"),
            abs(&["unix", "home", "notes.txt"])
        );
    }

    /// With neither set there is no home to expand to, so the `~` is left as
    /// the ordinary character it then is.
    #[test]
    fn a_tilde_with_no_home_anywhere_stays_a_tilde() {
        let _guard = HomeAs::vars(None, None);
        let here = std::env::current_dir().expect("a working directory");

        assert_eq!(
            expand_path("~/notes.txt"),
            here.join("~/notes.txt").to_string_lossy()
        );
    }

    // ---------------- what only Windows can actually check ----------------
    //
    // A backslash is a separator on Windows and an ordinary character in a file
    // name elsewhere, so each of these asserts against the *platform's* answer
    // rather than against a fixed one. On this host they are close to
    // tautologies -- `MAIN_SEPARATOR` is `/` and nothing distinguishes asking
    // from assuming. On Windows they are the difference between working and
    // not, and they fail there if a `/` is ever written back into the code they
    // cover.
    //
    // They are here because that is the most a Linux host can do. Running the
    // suite on Windows is the only thing that verifies any of this.

    #[test]
    fn what_counts_as_a_separator_when_splitting_a_name_comes_from_the_platform() {
        assert_eq!(
            last_component("a\\b"),
            if std::path::is_separator('\\') {
                "b"
            } else {
                "a\\b"
            }
        );
        assert_eq!(
            directory_part("a\\b"),
            if std::path::is_separator('\\') {
                Some("a\\".to_string())
            } else {
                None
            }
        );
    }

    /// Completion splits a part-typed path at its last separator. Hard-code
    /// that as `/` and on Windows every path the user types looks like one
    /// component, so the candidates come from the working directory instead of
    /// from the directory they are in.
    #[test]
    fn completion_splits_a_path_at_a_separator_the_platform_recognises() {
        use crate::primitives::io::split_for_completion;

        let (directory, name) = split_for_completion("a\\b");
        if std::path::is_separator('\\') {
            assert_eq!((directory.as_str(), name), ("a\\", "b"));
        } else {
            assert_eq!((directory.as_str(), name), ("", "a\\b"));
        }
    }

    /// A directory in a listing is marked with the separator, and the mark is
    /// what a user reads to know that Enter will descend.
    #[test]
    fn a_listing_marks_a_directory_with_the_platforms_separator() {
        use crate::primitives::io::directory_entries;

        let sandbox = std::env::temp_dir().join(format!("rsedit-sep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sandbox);
        std::fs::create_dir_all(sandbox.join("inner")).expect("sandbox");
        std::fs::write(sandbox.join("file.txt"), "").expect("a file");

        let entries = directory_entries(&sandbox.to_string_lossy()).expect("listing");
        let _ = std::fs::remove_dir_all(&sandbox);

        assert!(
            entries.contains(&format!("inner{MAIN_SEPARATOR}")),
            "a directory is marked with the platform separator: {entries:?}"
        );
        assert!(entries.contains(&"file.txt".to_string()), "{entries:?}");
    }

    /// Backslashes have to survive the trip through a Lisp string literal, and
    /// the reader turns `\\` into one backslash.
    fn escaped(path: &str) -> String {
        path.replace('\\', "\\\\")
    }
}
