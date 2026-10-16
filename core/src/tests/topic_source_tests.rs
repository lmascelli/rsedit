//! Where the editor's topics are emitted: each kind of change, at the one
//! place it goes through.
//!
//! A job is woken only by what is emitted, so a change that emits nothing is a
//! change the job never sees -- silently, until the screen shows text that
//! never got its colour. These tests count what each source the design names
//! emits, and one follows an edit all the way to the job it wakes. See
//! `doc/workmanager.typ`, "Gli eventi".
#[cfg(test)]
mod tests {
    use crate::buffer::{Buffer, BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{BUFFER_CHANGED, EditorState, SYNTAX_CHANGED, create_global_env};
    use crate::feature::minibuffer::set_minibuffer_content;
    use crate::managers::Buffers;
    use crate::primitives::edits::insert_text;
    use crate::work::{Job, JobError, Spec, Step, StepContext, Topic, Wake};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::{Arc, Mutex};

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env")
    }

    /// An editor whose current buffer holds TEXT, point at its end.
    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor();
        // The view of a replace asks how big the frame is.
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        ctx.with_current_buffer_mut(|buf| {
            buf.text = GapBuffer::from(text);
            let end = buf.text.cursor_1d_to_2d(buf.text.len());
            buf.text.cursor_move(end.0, end.1);
        });
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// How many times the editor's TOPIC is emitted while SRC is evaluated.
    fn emitted(topic: &str, src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> u64 {
        let topic = ctx.work().topic(topic);
        let before = topic.count();
        run(src, env, ctx);
        topic.count() - before
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|buf| buf.text.to_string())
    }

    // -- buffer-changed -------------------------------------------------------

    #[test]
    fn each_door_emits_buffer_changed_once() {
        let (ctx, env) = editor_with("");
        assert_eq!(emitted(BUFFER_CHANGED, r#"(insert "abc")"#, &env, &ctx), 1);
        assert_eq!(
            emitted(BUFFER_CHANGED, "(delete-region 0 1)", &env, &ctx),
            1
        );
        assert_eq!(text(&ctx), "bc");
    }

    #[test]
    fn an_edit_that_changes_nothing_emits_nothing() {
        let (ctx, env) = editor_with("abc");
        assert_eq!(emitted(BUFFER_CHANGED, r#"(insert "")"#, &env, &ctx), 0);
        run("(set-buffer-read-only t)", &env, &ctx);
        assert_eq!(
            emitted(BUFFER_CHANGED, r#"(insert "x")"#, &env, &ctx),
            0,
            "a read-only buffer turns the edit away at the door"
        );
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn undo_and_redo_emit_it_as_the_doors_do() {
        // They change the text below the doors, so they say so themselves.
        let (ctx, env) = editor_with("");
        run(r#"(insert "abc") (undo-boundary)"#, &env, &ctx);
        assert_eq!(emitted(BUFFER_CHANGED, "(undo)", &env, &ctx), 1);
        assert_eq!(text(&ctx), "");
        assert_eq!(emitted(BUFFER_CHANGED, "(redo)", &env, &ctx), 1);
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn stepping_back_in_a_replace_emits_it() {
        let (ctx, env) = editor_with("cat cat");
        run(r#"(goto-char 0) (query-replace "cat" "dog")"#, &env, &ctx);
        assert_eq!(
            emitted(BUFFER_CHANGED, "(replace-this)", &env, &ctx),
            2,
            "a replacement is a deletion and an insertion, through both doors"
        );
        assert_eq!(text(&ctx), "dog cat");
        assert_eq!(
            emitted(BUFFER_CHANGED, "(replace-back)", &env, &ctx),
            1,
            "taking it back is one undo, below the doors"
        );
        assert_eq!(text(&ctx), "cat cat");
    }

    #[test]
    fn adopting_new_text_emits_it() {
        // What a revert, a recovery and the file watcher's reload all do.
        let (ctx, _env) = editor_with("old");
        let topic = ctx.work().topic(BUFFER_CHANGED);
        let before = topic.count();
        ctx.with_current_buffer_mut(|buf| buf.adopt_text("new"));
        assert_eq!(topic.count() - before, 1);
    }

    #[test]
    fn a_buffer_joining_the_editor_emits_it() {
        let (ctx, env) = editor();
        assert_eq!(
            emitted(BUFFER_CHANGED, r#"(buffer-create "fresh")"#, &env, &ctx),
            1
        );
        assert_eq!(
            emitted(BUFFER_CHANGED, r#"(buffer-create "fresh")"#, &env, &ctx),
            0,
            "one that is already there is nothing new"
        );
    }

    #[test]
    fn changing_the_major_mode_emits_it_only_when_the_mode_changes() {
        let (ctx, env) = editor();
        run("(make-mode 'probe-mode)", &env, &ctx);
        assert_eq!(
            emitted(BUFFER_CHANGED, "(set-major-mode 'probe-mode)", &env, &ctx),
            1
        );
        assert_eq!(
            emitted(BUFFER_CHANGED, "(set-major-mode 'probe-mode)", &env, &ctx),
            0,
            "the same mode again changes nothing"
        );
    }

    #[test]
    fn rewriting_the_minibuffer_is_an_edit_like_any_other() {
        // It changed the text below the doors and said nothing: the one change
        // to a buffer's text that left the version where it was.
        let (ctx, _env) = editor_with("typed");
        let version = ctx.with_current_buffer(|buf| buf.version);
        let topic = ctx.work().topic(BUFFER_CHANGED);
        let before = topic.count();
        set_minibuffer_content(&ctx, "candidate");
        assert_eq!(text(&ctx), "candidate");
        assert_eq!(topic.count() - before, 1);
        assert_eq!(ctx.with_current_buffer(|buf| buf.version), version + 1);
    }

    #[test]
    fn a_buffer_counts_on_its_own_until_it_joins_a_table() {
        let mut loose = Buffer::<GapBuffer>::new("loose");
        assert!(insert_text(&mut loose, 0, "x"));
        assert_eq!(loose.changed.count(), 1, "a topic of its own, counting");

        let topic = Topic::detached();
        let mut buffers = Buffers::new(topic.clone());
        let before = topic.count();
        buffers.insert("loose", loose);
        assert_eq!(topic.count(), before + 1, "joining is a change");
        // From here on, its edits are the table's.
        let handle = buffers.handle("loose").expect("in the table");
        assert!(insert_text(&mut handle.write().unwrap(), 0, "y"));
        assert_eq!(topic.count(), before + 2);
    }

    // -- syntax-changed -------------------------------------------------------

    #[test]
    fn grammar_and_syntax_table_edits_emit_syntax_changed() {
        let (ctx, env) = editor();
        assert_eq!(
            emitted(SYNTAX_CHANGED, "(make-mode 'probe-mode)", &env, &ctx),
            1
        );
        assert_eq!(
            emitted(
                SYNTAX_CHANGED,
                r#"(add-syntax-rule 'probe-mode "probe" 'keyword)"#,
                &env,
                &ctx
            ),
            1
        );
        assert_eq!(
            emitted(
                SYNTAX_CHANGED,
                r#"(set-syntax-pairs 'probe-mode "()")"#,
                &env,
                &ctx
            ),
            1
        );
        assert_eq!(
            emitted(
                SYNTAX_CHANGED,
                r#"(set-syntax-pairs 'no-such-mode "()")"#,
                &env,
                &ctx
            ),
            0,
            "a mode that is not there has nothing to change"
        );
    }

    #[test]
    fn a_syntax_table_edit_leaves_every_colouring_current() {
        // The grammar epoch stamps every colouring. Moving it for a table
        // would recolour every buffer for a change no colouring depends on.
        let (ctx, env) = editor();
        run("(make-mode 'probe-mode)", &env, &ctx);
        let epoch = ctx.grammar_epoch();
        run(r#"(set-syntax-pairs 'probe-mode "()")"#, &env, &ctx);
        assert_eq!(ctx.grammar_epoch(), epoch);
        run(
            r#"(add-syntax-rule 'probe-mode "probe" 'keyword)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            ctx.grammar_epoch(),
            epoch + 1,
            "a grammar edit does move it"
        );
    }

    // -- the whole chain ------------------------------------------------------

    /// A job that waits for its topics, and records why it woke.
    struct Probe(Arc<Mutex<Vec<Wake>>>);

    impl<C: 'static> Job<C> for Probe {
        fn step(&mut self, cx: &mut StepContext<'_, C>) -> Result<Step, JobError> {
            self.0.lock().unwrap().push(cx.woken_by);
            Ok(Step::Wait)
        }
    }

    #[test]
    fn an_edit_wakes_a_job_waiting_for_buffer_changed() {
        // The door, the buffer's topic, the editor's WorkManager, the core's
        // subscription: an edit typed in Lisp wakes a job.
        let (ctx, env) = editor();
        let wakes = Arc::new(Mutex::new(Vec::new()));
        let probe = Probe(wakes.clone());
        ctx.work()
            .submit(Spec::new("probe").on(BUFFER_CHANGED), Box::new(probe))
            .unwrap();
        ctx.work().run_until_idle(&ctx);

        run(r#"(insert "x")"#, &env, &ctx);
        let delay = ctx.work().options().event_delay();
        ctx.work().run_for(&ctx, delay);
        assert_eq!(*wakes.lock().unwrap(), [Wake::Start, Wake::Event]);
    }
}
