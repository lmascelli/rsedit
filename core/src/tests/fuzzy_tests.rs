//! `fuzzy-filter`: what matches, and in what order.
//!
//! The order is the interesting half. Whether a pattern matches at all is a
//! subsequence test anyone would write the same way; which of several matches
//! comes *first* is the part that decides whether the completion strip feels
//! like it read your mind.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env")
    }

    /// `(fuzzy-filter PATTERN '(candidates...))` as a list of strings.
    fn filter(pattern: &str, candidates: &[&str]) -> Vec<String> {
        let (ctx, env) = editor();
        let quoted: Vec<String> = candidates.iter().map(|c| format!("{c:?}")).collect();
        let src = format!("(fuzzy-filter {pattern:?} '({}))", quoted.join(" "));
        let ast = Parser::new(&src).next().expect("source must parse");
        eval(&ast, env.clone(), &ctx)
            .unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
            .iter()
            .map(|item| match item {
                LispExp::String(s) => s.to_string(),
                other => panic!("expected a string, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn characters_may_be_scattered_so_long_as_they_are_in_order() {
        assert_eq!(
            filter("cap", &["completion-at-point"]),
            vec!["completion-at-point"]
        );
    }

    #[test]
    fn out_of_order_characters_do_not_match() {
        assert!(filter("pac", &["completion-at-point"]).is_empty());
    }

    #[test]
    fn a_character_that_is_not_there_does_not_match() {
        assert!(filter("cazp", &["completion-at-point"]).is_empty());
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(filter("CAP", &["completion-at-point"]).len(), 1);
        assert_eq!(filter("vec", &["Vec"]).len(), 1);
    }

    #[test]
    fn an_empty_pattern_matches_everything_in_the_order_given() {
        // What a freshly-opened strip shows, before anything is typed.
        assert_eq!(filter("", &["one", "two", "three"]), vec!["one", "two", "three"]);
    }

    #[test]
    fn word_starts_beat_a_scattering_of_the_same_letters() {
        // `cap` is an abbreviation of the first, and a coincidence in the
        // second. Both match; only the ordering says which was meant.
        let found = filter("cap", &["chunky-apple-pie-crumble", "completion-at-point"]);
        assert_eq!(found.first().map(String::as_str), Some("completion-at-point"));
    }

    #[test]
    fn a_run_of_adjacent_characters_beats_a_scattered_one() {
        let found = filter("abc", &["a-b-c-x-y-z", "abc"]);
        assert_eq!(found.first().map(String::as_str), Some("abc"));
    }

    #[test]
    fn a_literal_substring_beats_a_fuzzy_match() {
        let found = filter("point", &["previous-indentation-thing", "point-max"]);
        assert_eq!(found.first().map(String::as_str), Some("point-max"));
    }

    #[test]
    fn a_prefix_beats_the_same_text_in_the_middle() {
        let found = filter("buf", &["current-buffer", "buffer-name"]);
        assert_eq!(found.first().map(String::as_str), Some("buffer-name"));
    }

    #[test]
    fn camel_case_counts_as_word_starts_too() {
        let found = filter("gcb", &["getCurrentBuffer", "global-cab-bureau-xyz"]);
        assert_eq!(found.first().map(String::as_str), Some("getCurrentBuffer"));
    }

    #[test]
    fn a_path_separator_starts_a_word() {
        let found = filter("csm", &["core/src/main.rs", "cosmological-summary"]);
        assert_eq!(found.first().map(String::as_str), Some("core/src/main.rs"));
    }

    #[test]
    fn the_shorter_of_two_similar_matches_comes_first() {
        let found = filter("buf", &["buffer-substring-no-properties", "buffer"]);
        assert_eq!(found.first().map(String::as_str), Some("buffer"));
    }

    #[test]
    fn equal_scores_keep_the_order_they_arrived_in() {
        // A source that sorted its own output keeps that order among equals,
        // rather than having it shuffled by an unstable comparison.
        let found = filter("a", &["ax", "ay", "az"]);
        assert_eq!(found, vec!["ax", "ay", "az"]);
    }

    #[test]
    fn a_candidate_keeps_the_shape_it_arrived_in() {
        // Candidates may be (VALUE . DESCRIPTION); the match is against VALUE
        // and the pair comes back whole, so descriptions survive filtering.
        let (ctx, env) = editor();
        let ast = Parser::new(r#"(fuzzy-filter "buf" '(("buffer" . "a thing") "nope"))"#)
            .next()
            .expect("source must parse");
        let answer = eval(&ast, env.clone(), &ctx).expect("fuzzy-filter must not fail");
        let items: Vec<_> = answer.iter().collect();
        assert_eq!(items.len(), 1);
        let LispExp::Cons(cell) = &items[0] else {
            panic!("the pair should have survived, got {:?}", items[0]);
        };
        assert_eq!(cell.car, LispExp::string("buffer".to_string()));
        assert_eq!(cell.cdr, LispExp::string("a thing".to_string()));
    }

    #[test]
    fn nothing_matching_is_an_empty_list_not_an_error() {
        assert!(filter("zzz", &["one", "two"]).is_empty());
    }

    #[test]
    fn a_pattern_longer_than_the_candidate_cannot_match() {
        assert!(filter("abcdef", &["abc"]).is_empty());
    }
}
