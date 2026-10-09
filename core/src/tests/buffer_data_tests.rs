//! Things hung on a buffer, and the one property that makes them worth having.
//!
//! # What is worth testing here
//!
//! That an attachment lives exactly as long as its buffer. Everything else
//! about this table is ordinary map behaviour; that one property is the reason
//! it exists rather than a global keyed by buffer name, and it is the one that
//! cannot be checked by reading the code -- "the destructor runs" is a claim
//! about what `Drop` does when a `HashMap` entry goes, several types away from
//! anything written here.
//!
//! So there is a test below that attaches a value which *notices* being
//! dropped, kills the buffer, and asks it. That is the difference between this
//! and the raw-pointer version the design started as: a pointer would pass
//! every other test in this file and fail that one silently, by leaking.
//!
//! The rest is about the two ways the old arrangements went wrong: a global
//! keyed by name is wrong after a rename, and a global is still there after
//! the buffer is killed and a new one takes its name.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// Something that says when it is dropped.
    struct Tattletale(Arc<AtomicUsize>);

    impl Drop for Tattletale {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    // ----------------------------------------------------------------
    // The property the table exists for
    // ----------------------------------------------------------------

    #[test]
    fn killing_a_buffer_runs_the_destructor_of_what_was_attached_to_it() {
        // The whole claim. A raw pointer in a box would pass every other test
        // in this file and fail this one by doing nothing at all -- which is
        // to say, by leaking, silently, once per search.
        let dropped = Arc::new(AtomicUsize::new(0));
        let (ctx, env) = editor();
        run(r#"(buffer-create "*holder*")"#, &env, &ctx);
        ctx.with_buffer_mut("*holder*", |buf| {
            buf.put_data("test:tattletale", Tattletale(dropped.clone()));
        });
        assert_eq!(dropped.load(Ordering::SeqCst), 0, "still attached");

        run(r#"(kill-buffer "*holder*")"#, &env, &ctx);
        assert_eq!(
            dropped.load(Ordering::SeqCst),
            1,
            "killing the buffer should have freed what was hung on it"
        );
    }

    #[test]
    fn replacing_an_attachment_frees_the_one_it_replaced() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let (ctx, _env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.put_data("test:tattletale", Tattletale(dropped.clone()));
            buf.put_data("test:tattletale", Tattletale(dropped.clone()));
        });
        assert_eq!(dropped.load(Ordering::SeqCst), 1, "the first one went");
    }

    #[test]
    fn forgetting_an_attachment_frees_it() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let (ctx, _env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.put_data("test:tattletale", Tattletale(dropped.clone()));
            assert!(buf.forget_data("test:tattletale"));
            assert!(!buf.forget_data("test:tattletale"), "only once");
        });
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    // ----------------------------------------------------------------
    // Typed attachments, from Rust
    // ----------------------------------------------------------------

    #[test]
    fn what_goes_in_comes_back_with_its_type() {
        let (ctx, _env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.put_data("test:count", 42usize);
            buf.put_data("test:name", "occur".to_string());
        });
        ctx.with_current_buffer(|buf| {
            assert_eq!(buf.data::<usize>("test:count"), Some(&42));
            assert_eq!(
                buf.data::<String>("test:name").map(String::as_str),
                Some("occur")
            );
        });
    }

    #[test]
    fn asking_for_the_wrong_type_answers_nothing_rather_than_lying() {
        // The reason this is `dyn Any` and not a pointer cast: a wrong guess
        // is a `None`, not undefined behaviour in code reachable from Lisp.
        let (ctx, _env) = editor();
        ctx.with_current_buffer_mut(|buf| buf.put_data("test:count", 42usize));
        ctx.with_current_buffer(|buf| {
            assert_eq!(buf.data::<String>("test:count"), None);
            assert_eq!(buf.data::<usize>("test:count"), Some(&42));
        });
    }

    #[test]
    fn an_attachment_can_be_changed_in_place() {
        let (ctx, _env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.put_data("test:count", 1usize);
            if let Some(count) = buf.data_mut::<usize>("test:count") {
                *count += 1;
            }
        });
        ctx.with_current_buffer(|buf| assert_eq!(buf.data::<usize>("test:count"), Some(&2)));
    }

    #[test]
    fn two_features_attach_to_one_buffer_without_meeting() {
        let (ctx, _env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.put_data("occur:results", 3usize);
            buf.put_data("diff:hunks", "two".to_string());
        });
        ctx.with_current_buffer(|buf| {
            assert_eq!(buf.data::<usize>("occur:results"), Some(&3));
            assert_eq!(
                buf.data::<String>("diff:hunks").map(String::as_str),
                Some("two")
            );
        });
    }

    // ----------------------------------------------------------------
    // Lisp values
    // ----------------------------------------------------------------

    #[test]
    fn a_lisp_value_can_be_put_and_got_back() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(buffer-put 'pattern "TODO")"#, &env, &ctx),
            LispExp::string("TODO".into())
        );
        assert_eq!(
            run("(buffer-get 'pattern)", &env, &ctx),
            LispExp::string("TODO".into())
        );
    }

    #[test]
    fn a_key_nothing_was_put_under_reads_as_nil() {
        let (ctx, env) = editor();
        assert_eq!(
            run("(buffer-get 'nothing-here)", &env, &ctx),
            LispExp::nil()
        );
    }

    #[test]
    fn a_list_survives_the_round_trip_whole() {
        // What the result set will be, in the meantime: entries are lists,
        // and a table that flattened or copied them oddly would be found out
        // much later.
        let (ctx, env) = editor();
        run(r#"(buffer-put 'entries '(("buffer" "x" 1 2)))"#, &env, &ctx);
        assert_eq!(
            run("(car (car (buffer-get 'entries)))", &env, &ctx),
            LispExp::string("buffer".into())
        );
    }

    #[test]
    fn each_buffer_has_its_own() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "*one*") (buffer-create "*two*")
               (buffer-put 'who "first" "*one*")
               (buffer-put 'who "second" "*two*")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run(r#"(buffer-get 'who "*one*")"#, &env, &ctx),
            LispExp::string("first".into())
        );
        assert_eq!(
            run(r#"(buffer-get 'who "*two*")"#, &env, &ctx),
            LispExp::string("second".into())
        );
        assert_eq!(
            run("(buffer-get 'who)", &env, &ctx),
            LispExp::nil(),
            "and the current buffer has none of it"
        );
    }

    #[test]
    fn putting_nil_forgets_rather_than_storing_nil() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-put 'pattern "TODO") (buffer-put 'pattern nil)"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(buffer-get 'pattern)", &env, &ctx), LispExp::nil());
        // And the entry really is gone, not holding a nil.
        ctx.with_current_buffer(|buf| {
            assert!(
                !buf.data.contains_key("lisp:pattern"),
                "the entry should be gone"
            );
        });
    }

    #[test]
    fn a_lisp_key_and_a_rust_key_of_the_same_name_do_not_collide() {
        // A module putting `results' and a Rust feature attaching its own
        // `results' are two different things; the one that looked second
        // would otherwise find a value of a type it never stored.
        let (ctx, env) = editor();
        ctx.with_current_buffer_mut(|buf| buf.put_data("results", 7usize));
        run(r#"(buffer-put 'results "mine")"#, &env, &ctx);
        assert_eq!(
            run("(buffer-get 'results)", &env, &ctx),
            LispExp::string("mine".into())
        );
        ctx.with_current_buffer(|buf| assert_eq!(buf.data::<usize>("results"), Some(&7)));
    }

    // ----------------------------------------------------------------
    // The two ways a global keyed by name goes wrong
    // ----------------------------------------------------------------

    #[test]
    fn what_is_attached_follows_a_rename() {
        // A global keyed by the buffer's name is wrong the instant the buffer
        // is renamed, and nothing says so -- the value is simply not found,
        // under a name nobody thinks to look up.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "*before*") (buffer-put 'who "mine" "*before*")"#,
            &env,
            &ctx,
        );
        ctx.rename_buffer("*before*", "*after*");
        assert_eq!(
            run(r#"(buffer-get 'who "*after*")"#, &env, &ctx),
            LispExp::string("mine".into())
        );
    }

    #[test]
    fn a_new_buffer_of_the_same_name_inherits_nothing() {
        // The other half: a global outlives the buffer it was about, and the
        // next buffer to take that name finds somebody else's state.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "*reused*") (buffer-put 'who "first" "*reused*")
               (kill-buffer "*reused*") (buffer-create "*reused*")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run(r#"(buffer-get 'who "*reused*")"#, &env, &ctx),
            LispExp::nil()
        );
    }

    #[test]
    fn a_buffer_that_does_not_exist_is_reported_rather_than_invented() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(buffer-put 'who "mine" "*no-such*")"#, &env, &ctx),
            LispExp::nil()
        );
        assert_eq!(
            run(r#"(buffer-get 'who "*no-such*")"#, &env, &ctx),
            LispExp::nil()
        );
    }

    #[test]
    fn reverting_a_buffer_leaves_its_attachments_alone() {
        // Deliberately unlike the undo history and the overlays, which
        // `adopt_text' throws away because they describe positions in text
        // that no longer exists. An attachment describes whatever its owner
        // says it does, and only its owner knows whether new text invalidates
        // it.
        let (ctx, _env) = editor();
        ctx.with_current_buffer_mut(|buf| {
            buf.put_data("test:count", 5usize);
            buf.adopt_text("something else entirely");
        });
        ctx.with_current_buffer(|buf| assert_eq!(buf.data::<usize>("test:count"), Some(&5)));
    }
}
