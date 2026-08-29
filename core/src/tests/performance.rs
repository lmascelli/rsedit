//! Performance characterisation of the editor: the gap buffer, layout
//! composition, keystroke routing, concurrent buffer access, and the Lisp
//! execution budget.
//!
//! Every assertion is either a ratio between two measurements taken in the same
//! run, or a correctness invariant -- see `crate::tests::bench_util` for why
//! wall-clock thresholds are deliberately avoided. Absolute timings are printed
//! for tracking, never asserted on.
#[cfg(test)]
mod performance_tests {
    use crate::{
        buffer::{Buffer, BufferTrait, gap_buffer::GapBuffer},
        editor::create_global_env,
        input::{KeyCode, KeyEvent, KeyModifiers},
        lisp::{EvalError, Parser, eval},
        tests::bench_util::{growth, time_median},
        ui::*,
    };
    use std::sync::{Arc, RwLock};
    use std::thread;
    use std::time::{Duration, Instant};

    fn char_event(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::default(),
        }
    }

    /// A buffer of `lines` identical lines, as one `GapBuffer`.
    fn buffer_of_lines(lines: usize) -> GapBuffer {
        let mut text = String::with_capacity(lines * 52);
        for _ in 0..lines {
            text.push_str("a reasonably long line of text to add some volume.\n");
        }
        GapBuffer::from(text.as_str())
    }

    // ---------------------------------------------------------------------
    // Gap buffer
    // ---------------------------------------------------------------------

    /// Typing twice as many characters must cost about twice as much.
    ///
    /// The gap buffer's whole reason for existing: inserting at the cursor
    /// should not care how much text is already in the buffer. If insertion ever
    /// became O(n) -- a memmove per keystroke, say -- this doubles to ~4x.
    #[test]
    fn gap_buffer_insertion_is_linear_in_characters_typed() {
        const N: usize = 50_000;

        let insert = |n: usize| -> Duration {
            time_median(|| {
                let mut buf = GapBuffer::default();
                for _ in 0..n {
                    buf.insert('x');
                    if buf.len() % 80 == 0 {
                        buf.insert('\n');
                    }
                }
                assert!(buf.len() >= n);
            })
        };

        let small = insert(N);
        let large = insert(N * 2);
        let g = growth(large, small);
        println!(
            "gap buffer insert: {N} chars {small:?}, {} chars {large:?} -> {g:.2}x",
            N * 2
        );

        assert!(
            g < 3.0,
            "doubling the number of inserted characters cost {g:.2}x rather than ~2x -- \
             insertion is no longer independent of buffer size"
        );
    }

    /// `get_lines` walks the buffer from character zero to find the requested
    /// start line -- there is no cached line index -- so the cost of drawing one
    /// screenful grows with how deep into the file that screenful sits. And
    /// `render_screen` calls it on every keystroke.
    ///
    /// This reads the *same* number of lines from the head of the file and from
    /// the middle, and compares. The ratio today is in the hundreds, which is
    /// simply what "linear" looks like at these sizes: reaching line 50,000 to
    /// read 50 lines scans about a thousand times more text than reading 50
    /// lines at the head. The bound is therefore set well above that -- it is
    /// not asking the operation to be fast, it is asking it to stay *linear*, as
    /// anything quadratic would overshoot by orders of magnitude.
    ///
    /// The printed ratio is the number to watch. If a cached line index lands,
    /// it collapses toward 1.0x and this bound should be tightened hard to lock
    /// the win in.
    #[test]
    fn get_lines_cost_grows_with_distance_into_the_file() {
        const LINES: usize = 100_000;
        const VIEWPORT: usize = 50;

        let buf = buffer_of_lines(LINES);
        let at_head = time_median(|| {
            assert_eq!(buf.get_lines(0, VIEWPORT).len(), VIEWPORT);
        });
        let at_middle = time_median(|| {
            let mid = LINES / 2;
            assert_eq!(buf.get_lines(mid, mid + VIEWPORT).len(), VIEWPORT);
        });

        let g = growth(at_middle, at_head);
        println!(
            "get_lines {VIEWPORT} lines: at head {at_head:?}, at middle of {LINES} {at_middle:?} \
             -> {g:.1}x"
        );

        assert!(
            g < 2_500.0,
            "reading a viewport deep in the file is no longer merely linear in how far in it \
             sits: {g:.1}x the cost of reading the same lines at the head"
        );
    }

    // ---------------------------------------------------------------------
    // Layout
    // ---------------------------------------------------------------------

    /// Rendering twice as many frames must cost about twice as much -- i.e. the
    /// per-frame cost of walking the window tree must not drift upward as the
    /// editor runs.
    ///
    /// The tree is built by hand because there is still no `split-window`
    /// primitive to build one from Lisp (roadmap item 3).
    #[test]
    fn layout_composition_is_linear_in_frames_rendered() {
        const FRAMES: usize = 200;

        fn leaf(id: usize) -> LayoutNode {
            LayoutNode::Leaf(Window {
                id,
                buffer_name: format!("buf{id}"),
                scroll_x: 0,
                scroll_y: 0,
            })
        }

        let mut root = LayoutNode::Split {
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            left: Box::new(LayoutNode::Split {
                orientation: Orientation::Vertical,
                ratio: 0.5,
                left: Box::new(leaf(1)),
                right: Box::new(leaf(2)),
            }),
            right: Box::new(LayoutNode::Split {
                orientation: Orientation::Vertical,
                ratio: 0.5,
                left: Box::new(leaf(3)),
                right: Box::new(leaf(4)),
            }),
        };

        let mut buffers = std::collections::HashMap::new();
        for id in 1..=4 {
            let name = format!("buf{id}");
            buffers.insert(
                name.clone(),
                Arc::new(RwLock::new(Buffer {
                    text: GapBuffer::from("some buffer content\n"),
                    name,
                    current_mode: "fundamental".into(),
                    file_path: None,
                    is_modified: false,
                    local_keymap: None,
                })),
            );
        }

        let screen = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };

        let mut render = |frames: usize| -> Duration {
            time_median(|| {
                let mut views = Vec::new();
                for _ in 0..frames {
                    views.clear();
                    root.compute_tiled_views(screen.clone(), 1, &buffers, &mut views);
                }
                assert_eq!(views.len(), 4);
            })
        };

        let small = render(FRAMES);
        let large = render(FRAMES * 2);
        let g = growth(large, small);
        println!(
            "layout: {FRAMES} frames {small:?}, {} frames {large:?} -> {g:.2}x",
            FRAMES * 2
        );

        assert!(
            g < 3.0,
            "doubling the frame count cost {g:.2}x rather than ~2x -- per-frame layout cost is \
             drifting as the editor runs"
        );
    }

    // ---------------------------------------------------------------------
    // Keystroke routing (the full editor path)
    // ---------------------------------------------------------------------

    /// Typing twice as many characters through the *whole* editor path -- keymap
    /// lookup, Lisp dispatch of `(self-insert "a")`, the gap-buffer write, and
    /// the post-command hook -- must cost about twice as much.
    ///
    /// The correctness half matters as much as the timing half. This benchmark
    /// used to silently average real insertions together with the far cheaper
    /// `OutOfFuel` error path, because the Lisp budget was a whole-session one
    /// that ran dry about halfway through. Asserting that every keystroke
    /// actually reached the buffer is what stops a benchmark from measuring a
    /// failure path and reporting a flatteringly small number.
    #[test]
    fn keystroke_routing_is_linear_and_every_keystroke_lands() {
        const N: usize = 5_000;

        // Each sample needs a *fresh* editor, so this cannot go through
        // `time_median`; the warm-up-then-median it does is reproduced by hand
        // here. Without the discarded first run the cold-start cost lands
        // entirely on whichever size is measured first and flattens the ratio.
        let type_n = |n: usize| -> Duration {
            let mut samples = Vec::new();
            for sample in 0..4 {
                let (state, env) =
                    create_global_env::<GapBuffer>().expect("Failed to create the global env");
                let scratch = state
                    .get_buffer("*scratch*")
                    .expect("*scratch* buffer must exist");
                let before = scratch.read().unwrap().text.len();

                let start = Instant::now();
                for _ in 0..n {
                    state.handle_key_event(char_event('a'), &env);
                }
                let elapsed = start.elapsed();

                let inserted = scratch.read().unwrap().text.len() - before;
                assert_eq!(
                    inserted, n,
                    "only {inserted} of {n} keystrokes reached the buffer -- the timing would be \
                     averaging real insertions with a cheaper failure path"
                );
                if sample > 0 {
                    samples.push(elapsed);
                }
            }
            samples.sort_unstable();
            samples[samples.len() / 2]
        };

        let small = type_n(N);
        let large = type_n(N * 2);
        let g = growth(large, small);
        println!(
            "keystrokes: {N} in {small:?}, {} in {large:?} -> {g:.2}x",
            N * 2
        );

        assert!(
            g < 3.0,
            "doubling the keystroke count cost {g:.2}x rather than ~2x -- per-keystroke cost \
             grows with session length"
        );
    }

    // ---------------------------------------------------------------------
    // Concurrency
    // ---------------------------------------------------------------------

    /// Buffers are locked individually, so threads writing to *different*
    /// buffers should not get in each other's way.
    ///
    /// Comparing wall-clock against a fixed threshold would just measure the
    /// machine, so instead this runs the same total amount of work twice -- once
    /// spread across three threads on three buffers, once sequentially on one
    /// thread -- and compares. Fine-grained locking makes the concurrent run no
    /// slower than the sequential one (usually faster). A single coarse lock
    /// around the buffer table would make it *slower*, since every write would
    /// serialise anyway and pay contention on top.
    ///
    /// The bound is loose because thread scheduling is genuinely noisy; it is
    /// here to catch a lock becoming coarse, not to police jitter.
    #[test]
    fn concurrent_writes_to_separate_buffers_do_not_serialize() {
        const WRITES: usize = 20_000;

        let (state, _env) = create_global_env::<GapBuffer>().expect("Failed to create global env");
        for name in ["*a*", "*b*", "*c*"] {
            state.new_buffer(name, None, None);
        }
        let handle = |name: &str| {
            state
                .get_buffer(name)
                .unwrap_or_else(|| panic!("{name} must exist"))
        };

        // Sequential: one thread does all three buffers' work.
        let start = Instant::now();
        for name in ["*a*", "*b*", "*c*"] {
            let buf = handle(name);
            for _ in 0..WRITES {
                state.mutate_buffer(buf.clone(), |b| b.text.insert('x'));
            }
        }
        let sequential = start.elapsed();

        // Concurrent: three threads, one buffer each, same total work.
        for name in ["*a*", "*b*", "*c*"] {
            state.mutate_buffer(handle(name), |b| {
                while b.text.cursor_pos() != (0, 0) {
                    b.text.delete();
                }
            });
        }
        let start = Instant::now();
        let workers: Vec<_> = ["*a*", "*b*", "*c*"]
            .into_iter()
            .map(|name| {
                let state = state.clone();
                let name = name.to_string();
                thread::spawn(move || {
                    let buf = state.get_buffer(&name).expect("buffer must exist");
                    for _ in 0..WRITES {
                        state.mutate_buffer(buf.clone(), |b| b.text.insert('x'));
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().expect("worker thread panicked");
        }
        let concurrent = start.elapsed();

        // Every write must have survived, whichever way it ran.
        for name in ["*a*", "*b*", "*c*"] {
            assert_eq!(
                handle(name).read().unwrap().text.len(),
                WRITES,
                "{name} lost writes to a race"
            );
        }

        let g = growth(concurrent, sequential);
        println!(
            "{} writes: sequential {sequential:?}, 3 threads {concurrent:?} -> {g:.2}x",
            WRITES * 3
        );

        // Observed around 0.35-0.55x with per-buffer locks. A single coarse
        // lock would push this to 1.0x or above, since every write would
        // serialise anyway and pay contention on top; 1.5x sits between the two
        // with room for scheduling noise.
        assert!(
            g < 1.5,
            "spreading the same work across three buffers on three threads cost {g:.2}x doing \
             it sequentially -- writes to separate buffers are serialising on a shared lock"
        );
    }

    // ---------------------------------------------------------------------
    // Lisp execution budget
    // ---------------------------------------------------------------------

    /// Regression guard for the bug the `FuelMeter` work exists to fix: fuel used
    /// to be a single budget for the *entire session*, never replenished, so
    /// after roughly five thousand keystrokes every further evaluation failed
    /// with `OutOfFuel` and the editor stopped accepting input while still
    /// running.
    #[test]
    fn fuel_is_replenished_for_every_command() {
        const ATTEMPTS: usize = 20_000;

        let (state, env) = create_global_env::<GapBuffer>().expect("Failed to create global env");
        let scratch = state
            .get_buffer("*scratch*")
            .expect("*scratch* buffer must exist");
        let before = scratch.read().unwrap().text.len();

        for _ in 0..ATTEMPTS {
            state.handle_key_event(char_event('a'), &env);
        }

        let inserted = scratch.read().unwrap().text.len() - before;
        let out_of_fuel = state
            .get_logs()
            .iter()
            .filter(|line| line.contains("OutOfFuel"))
            .count();

        assert_eq!(
            inserted, ATTEMPTS,
            "only {inserted} of {ATTEMPTS} keystrokes were accepted -- fuel is not being \
             refilled per command"
        );
        assert_eq!(out_of_fuel, 0, "no keystroke should exhaust a fresh budget");
    }

    /// Refilling per command must not amount to removing the guard: an infinite
    /// loop still has to be stopped. Uses a deliberately tiny budget so this
    /// finishes instantly rather than burning the millions of steps a real
    /// command is given.
    #[test]
    fn a_runaway_loop_is_still_aborted() {
        let (state, env) = create_global_env::<GapBuffer>().expect("Failed to create global env");
        state.set_fuel_budget(50_000);

        let ast = Parser::new("(while t 1)")
            .next()
            .expect("test source must parse");

        let start = Instant::now();
        let result = eval(&ast, env.clone(), &state);

        assert_eq!(
            result.unwrap_err(),
            EvalError::OutOfFuel,
            "an infinite loop must still be stopped by the fuel guard"
        );
        println!(
            "runaway (while t 1) aborted after 50k steps in {:?}",
            start.elapsed()
        );
    }
}
