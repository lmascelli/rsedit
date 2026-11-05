//! The count of a search's matches, as 3/17 before its prompt: where each
//! match starts, which of them the search is at, a count made at once in a
//! small buffer and on a thread of its own in a large one, and a count of a
//! pattern no longer typed thrown away.
//!
//! Design: `doc/architettura-editor.typ, <conteggio>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::feature::isearch::COUNT_AT_ONCE;
    use crate::feature::minibuffer::PROMPT_BUFFER;
    use crate::text::isearch::{Counting, Direction, Isearch, MatchCount};
    use crate::text::search::{Match, Pattern};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
        });
        (ctx, env)
    }

    /// Types TEXT at the end of the search's prompt, and lets the search
    /// update, as a key would.
    fn type_in(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        ctx.with_buffer_mut(PROMPT_BUFFER, |buf| {
            let end = buf.text.len();
            let (line, col) = buf.text.cursor_1d_to_2d(end);
            buf.text.cursor_move(line, col);
            for c in text.chars() {
                buf.text.insert(c);
            }
        });
        run("(isearch-update)", env, ctx);
    }

    fn search(pattern: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run("(isearch-forward)", env, ctx);
        type_in(pattern, env, ctx);
    }

    /// Makes the calls owed until the echo area says WANTED, or ten seconds
    /// have gone.
    fn until_echo(wanted: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            ctx.run_owed_callbacks(env);
            let echo = ctx.get_echo_message();
            if echo == wanted || Instant::now() > deadline {
                return echo;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // ----------------------------------------------------------------
    // Where the matches start
    // ----------------------------------------------------------------

    #[test]
    fn the_starts_are_in_characters_and_do_not_overlap() {
        let pattern = Pattern::new("aa", false, true).expect("literal");
        assert_eq!(
            pattern.starts("é aa aaa", 10, || true),
            Some((vec![2, 5], false)),
            "aaa holds one match that does not overlap the one before it"
        );
    }

    #[test]
    fn the_starts_stop_at_the_limit_and_say_so() {
        let pattern = Pattern::new("a", false, true).expect("literal");
        assert_eq!(pattern.starts("aaaa", 2, || true), Some((vec![0, 1], true)));
        assert_eq!(pattern.starts("aa", 2, || true), Some((vec![0, 1], false)));
    }

    #[test]
    fn a_count_told_to_stop_gives_nothing() {
        let pattern = Pattern::new("a", false, true).expect("literal");
        assert_eq!(pattern.starts("aaaa", 10, || false), None);
    }

    // ----------------------------------------------------------------
    // The label
    // ----------------------------------------------------------------

    fn counted(starts: Vec<usize>, truncated: bool, found_at: Option<usize>) -> Isearch {
        let mut session = Isearch::new("b".into(), 0, Direction::Forward, false);
        let counting = Counting::new("x", 0);
        let _ = counting.result.set(MatchCount { starts, truncated });
        session.counting = Some(counting);
        session.found = found_at.map(|start| Match {
            start,
            end: start + 1,
            groups: Vec::new(),
        });
        session
    }

    #[test]
    fn the_label_says_which_match_of_how_many() {
        assert_eq!(counted(vec![0, 4, 9], false, Some(4)).count_label(), "2/3 ");
        assert_eq!(counted(vec![0, 4, 9], false, None).count_label(), "?/3 ");
        assert_eq!(counted(Vec::new(), false, None).count_label(), "0/0 ");
    }

    #[test]
    fn a_truncated_count_says_so_and_does_not_guess_past_its_end() {
        assert_eq!(counted(vec![0, 4], true, Some(4)).count_label(), "2/2+ ");
        assert_eq!(counted(vec![0, 4], true, Some(20)).count_label(), "?/2+ ");
    }

    #[test]
    fn no_label_until_the_count_is_made() {
        let mut session = Isearch::new("b".into(), 0, Direction::Forward, false);
        assert_eq!(session.count_label(), "");
        session.counting = Some(Counting::new("x", 0));
        assert_eq!(session.count_label(), "");
    }

    // ----------------------------------------------------------------
    // In a small buffer: at once
    // ----------------------------------------------------------------

    #[test]
    fn the_report_counts_the_matches_and_follows_the_search() {
        let (ctx, env) = editor_with("foo bar foo baz foo");
        search("foo", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), "1/3 I-search: foo");
        run("(isearch-repeat-forward) (isearch-update)", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), "2/3 I-search: foo");
        type_in("z", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), "0/0 Failing I-search: fooz");
    }

    #[test]
    fn a_search_that_has_wrapped_counts_from_the_top() {
        let (ctx, env) = editor_with("foo bar foo");
        ctx.with_buffer_mut("*scratch*", |b| b.text.cursor_move(0, 5));
        search("foo", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), "2/2 I-search: foo");
        run("(isearch-repeat-forward) (isearch-update)", &env, &ctx);
        run("(isearch-repeat-forward) (isearch-update)", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), "1/2 Wrapped I-search: foo");
    }

    #[test]
    fn with_isearch_lazy_count_off_nothing_is_counted() {
        let (ctx, env) = editor_with("foo bar foo");
        run("(setq isearch-lazy-count nil)", &env, &ctx);
        search("foo", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), "I-search: foo");
    }

    // ----------------------------------------------------------------
    // In a large buffer: on a thread of its own
    // ----------------------------------------------------------------

    /// A buffer past [`COUNT_AT_ONCE`], with three foo in it.
    fn large() -> String {
        let filler = "x".repeat(COUNT_AT_ONCE / 2);
        format!("foo{filler}foo{filler}foo")
    }

    #[test]
    fn in_a_large_buffer_the_count_arrives_after_the_key() {
        let (ctx, env) = editor_with(&large());
        search("foo", &env, &ctx);
        assert_eq!(
            ctx.get_echo_message(),
            "I-search: foo",
            "the key draws its report before the count"
        );
        assert_eq!(
            until_echo("1/3 I-search: foo", &env, &ctx),
            "1/3 I-search: foo"
        );
    }

    #[test]
    fn a_count_of_a_pattern_no_longer_typed_is_thrown_away() {
        let (ctx, env) = editor_with(&large());
        search("foo", &env, &ctx);
        type_in("z", &env, &ctx);
        let wanted = "0/0 Failing I-search: fooz";
        assert_eq!(until_echo(wanted, &env, &ctx), wanted);
        // And nothing owed later puts the old count back.
        std::thread::sleep(Duration::from_millis(50));
        ctx.run_owed_callbacks(&env);
        assert_eq!(ctx.get_echo_message(), wanted);
    }

    #[test]
    fn a_count_arriving_after_the_search_ended_changes_nothing() {
        let (ctx, env) = editor_with(&large());
        search("foo", &env, &ctx);
        run("(minibuffer-confirm)", &env, &ctx);
        let after = ctx.get_echo_message();
        std::thread::sleep(Duration::from_millis(50));
        ctx.run_owed_callbacks(&env);
        assert_eq!(ctx.get_echo_message(), after);
    }
}
