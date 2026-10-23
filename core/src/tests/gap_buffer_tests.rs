//! The gap buffer: typing, deleting and moving the gap, on text of any width,
//! and its line index, checked against a scan of the text after every change.
//! What every implementation of `BufferTrait` has to do is checked in
//! `buffer_conformance_tests`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};

    // ----------------------------------------------------------------
    // Typing, deleting and moving the gap
    // ----------------------------------------------------------------

    #[test]
    fn test_initialization() {
        let buf = GapBuffer::default();
        // Empty, with room to type into.
        assert_eq!(buf.to_string(), "");
        assert!(buf.gap_width() > 0);
    }

    #[test]
    fn test_basic_typing_and_unicode() {
        let mut buf = GapBuffer::default();
        buf.insert('R');
        buf.insert('u');
        buf.insert('s');
        buf.insert('t');
        buf.insert(' ');
        buf.insert('🦀'); // four bytes in UTF-8
        buf.insert('ℵ'); // three

        assert_eq!(buf.to_string(), "Rust 🦀ℵ");
    }

    #[test]
    fn test_gap_movement_and_insertion() {
        let mut buf = GapBuffer::default();
        for c in "Hello World".chars() {
            buf.insert(c);
        }

        buf.move_gap(5); // after "Hello"
        buf.insert(',');

        assert_eq!(buf.to_string(), "Hello, World");
    }

    #[test]
    fn test_backspace_with_unicode() {
        let mut buf = GapBuffer::default();
        for c in "Logic 💡".chars() {
            buf.insert(c);
        }

        buf.delete(); // the 💡, one character
        assert_eq!(buf.to_string(), "Logic ");

        buf.move_gap(2); // after "Lo"
        buf.delete();
        assert_eq!(buf.to_string(), "Lgic ");
    }

    #[test]
    fn test_moving_gap_to_extremes() {
        let mut buf = GapBuffer::default();
        for c in "Limit".chars() {
            buf.insert(c);
        }

        buf.move_gap(0);
        buf.insert('>');
        assert_eq!(buf.to_string(), ">Limit");

        let len = buf.to_string().chars().count();
        buf.move_gap(len);
        buf.insert('<');
        assert_eq!(buf.to_string(), ">Limit<");
    }

    #[test]
    fn test_buffer_growth() {
        // 1,200 characters, more than a new buffer has room for: the gap grows
        // on the way.
        let mut buf = GapBuffer::default();
        let long_string = "Specialized Unicode: 🚀💎🌈".repeat(50);

        for c in long_string.chars() {
            buf.insert(c);
        }

        assert_eq!(buf.to_string(), long_string);

        // And the buffer still takes an edit once it has grown.
        buf.move_gap(10);
        buf.insert('!');
        assert!(buf.to_string().contains("Specialize!d"));
    }

    #[test]
    fn test_complex_unicode_ordering() {
        let mut buf = GapBuffer::default();
        let input = "नमस्ते"; // with combining characters
        for c in input.chars() {
            buf.insert(c);
        }

        // Positions count characters, not grapheme clusters, so the gap can
        // sit inside a combining sequence: no character is lost there.
        buf.move_gap(2);
        buf.insert('X');

        let result = buf.to_string();
        assert!(result.contains('X'));
        assert_eq!(result.chars().count(), input.chars().count() + 1);
    }

    #[test]
    fn test_empty_buffer_invariants() {
        let mut buf = GapBuffer::from("");

        assert_eq!(buf.len(), 0);
        assert_eq!(buf.cursor_pos(), (0, 0));
        assert_eq!(buf.cursor_pos_1d(), 0);
        assert_eq!(buf.at(0), None);

        // Nowhere to move, and nothing to delete.
        assert!(!buf.cursor_move_forward());
        assert!(!buf.cursor_move_backward());
        buf.delete();
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_coordinate_translations_and_clamping() {
        // Lines of 3, 4 and 3 characters.
        let buf = GapBuffer::from("abc\ndefg\nhij");

        assert_eq!(buf.cursor_2d_to_1d(0, 0), 0);
        assert_eq!(buf.cursor_2d_to_1d(0, 3), 3); // the first newline
        assert_eq!(buf.cursor_2d_to_1d(1, 0), 4);

        assert_eq!(buf.cursor_1d_to_2d(3), (0, 3));
        assert_eq!(buf.cursor_1d_to_2d(4), (1, 0));

        assert_eq!(
            buf.cursor_2d_to_1d(0, 10),
            3,
            "a column past the end of its line stops at the line's end"
        );
        assert_eq!(
            buf.cursor_2d_to_1d(10, 0),
            buf.len(),
            "a line past the last stops at the end of the text"
        );
    }

    #[test]
    fn test_movement_and_traversal() {
        let mut buf = GapBuffer::from("a\nb\nc");

        let mut steps = 0;
        while buf.cursor_move_forward() {
            steps += 1;
        }
        assert_eq!(steps, 5); // 'a', '\n', 'b', '\n', 'c'
        assert_eq!(buf.cursor_pos_1d(), 5);

        assert!(buf.cursor_move_backward());
        assert_eq!(buf.cursor_pos_1d(), 4);

        buf.cursor_move(1, 1); // after the 'b'
        assert_eq!(buf.cursor_pos_1d(), 3);
        assert_eq!(buf.at(3), Some('\n'));
    }

    #[test]
    fn test_search_mechanisms() {
        let mut buf = GapBuffer::from("hello\nworld\nhello");

        buf.cursor_move(1, 0); // at the start of "world", offset 6

        assert_eq!(buf.find_forward('h'), Some(12));
        assert_eq!(buf.find_backward('h'), Some(0));

        assert_eq!(buf.find_forward('z'), None);
        assert_eq!(buf.find_backward('z'), None);
    }

    #[test]
    fn test_unicode_boundary_safety() {
        // Four bytes each in UTF-8, and one character each here.
        let mut buf = GapBuffer::from("🦀\n🚀");

        assert_eq!(buf.len(), 3);
        assert_eq!(buf.at(0), Some('🦀'));
        assert_eq!(buf.at(2), Some('🚀'));

        buf.cursor_move(0, 0);
        assert_eq!(buf.cursor_pos_1d(), 0);

        buf.cursor_move_forward();
        assert_eq!(buf.cursor_pos_1d(), 1);
        assert_eq!(buf.at(buf.cursor_pos_1d()), Some('\n'));
    }

    #[test]
    fn test_viewport_line_extraction() {
        let text = "Line 0\nLine 1\nLine 2\nLine 3\nLine 4\nLine 5";
        let buf = GapBuffer::from(text);

        assert_eq!(buf.line_count(), 6);

        let viewport = buf.get_lines(2, 4);
        assert_eq!(viewport.len(), 2);
        assert_eq!(viewport[0], "Line 2");
        assert_eq!(viewport[1], "Line 3");

        // Past the end of the text, the lines asked for are there, empty.
        let end_viewport = buf.get_lines(4, 7);
        assert_eq!(end_viewport.len(), 3);
        assert_eq!(end_viewport[0], "Line 4");
        assert_eq!(end_viewport[1], "Line 5");
        assert_eq!(end_viewport[2], "");
    }

    /// Reading the text a character at a time with `at` gives the text,
    /// wherever the gap is: at the gap, the character is the one past it.
    #[test]
    fn reading_a_buffer_character_by_character_matches_its_text() {
        let mut buf = GapBuffer::from("abc");
        buf.move_gap(0);
        buf.insert('X');

        let via_at: String = (0..buf.len())
            .map(|i| buf.at(i).expect("in range"))
            .collect();
        assert_eq!(
            via_at,
            buf.to_string(),
            "at() disagreed with the buffer's text"
        );
        assert_eq!(via_at, "Xabc");

        // And across a mixture of edits and gap positions.
        let mut buf = GapBuffer::from("hello\nworld");
        for target in [0usize, 5, 11, 3, 8] {
            buf.move_gap(target);
            buf.insert('.');
            let via_at: String = (0..buf.len())
                .map(|i| buf.at(i).expect("in range"))
                .collect();
            assert_eq!(
                via_at,
                buf.to_string(),
                "at() disagreed with the gap at {target}"
            );
        }
    }

    /// Clearing takes the same time however much text there is.
    #[test]
    fn clearing_a_large_buffer_is_immediate() {
        let mut buf = GapBuffer::from("a line of text\n".repeat(20_000).as_str());
        assert!(buf.len() > 250_000);

        let start = std::time::Instant::now();
        buf.clear();
        let elapsed = start.elapsed();

        assert_eq!(buf.len(), 0);
        assert_eq!(buf.to_string(), "");
        assert_eq!(buf.line_count(), 1, "an empty buffer still has one line");
        assert_eq!(buf.cursor_pos(), (0, 0));
        // Generous: the point is that it is not proportional to the text.
        assert!(
            elapsed < std::time::Duration::from_millis(100),
            "clearing took {elapsed:?} -- it is still unwinding character by character"
        );
        // The buffer is usable afterwards.
        buf.insert('x');
        assert_eq!(buf.to_string(), "x");
    }

    // ----------------------------------------------------------------
    // The line index
    // ----------------------------------------------------------------

    /// Returns a buffer of LINES numbered lines, with the cursor at the start.
    fn numbered(lines: usize) -> GapBuffer {
        let mut text = String::new();
        for i in 0..lines {
            text.push_str(&format!("line {i}\n"));
        }
        GapBuffer::from(text.as_str())
    }

    #[test]
    fn the_index_survives_construction_and_gap_movement() {
        let mut buf = numbered(200);
        buf.assert_index_valid();
        for target in [0usize, 1, 500, 1400, 7, 1399, 0] {
            buf.move_gap(target.min(buf.len()));
            buf.assert_index_valid();
        }
    }

    #[test]
    fn the_index_survives_typing_and_deleting_newlines() {
        let mut buf = GapBuffer::default();
        for c in "alpha\nbeta\ngamma".chars() {
            buf.insert(c);
            buf.assert_index_valid();
        }
        assert_eq!(buf.line_count(), 3);
        for _ in 0..7 {
            buf.delete();
            buf.assert_index_valid();
        }
        assert_eq!(buf.to_string(), "alpha\nbet");
        assert_eq!(buf.line_count(), 2);
    }

    /// The index stays right through the gap growing, which slides everything
    /// after the cursor to the far end of a larger buffer.
    #[test]
    fn the_index_survives_the_gap_growing() {
        let mut buf = numbered(400);
        buf.move_gap(0);
        for c in "typed at the very start\nwith a newline\n".chars() {
            buf.insert(c);
        }
        buf.assert_index_valid();
        assert_eq!(buf.line_count(), 403);
        assert_eq!(
            buf.get_lines(0, 1),
            vec!["typed at the very start".to_string()]
        );
        assert_eq!(buf.get_lines(2, 3), vec!["line 0".to_string()]);
    }

    /// Random editing, checked against a scan after every operation: what
    /// catches an invariant that breaks only on some particular interleaving
    /// of moves, inserts and deletes.
    #[test]
    fn the_index_survives_arbitrary_editing() {
        let mut buf = numbered(60);
        // A tiny deterministic PRNG: reproducible, and no dev-dependency.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for step in 0..3_000 {
            let len = buf.len();
            match next() % 4 {
                0 => buf.insert(if next() % 5 == 0 { '\n' } else { 'x' }),
                1 => buf.delete(),
                _ => buf.move_gap((next() as usize) % (len + 1)),
            }
            buf.assert_index_valid();
            assert_eq!(
                buf.line_count(),
                buf.to_string().chars().filter(|&c| c == '\n').count() + 1,
                "line_count disagreed with the text at step {step}"
            );
        }
    }

    /// What the functions that read the index return, checked against a plain
    /// scan of the text.
    #[test]
    fn indexed_lookups_agree_with_a_plain_scan() {
        let buf = numbered(50);
        let text = buf.to_string();
        let flat: Vec<char> = text.chars().collect();

        // get_lines, including ranges past the end
        for (a, b) in [(0, 3), (10, 14), (49, 52), (0, 51), (7, 7)] {
            let expected: Vec<String> = {
                let mut v: Vec<String> = text
                    .split('\n')
                    .skip(a)
                    .take(b - a)
                    .map(String::from)
                    .collect();
                while v.len() < b - a {
                    v.push(String::new());
                }
                v
            };
            assert_eq!(buf.get_lines(a, b), expected, "get_lines({a}, {b})");
        }

        // 1d <-> 2d round trips at every position
        for pos in 0..=flat.len() {
            let (line, col) = buf.cursor_1d_to_2d(pos);
            assert_eq!(
                buf.cursor_2d_to_1d(line, col),
                pos,
                "round trip failed at {pos} -> ({line}, {col})"
            );
        }
    }

    /// Reading a viewport 50,000 lines into a large text gives the lines
    /// there, as reading one at the head does. What the seek costs is measured
    /// by the performance report, at `gapbuffer/locality`.
    #[test]
    fn reading_a_viewport_seeks_to_it() {
        const VIEWPORT: usize = 50;
        let small = numbered(1_000);
        let large = numbered(100_000);

        let head = small.get_lines(0, VIEWPORT);
        assert_eq!(head.len(), VIEWPORT);
        assert_eq!(large.get_lines(0, VIEWPORT), head);

        let middle = large.get_lines(50_000, 50_000 + VIEWPORT);
        assert_eq!(middle.len(), VIEWPORT);
        assert_eq!(middle[0], "line 50000");
        assert_eq!(
            middle[VIEWPORT - 1],
            format!("line {}", 50_000 + VIEWPORT - 1)
        );
    }
}
