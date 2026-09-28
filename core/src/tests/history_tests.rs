//! Minibuffer history: what a prompt remembers, and walking through it.
//!
//! # What is worth testing here
//!
//! Recall is a feature whose failures are all quiet. A history that keys on
//! the wrong thing still works -- it just mixes two prompts' entries, which
//! looks like a bug in whichever one you noticed second. A walk that forgets
//! what was half-typed still works, until the one time you wanted it back. And
//! a walk whose position survives the prompt it was walking in works perfectly
//! right up to the next prompt, where the first `M-p' continues somebody
//! else's journey.
//!
//! So these check the boundaries rather than the happy path: which ring an
//! entry lands in, what happens at each end of a ring, and what a prompt
//! closing does to a walk in progress.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        (ctx, env)
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

    /// Open a prompt, type TEXT into it, and press Return.
    fn answer(prompt: &str, text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        open(prompt, env, ctx);
        type_in(text, ctx);
        run("(minibuffer-confirm)", env, ctx);
    }

    fn open(prompt: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(r#"(minibuffer-read "{prompt}" nil nil nil)"#),
            env,
            ctx,
        );
    }

    /// Put TEXT in the prompt the way typing would, so the buffer is in the
    /// state a recall has to cope with.
    fn type_in(text: &str, ctx: &Ctx) {
        ctx.with_current_buffer_mut(|buf| {
            for c in text.chars() {
                buf.text.insert(c);
            }
        });
    }

    /// What is in the prompt now.
    fn contents(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|buf| buf.text.to_string())
    }

    fn strings(values: &LispExp<Ctx>) -> Vec<String> {
        values
            .iter()
            .filter_map(|item| match item {
                LispExp::String(text) => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    // ----------------------------------------------------------------
    // What is remembered
    // ----------------------------------------------------------------

    #[test]
    fn a_prompt_remembers_what_was_typed_into_it() {
        let (ctx, env) = editor();
        answer("Eval:", "(+ 1 2)", &env, &ctx);
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Eval:")"#, &env, &ctx)),
            vec!["(+ 1 2)"]
        );
    }

    #[test]
    fn the_newest_entry_comes_first() {
        let (ctx, env) = editor();
        answer("Eval:", "first", &env, &ctx);
        answer("Eval:", "second", &env, &ctx);
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Eval:")"#, &env, &ctx)),
            vec!["second", "first"]
        );
    }

    #[test]
    fn two_prompts_keep_separate_histories() {
        // The point of keying on the prompt at all. Sharing one ring would
        // offer file names at an eval prompt, which is worse than no history:
        // it is history that is confidently wrong.
        let (ctx, env) = editor();
        answer("Eval:", "(+ 1 2)", &env, &ctx);
        answer("Find file:", "/tmp/x", &env, &ctx);
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Eval:")"#, &env, &ctx)),
            vec!["(+ 1 2)"]
        );
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Find file:")"#, &env, &ctx)),
            vec!["/tmp/x"]
        );
    }

    #[test]
    fn a_prompt_may_name_the_ring_it_shares() {
        // Two prompts worded differently that are the same question -- and the
        // reverse, which is why the default is not the only option.
        let (ctx, env) = editor();
        run(
            r#"(minibuffer-read "Open file:" nil nil nil nil "files")"#,
            &env,
            &ctx,
        );
        type_in("/tmp/a", &ctx);
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(
            strings(&run(r#"(minibuffer-history "files")"#, &env, &ctx)),
            vec!["/tmp/a"]
        );
        assert!(
            run(r#"(minibuffer-history "Open file:")"#, &env, &ctx).is_nil(),
            "nothing was filed under the wording"
        );
    }

    #[test]
    fn an_empty_answer_is_not_remembered() {
        // Answering a prompt with nothing is how you back out of it, and a
        // history full of blanks is one you have to walk past.
        let (ctx, env) = editor();
        answer("Eval:", "", &env, &ctx);
        assert!(run(r#"(minibuffer-history "Eval:")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn repeating_the_newest_entry_does_not_add_a_second_copy() {
        // Running the same thing four times should leave one entry, or walking
        // back through the history means four presses to move one step.
        let (ctx, env) = editor();
        answer("Eval:", "same", &env, &ctx);
        answer("Eval:", "same", &env, &ctx);
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Eval:")"#, &env, &ctx)),
            vec!["same"]
        );
    }

    #[test]
    fn a_repeat_further_back_is_left_where_it_is() {
        // Hoisting it would reorder the history under the user, so a sequence
        // they had walked through once would not be there the next time.
        let (ctx, env) = editor();
        answer("Eval:", "a", &env, &ctx);
        answer("Eval:", "b", &env, &ctx);
        answer("Eval:", "a", &env, &ctx);
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Eval:")"#, &env, &ctx)),
            vec!["a", "b", "a"]
        );
    }

    #[test]
    fn a_cancelled_prompt_is_not_remembered() {
        let (ctx, env) = editor();
        open("Eval:", &env, &ctx);
        type_in("abandoned", &ctx);
        run("(minibuffer-cancel)", &env, &ctx);
        assert!(run(r#"(minibuffer-history "Eval:")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn a_ring_is_capped_at_history_length() {
        let (ctx, env) = editor();
        run("(setq history-length 2)", &env, &ctx);
        answer("Eval:", "one", &env, &ctx);
        answer("Eval:", "two", &env, &ctx);
        answer("Eval:", "three", &env, &ctx);
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Eval:")"#, &env, &ctx)),
            vec!["three", "two"]
        );
    }

    // ----------------------------------------------------------------
    // Walking through it
    // ----------------------------------------------------------------

    #[test]
    fn history_previous_puts_the_last_answer_back_in_the_prompt() {
        let (ctx, env) = editor();
        answer("Eval:", "(+ 1 2)", &env, &ctx);
        open("Eval:", &env, &ctx);
        run("(history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "(+ 1 2)");
    }

    #[test]
    fn repeated_steps_walk_further_back() {
        let (ctx, env) = editor();
        answer("Eval:", "oldest", &env, &ctx);
        answer("Eval:", "middle", &env, &ctx);
        answer("Eval:", "newest", &env, &ctx);
        open("Eval:", &env, &ctx);
        run("(history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "newest");
        run("(history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "middle");
        run("(history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "oldest");
    }

    #[test]
    fn the_oldest_entry_is_where_walking_back_stops() {
        // Left exactly as it is rather than rewritten with a copy of itself:
        // a step that redraws the same text is indistinguishable from one
        // that moved, and "you are at the end" is the one thing the key has
        // to be able to say.
        let (ctx, env) = editor();
        answer("Eval:", "only", &env, &ctx);
        open("Eval:", &env, &ctx);
        assert_eq!(run("(history-previous)", &env, &ctx), LispExp::t());
        assert!(run("(history-previous)", &env, &ctx).is_nil());
        assert_eq!(contents(&ctx), "only");
    }

    #[test]
    fn history_next_walks_back_towards_the_newest() {
        let (ctx, env) = editor();
        answer("Eval:", "older", &env, &ctx);
        answer("Eval:", "newer", &env, &ctx);
        open("Eval:", &env, &ctx);
        run("(history-previous) (history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "older");
        run("(history-next)", &env, &ctx);
        assert_eq!(contents(&ctx), "newer");
    }

    #[test]
    fn walking_forward_past_the_newest_restores_what_was_half_typed() {
        // Looking through the history has to be undoable, or the way back to
        // what you were writing is to cancel the prompt and start again.
        let (ctx, env) = editor();
        answer("Eval:", "remembered", &env, &ctx);
        open("Eval:", &env, &ctx);
        type_in("half-ty", &ctx);
        run("(history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "remembered");
        run("(history-next)", &env, &ctx);
        assert_eq!(contents(&ctx), "half-ty");
    }

    #[test]
    fn the_stash_survives_several_steps_back() {
        // Only the *first* step stashes. A later one would stash the entry
        // being shown, and walking forward would hand back a history entry
        // dressed up as what you had typed.
        let (ctx, env) = editor();
        answer("Eval:", "one", &env, &ctx);
        answer("Eval:", "two", &env, &ctx);
        open("Eval:", &env, &ctx);
        type_in("mine", &ctx);
        run("(history-previous) (history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "one");
        run("(history-next) (history-next)", &env, &ctx);
        assert_eq!(contents(&ctx), "mine");
    }

    #[test]
    fn history_next_does_nothing_when_no_walk_is_in_progress() {
        let (ctx, env) = editor();
        answer("Eval:", "something", &env, &ctx);
        open("Eval:", &env, &ctx);
        type_in("fresh", &ctx);
        assert!(run("(history-next)", &env, &ctx).is_nil());
        assert_eq!(contents(&ctx), "fresh");
    }

    #[test]
    fn a_prompt_with_no_history_does_nothing() {
        let (ctx, env) = editor();
        open("Never used:", &env, &ctx);
        assert!(run("(history-previous)", &env, &ctx).is_nil());
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn a_walk_does_not_survive_the_prompt_it_was_walking_in() {
        // The position means nothing once the line it was walking through is
        // gone. Left behind, the next prompt's first step would continue this
        // one's journey -- from the middle of a ring, with a stash of text
        // nobody can see.
        let (ctx, env) = editor();
        answer("Eval:", "one", &env, &ctx);
        answer("Eval:", "two", &env, &ctx);
        open("Eval:", &env, &ctx);
        run("(history-previous) (history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "one");
        run("(minibuffer-cancel)", &env, &ctx);

        open("Eval:", &env, &ctx);
        run("(history-previous)", &env, &ctx);
        assert_eq!(contents(&ctx), "two", "the walk starts again at the newest");
    }

    #[test]
    fn answering_a_prompt_adds_to_the_ring_being_walked() {
        let (ctx, env) = editor();
        answer("Eval:", "old", &env, &ctx);
        open("Eval:", &env, &ctx);
        run("(history-previous)", &env, &ctx);
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Eval:")"#, &env, &ctx)),
            vec!["old"],
            "recalling and confirming the same text adds no second copy"
        );
    }

    // ----------------------------------------------------------------
    // Forgetting
    // ----------------------------------------------------------------

    #[test]
    fn a_ring_can_be_cleared() {
        let (ctx, env) = editor();
        answer("Eval:", "secret", &env, &ctx);
        assert_eq!(
            run(r#"(clear-minibuffer-history "Eval:")"#, &env, &ctx),
            LispExp::t()
        );
        assert!(run(r#"(minibuffer-history "Eval:")"#, &env, &ctx).is_nil());
        assert!(
            run(r#"(clear-minibuffer-history "Eval:")"#, &env, &ctx).is_nil(),
            "clearing an empty ring says there was nothing"
        );
    }

    #[test]
    fn the_recall_commands_are_registered_so_they_can_be_rebound() {
        let (ctx, env) = editor();
        assert_eq!(
            run("(commandp 'history-previous)", &env, &ctx),
            LispExp::t()
        );
        assert_eq!(run("(commandp 'history-next)", &env, &ctx), LispExp::t());
    }
}
