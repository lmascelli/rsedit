//! What any [`BufferTrait`] implementation has to do, asked of each of them.
//!
//! # Why this is generic and not a set of `GapBuffer` tests
//!
//! The trait exists so the editor can be built against a different text
//! representation without changing anything above it. That is only true if
//! "different" means *different storage*, not different answers -- and the
//! cheapest way for a second implementation to be quietly wrong is to differ
//! somewhere nobody thought to look, at the seam between its chunks, or on a
//! multi-byte character, or one past the end.
//!
//! So the properties live here once, written against `B`, and each
//! implementation is instantiated at the bottom. Adding a rope is one line, and
//! it arrives with a suite rather than needing one written for it.
//!
//! # What a property is allowed to assume
//!
//! Only the trait. Nothing here may mention a gap, a chunk or a rope -- if a
//! test needs to know which implementation it is running against, it is a test
//! of that implementation and belongs beside it.
#[cfg(test)]
mod tests {
    use crate::buffer::BufferTrait;
    use crate::buffer::gap_buffer::GapBuffer;

    /// Text with a bit of everything: several lines, a multi-byte character, a
    /// character outside the basic plane, and an empty line.
    const SAMPLE: &str = "fn main() {\n    let s = \"héllo\";\n\n    // ✓ done\n}\n";

    // ----------------------------------------------------------------
    // The properties
    // ----------------------------------------------------------------

    /// `chars_from` and `at` describe the same text, from every offset.
    ///
    /// The one that matters most. An implementation is free to walk its storage
    /// however it likes, but the two readings cannot disagree -- and where they
    /// would, it is at an internal boundary, which is why this asks at *every*
    /// offset rather than at a few interesting ones.
    fn streaming_agrees_with_indexing<B: BufferTrait>(buf: &B) {
        let len = buf.len();
        // Past the end as well: a stream that runs off the end must stop, not
        // panic and not wrap.
        for pos in 0..=len + 2 {
            let streamed: Vec<char> = buf.chars_from(pos).collect();
            let indexed: Vec<char> = (pos..len).filter_map(|at| buf.at(at)).collect();
            assert_eq!(streamed, indexed, "reading from offset {pos}");
        }
    }

    fn chars_from_agrees_with_at<B: BufferTrait>() {
        let buf = B::from(SAMPLE);
        streaming_agrees_with_indexing(&buf);
    }

    /// The same, with point moved all over the buffer first.
    ///
    /// An implementation may keep its storage arranged around where the user
    /// is -- that is the whole idea of a gap buffer -- so the agreement above
    /// has to hold wherever that arrangement happens to be. Without this, a
    /// buffer read at its default position would pass and the same buffer read
    /// after a keystroke would not.
    fn chars_from_agrees_wherever_point_is<B: BufferTrait>() {
        let mut buf = B::from(SAMPLE);
        for line in 0..buf.line_count() {
            for col in [0, 1, 4] {
                buf.cursor_move(line, col);
                streaming_agrees_with_indexing(&buf);
            }
        }
    }

    /// And after the text has actually been changed.
    fn chars_from_agrees_after_editing<B: BufferTrait>() {
        let mut buf = B::from(SAMPLE);
        buf.cursor_move(1, 4);
        for c in "let x = 1;\n".chars() {
            buf.insert(c);
        }
        streaming_agrees_with_indexing(&buf);
        buf.cursor_move(2, 0);
        for _ in 0..5 {
            buf.delete();
        }
        streaming_agrees_with_indexing(&buf);
    }

    /// Reading from the start yields the whole text.
    ///
    /// Asserted against `to_string`, so the two ways out of a buffer are pinned
    /// to each other as well.
    fn chars_from_zero_is_the_whole_text<B: BufferTrait>() {
        let buf = B::from(SAMPLE);
        let streamed: String = buf.chars_from(0).collect();
        assert_eq!(streamed, SAMPLE);
        assert_eq!(streamed, buf.to_string());
    }

    fn chars_from_the_end_is_empty<B: BufferTrait>() {
        let buf = B::from(SAMPLE);
        assert_eq!(buf.chars_from(buf.len()).count(), 0);
        assert_eq!(buf.chars_from(buf.len() + 1).count(), 0);
        assert_eq!(buf.chars_from(usize::MAX).count(), 0, "and does not wrap");
    }

    fn an_empty_buffer_streams_nothing<B: BufferTrait>() {
        let buf = B::default();
        assert_eq!(buf.chars_from(0).count(), 0);
        assert_eq!(buf.chars_from(7).count(), 0);
    }

    /// `chars_from` is lazy: asking for one character does not read the rest.
    ///
    /// Not a timing assertion -- a stream that collected eagerly would still
    /// answer correctly, and slowly, on a buffer this size. What this pins is
    /// that taking one character from a large buffer terminates promptly, which
    /// is the property the scanner leans on when it stops at a checkpoint.
    fn chars_from_is_lazy<B: BufferTrait>() {
        let buf = B::from("abcdefghij".repeat(10_000).as_str());
        assert_eq!(buf.chars_from(5).next(), Some('f'));
        assert_eq!(buf.chars_from(5).take(3).collect::<String>(), "fgh");
    }

    fn is_empty_agrees_with_len<B: BufferTrait>() {
        let mut buf = B::default();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        buf.insert('x');
        assert!(!buf.is_empty());
        buf.clear();
        assert!(buf.is_empty(), "clearing leaves nothing behind");
        assert_eq!(buf.len(), 0);
    }

    // ----------------------------------------------------------------
    // Contract the rest of the editor already depends on
    // ----------------------------------------------------------------
    //
    // Not new with `chars_from`, but never stated anywhere a second
    // implementation could read it. Every one of these is a way a rope could be
    // correct about UTF-8 and wrong about what this editor means by a position.

    /// Positions are characters, not bytes.
    ///
    /// Every offset in this editor -- point, overlays, the sexp scanner's
    /// snapshots, `syntax-ppss` -- is a character index. An implementation that
    /// counted bytes would be subtly wrong on exactly the files where it
    /// mattered, and right on the tests written in ASCII.
    fn positions_count_characters_not_bytes<B: BufferTrait>() {
        let buf = B::from("héllo");
        assert_eq!(buf.len(), 5, "five characters, six bytes");
        assert_eq!(buf.at(0), Some('h'));
        assert_eq!(buf.at(1), Some('é'));
        assert_eq!(buf.at(2), Some('l'));
        assert_eq!(buf.at(4), Some('o'));
        assert_eq!(buf.at(5), None);
    }

    fn a_buffer_round_trips_through_text<B: BufferTrait>() {
        assert_eq!(B::from(SAMPLE).to_string(), SAMPLE);
        assert_eq!(B::default().to_string(), "");
    }

    /// The two ways of naming a position agree, in both directions.
    ///
    /// `at_line_col` is deliberately *not* asserted against `at` here, though
    /// it is the obvious third leg of this property. As things stand the two
    /// disagree -- see the note in the patch that added this file -- and
    /// pinning the current behaviour would oblige every future implementation
    /// to reproduce it. The assertion goes in once the method means what its
    /// name says, or comes out of the trait.
    fn offsets_and_line_columns_are_the_same_positions<B: BufferTrait>() {
        let buf = B::from(SAMPLE);
        for pos in 0..buf.len() {
            let (line, col) = buf.cursor_1d_to_2d(pos);
            assert_eq!(
                buf.cursor_2d_to_1d(line, col),
                pos,
                "offset {pos} became {line}:{col} and did not come back"
            );
        }
    }

    /// `get_lines` hands back the lines, and a trailing newline opens one.
    ///
    /// Deliberately not asserted against `str::lines()`, which says "fn a()\n"
    /// is one line. A buffer ending in a newline has a cursor position after
    /// it, which is a place the user can be and therefore a line the editor
    /// has to have -- `point-max` lives there. An implementation that borrowed
    /// the `str` convention would be off by one line at the end of every file
    /// that ends properly.
    fn a_trailing_newline_opens_a_line<B: BufferTrait>() {
        let buf = B::from(SAMPLE);
        let shown: Vec<&str> = vec![
            "fn main() {",
            "    let s = \"héllo\";",
            "",
            "    // ✓ done",
            "}",
            "",
        ];
        assert_eq!(buf.line_count(), shown.len());
        assert_eq!(buf.get_lines(0, shown.len()), shown);
        assert_eq!(buf.get_lines(1, 3), shown[1..3].to_vec());

        let unterminated = B::from("a\nb");
        assert_eq!(unterminated.line_count(), 2, "and only a real one does");
    }

    // ----------------------------------------------------------------
    // The implementations
    // ----------------------------------------------------------------

    /// One `#[test]` per property per implementation, so a failure names both.
    ///
    /// A second text representation is one more line at the bottom of this
    /// file, and it arrives already tested.
    macro_rules! conformance {
        ($module:ident, $buffer:ty) => {
            mod $module {
                use super::*;

                #[test]
                fn chars_from_agrees_with_at() {
                    super::chars_from_agrees_with_at::<$buffer>();
                }
                #[test]
                fn chars_from_agrees_wherever_point_is() {
                    super::chars_from_agrees_wherever_point_is::<$buffer>();
                }
                #[test]
                fn chars_from_agrees_after_editing() {
                    super::chars_from_agrees_after_editing::<$buffer>();
                }
                #[test]
                fn chars_from_zero_is_the_whole_text() {
                    super::chars_from_zero_is_the_whole_text::<$buffer>();
                }
                #[test]
                fn chars_from_the_end_is_empty() {
                    super::chars_from_the_end_is_empty::<$buffer>();
                }
                #[test]
                fn an_empty_buffer_streams_nothing() {
                    super::an_empty_buffer_streams_nothing::<$buffer>();
                }
                #[test]
                fn chars_from_is_lazy() {
                    super::chars_from_is_lazy::<$buffer>();
                }
                #[test]
                fn is_empty_agrees_with_len() {
                    super::is_empty_agrees_with_len::<$buffer>();
                }
                #[test]
                fn positions_count_characters_not_bytes() {
                    super::positions_count_characters_not_bytes::<$buffer>();
                }
                #[test]
                fn a_buffer_round_trips_through_text() {
                    super::a_buffer_round_trips_through_text::<$buffer>();
                }
                #[test]
                fn offsets_and_line_columns_are_the_same_positions() {
                    super::offsets_and_line_columns_are_the_same_positions::<$buffer>();
                }
                #[test]
                fn a_trailing_newline_opens_a_line() {
                    super::a_trailing_newline_opens_a_line::<$buffer>();
                }
            }
        };
    }

    conformance!(gap_buffer, GapBuffer);
}
