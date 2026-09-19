//! Overlays, and the one thing about them that is hard: surviving an edit.
//!
//! Everything else in this editor that knows a position either recomputes it
//! every frame or is invalidated the moment the text changes. These are the
//! first stored positions that have to move with the text, so most of what is
//! below is the adjustment rules -- worked through case by case, because a
//! highlight over the wrong words is noticed minutes later with nothing to say
//! why.
#[cfg(test)]
mod tests {
    use crate::buffer::overlay::OverlayTable;
    use crate::buffer::{Buffer, gap_buffer::GapBuffer};
    use crate::primitives::edits::{delete_range, insert_text};
    use crate::ui::Face;
    use std::sync::Arc;

    fn table() -> OverlayTable {
        OverlayTable::default()
    }

    /// Add an overlay in the default face and category, returning its id.
    fn add(table: &mut OverlayTable, start: usize, end: usize) -> usize {
        table
            .add(start, end, Face::DEFAULT, 0, Arc::from("test"))
            .expect("a non-empty overlay")
    }

    /// The (start, end) of every overlay, in order.
    fn spans(table: &OverlayTable) -> Vec<(usize, usize)> {
        table.iter().map(|o| (o.start, o.end)).collect()
    }

    // ----------------------------------------------------------------
    // Insertion: the asymmetry that makes "neither end grows"
    // ----------------------------------------------------------------

    #[test]
    fn inserting_before_an_overlay_moves_the_whole_thing() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_insert(0, 3);
        assert_eq!(spans(&table), vec![(8, 13)]);
    }

    #[test]
    fn inserting_at_the_start_leaves_the_text_outside() {
        // `start` moves because the rule is `>=`. A highlighted match stays
        // exactly the text that matched, however much is typed in front of it.
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_insert(5, 3);
        assert_eq!(spans(&table), vec![(8, 13)]);
    }

    #[test]
    fn inserting_at_the_end_leaves_the_text_outside() {
        // `end` does not move because the rule is `>`. The other half of the
        // same asymmetry.
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_insert(10, 3);
        assert_eq!(spans(&table), vec![(5, 10)]);
    }

    #[test]
    fn inserting_strictly_inside_grows_the_overlay() {
        // Text typed into the middle of a highlighted region is part of that
        // region -- which is why only the boundaries are special.
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_insert(7, 3);
        assert_eq!(spans(&table), vec![(5, 13)]);
    }

    #[test]
    fn inserting_after_everything_changes_nothing() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_insert(50, 3);
        assert_eq!(spans(&table), vec![(5, 10)]);
    }

    #[test]
    fn an_insertion_of_nothing_changes_nothing() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_insert(0, 0);
        assert_eq!(spans(&table), vec![(5, 10)]);
    }

    // ----------------------------------------------------------------
    // Deletion
    // ----------------------------------------------------------------

    #[test]
    fn deleting_before_an_overlay_moves_it_back() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_delete(0, 3);
        assert_eq!(spans(&table), vec![(2, 7)]);
    }

    #[test]
    fn deleting_inside_an_overlay_shrinks_it() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_delete(6, 8);
        assert_eq!(spans(&table), vec![(5, 8)]);
    }

    #[test]
    fn deleting_across_the_start_clamps_it() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_delete(3, 7);
        assert_eq!(spans(&table), vec![(3, 6)]);
    }

    #[test]
    fn deleting_across_the_end_clamps_it() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_delete(8, 12);
        assert_eq!(spans(&table), vec![(5, 8)]);
    }

    #[test]
    fn deleting_everything_an_overlay_covered_removes_it() {
        // An overlay over no characters highlights nothing, and keeping it
        // would fill the buffer with entries that cannot be seen.
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_delete(3, 12);
        assert!(spans(&table).is_empty());
    }

    #[test]
    fn deleting_exactly_an_overlay_removes_it() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_delete(5, 10);
        assert!(spans(&table).is_empty());
    }

    #[test]
    fn deleting_after_everything_changes_nothing() {
        let mut table = table();
        add(&mut table, 5, 10);
        table.adjust_for_delete(50, 60);
        assert_eq!(spans(&table), vec![(5, 10)]);
    }

    // ----------------------------------------------------------------
    // The invariant the adjustments rest on
    // ----------------------------------------------------------------

    #[test]
    fn sorted_order_survives_every_adjustment() {
        // Both rules are monotonic, which is why the table never has to be
        // re-sorted -- and why a binary search for the tail is sound.
        let mut table = table();
        add(&mut table, 30, 40);
        add(&mut table, 0, 5);
        add(&mut table, 10, 20);
        assert_eq!(spans(&table), vec![(0, 5), (10, 20), (30, 40)]);

        table.adjust_for_insert(12, 4);
        table.adjust_for_delete(2, 3);
        table.adjust_for_insert(0, 7);

        let starts: Vec<usize> = table.iter().map(|o| o.start).collect();
        let mut sorted = starts.clone();
        sorted.sort();
        assert_eq!(starts, sorted, "the table must stay ordered by start");
    }

    #[test]
    fn several_overlays_are_each_adjusted_by_their_own_position() {
        let mut table = table();
        add(&mut table, 0, 5);
        add(&mut table, 10, 20);
        table.adjust_for_insert(7, 3);
        // The first is entirely before the insertion; the second entirely
        // after it.
        assert_eq!(spans(&table), vec![(0, 5), (13, 23)]);
    }

    #[test]
    fn an_overlay_spanning_the_insertion_point_is_found_even_though_it_starts_before_it() {
        // The case the binary search alone would miss: sorted by `start`, this
        // one sits in the prefix, and only its `end` needs moving.
        let mut table = table();
        add(&mut table, 0, 100);
        add(&mut table, 90, 95);
        table.adjust_for_insert(50, 10);
        assert_eq!(spans(&table), vec![(0, 110), (100, 105)]);
    }

    // ----------------------------------------------------------------
    // Keeping and removing them
    // ----------------------------------------------------------------

    #[test]
    fn an_empty_overlay_is_refused_rather_than_stored() {
        let mut table = table();
        assert!(table.add(5, 5, Face::DEFAULT, 0, Arc::from("test")).is_none());
        assert!(table.is_empty());
    }

    #[test]
    fn a_backwards_span_is_taken_the_right_way_round() {
        let mut table = table();
        table
            .add(10, 5, Face::DEFAULT, 0, Arc::from("test"))
            .expect("a non-empty overlay");
        assert_eq!(spans(&table), vec![(5, 10)]);
    }

    #[test]
    fn an_overlay_can_be_removed_by_its_handle() {
        let mut table = table();
        let id = add(&mut table, 5, 10);
        add(&mut table, 20, 30);
        assert!(table.remove(id));
        assert_eq!(spans(&table), vec![(20, 30)]);
        assert!(!table.remove(id), "and removing it twice is not a success");
    }

    #[test]
    fn a_whole_category_can_be_replaced() {
        // What a producer actually wants: re-running a search means forgetting
        // every match it found last time.
        let mut table = table();
        table.add(0, 5, Face::DEFAULT, 0, Arc::from("isearch")).unwrap();
        table.add(10, 15, Face::DEFAULT, 0, Arc::from("isearch")).unwrap();
        table.add(20, 25, Face::DEFAULT, 0, Arc::from("diagnostics")).unwrap();

        assert_eq!(table.remove_category(Some("isearch")), 2);
        assert_eq!(spans(&table), vec![(20, 25)]);
    }

    #[test]
    fn removing_every_category_clears_the_table() {
        let mut table = table();
        add(&mut table, 0, 5);
        table.add(10, 15, Face::DEFAULT, 0, Arc::from("other")).unwrap();
        assert_eq!(table.remove_category(None), 2);
        assert!(table.is_empty());
    }

    #[test]
    fn an_id_is_never_reused() {
        // A stale handle should name nothing rather than naming whatever took
        // its place.
        let mut table = table();
        let first = add(&mut table, 0, 5);
        table.remove(first);
        let second = add(&mut table, 0, 5);
        assert_ne!(first, second);
    }

    // ----------------------------------------------------------------
    // Asking what is where
    // ----------------------------------------------------------------

    #[test]
    fn an_overlay_covers_its_start_but_not_its_end() {
        let mut table = table();
        add(&mut table, 5, 10);
        assert!(table.at(5).len() == 1, "the start is inside");
        assert!(table.at(9).len() == 1, "and so is the last character");
        assert!(table.at(10).is_empty(), "the end is not");
        assert!(table.at(4).is_empty());
    }

    #[test]
    fn overlapping_finds_anything_that_touches_the_range() {
        let mut table = table();
        add(&mut table, 0, 5);
        add(&mut table, 4, 8);
        add(&mut table, 20, 25);
        let found = table.overlapping(3, 6);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn overlapping_answers_lowest_priority_first() {
        // Draw order: a renderer painting in order leaves the highest on top.
        let mut table = table();
        table.add(0, 10, Face::DEFAULT, 5, Arc::from("high")).unwrap();
        table.add(0, 10, Face::DEFAULT, 1, Arc::from("low")).unwrap();
        let found = table.overlapping(0, 10);
        assert_eq!(&*found[0].category, "low");
        assert_eq!(&*found[1].category, "high");
    }

    #[test]
    fn equal_priorities_are_drawn_oldest_first() {
        let mut table = table();
        let first = table.add(0, 10, Face::DEFAULT, 0, Arc::from("a")).unwrap();
        let second = table.add(0, 10, Face::DEFAULT, 0, Arc::from("b")).unwrap();
        let found = table.overlapping(0, 10);
        assert_eq!(found[0].id, first);
        assert_eq!(found[1].id, second, "the later one covers the earlier");
    }

    // ----------------------------------------------------------------
    // Through the two doors, which is how they are really used
    // ----------------------------------------------------------------

    fn buffer_with(text: &str) -> Buffer<GapBuffer> {
        Buffer::from_text("test", text)
    }

    #[test]
    fn typing_before_a_highlight_carries_it_along() {
        let mut buf = buffer_with("hello world");
        buf.overlays
            .add(6, 11, Face::DEFAULT, 0, Arc::from("test"))
            .unwrap();
        assert!(insert_text(&mut buf, 0, ">> "));
        assert_eq!(spans(&buf.overlays), vec![(9, 14)]);
        // And it still covers the same word.
        let text = buf.text.to_string();
        assert_eq!(&text[9..14], "world");
    }

    #[test]
    fn deleting_the_highlighted_text_removes_the_highlight() {
        let mut buf = buffer_with("hello world");
        buf.overlays
            .add(6, 11, Face::DEFAULT, 0, Arc::from("test"))
            .unwrap();
        assert!(delete_range(&mut buf, 6, 11));
        assert!(buf.overlays.is_empty());
    }

    #[test]
    fn a_read_only_buffer_refuses_the_edit_and_leaves_overlays_alone() {
        // The refusal happens before anything is adjusted, so a buffer that
        // did not change has overlays that did not either.
        let mut buf = buffer_with("hello world");
        buf.read_only = true;
        buf.overlays
            .add(6, 11, Face::DEFAULT, 0, Arc::from("test"))
            .unwrap();
        assert!(!insert_text(&mut buf, 0, ">> "));
        assert_eq!(spans(&buf.overlays), vec![(6, 11)]);
    }

    #[test]
    fn a_buffer_with_no_overlays_pays_nothing() {
        // The guard that matters: this is the state almost every buffer is in
        // on almost every keystroke.
        let mut buf = buffer_with("hello world");
        assert!(insert_text(&mut buf, 0, ">> "));
        assert!(buf.overlays.is_empty());
    }
}
