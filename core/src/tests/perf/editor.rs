//! Editor benchmarks, classified by the operation they measure.
//!
//! The command path records Lisp fuel exactly; that count excludes the Rust
//! work. Buffer, syntax scan, layout and lock workloads are measured by elapsed
//! time and reported as advisory medians with their sample ranges.
use super::metrics::{per_unit_ns, ratio, repeat_elapsed, repeat_pair, time_median};
use super::report::{Report, Row};
use crate::{
    buffer::{
        Buffer, BufferTrait,
        gap_buffer::GapBuffer,
        scan::{ScanCache, checkpoint_line},
    },
    editor::create_global_env,
    input::{KeyCode, KeyEvent, KeyModifiers},
    managers::Buffers,
    modes::{SyntaxTable, sexp},
    ui::*,
};
use risp::{EvalError, Parser, eval, measure};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

fn char_event(c: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(c),
        modifiers: KeyModifiers::default(),
    }
}

/// `lines` identical lines of text.
fn text_of_lines(lines: usize) -> String {
    "a reasonably long line of text to add some volume.\n".repeat(lines)
}

// -------------------------------------------------------------------------
// Cost: the command path
// -------------------------------------------------------------------------

pub(super) fn cost(report: &mut Report) {
    // Exact fuel charged by the Lisp part of a keystroke. Rust work in the
    // buffer, renderer and editor hooks is outside this measurement.
    const SIZES: [usize; 3] = [1, 10_000, 100_000];

    let mut rows = Vec::new();
    let mut costs = Vec::new();
    for lines in SIZES {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        if lines > 1 {
            let text = text_of_lines(lines);
            state
                .with_buffer_mut("*scratch*", |b| b.text = GapBuffer::from(text.as_str()))
                .expect("*scratch* buffer must exist");
        }
        // One keystroke outside the measurement, so any first-use lazy setup is
        // not billed to the document size that happens to be measured first.
        state.handle_key_event(char_event('a'), &env);

        let length = || {
            state
                .with_buffer("*scratch*", |b| b.text.len())
                .expect("*scratch* buffer must exist")
        };
        let before = length();
        let (_, spent) = measure(&state.fuel_meter(), || {
            state.handle_key_event(char_event('a'), &env);
        });
        assert_eq!(
            length() - before,
            1,
            "the keystroke did not reach the buffer at {lines} lines -- the measurement \
             would be timing a cheap failure path instead of the work it claims to"
        );

        costs.push(spent);
        rows.push(Row::new(
            format!("command/keystroke-{lines}"),
            format!("{lines} line document"),
            spent as f64,
            format!("{spent}"),
            "units per keystroke",
        ));
    }

    report.section(
        "COMMAND PATH",
        "Lisp fuel charged for one keystroke at three document sizes. This measures\n\
         interpreter-accounted work only; Rust buffer edits and screen rendering are\n\
         measured separately by elapsed-time rows.",
        rows,
    );

    let flat = costs.windows(2).all(|w| w[0] == w[1]);
    report.verdict(
        flat,
        "Lisp fuel per keystroke is size-independent",
        format!(
            "one keystroke costs {} units at 1 / 10,000 / 100,000 lines",
            costs
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" / ")
        ),
    );
}

// -------------------------------------------------------------------------
// Timing
// -------------------------------------------------------------------------

pub(super) fn timing(report: &mut Report) {
    gap_buffer(report);
    traversal(report);
    sexp_scan(report);
    layout(report);
    command_path(report);
    concurrency(report);
    budget(report);
}

fn gap_buffer(report: &mut Report) {
    const N: usize = 50_000;

    // The gap buffer's whole reason for existing: insertion at the cursor
    // should not move the whole document on every character.
    let insert = |n: usize| {
        repeat_elapsed(|| {
            let mut buf = GapBuffer::default();
            let start = Instant::now();
            for _ in 0..n {
                buf.insert('x');
                if buf.len() % 80 == 0 {
                    buf.insert('\n');
                }
            }
            let elapsed = start.elapsed();
            assert!(buf.len() >= n);
            elapsed
        })
    };
    let small = insert(N);
    let insert_growth = ratio(insert(N * 2).as_secs_f64(), small.as_secs_f64());
    let insert_ns = per_unit_ns(small, N as u64);

    // `get_lines` uses the line index to seek to a viewport in a large buffer.
    const LINES: usize = 100_000;
    const VIEWPORT: usize = 50;
    let buf = GapBuffer::from(text_of_lines(LINES).as_str());
    let at_head = time_median(|| {
        assert_eq!(buf.get_lines(0, VIEWPORT).len(), VIEWPORT);
    });
    let at_middle = time_median(|| {
        let mid = LINES / 2;
        assert_eq!(buf.get_lines(mid, mid + VIEWPORT).len(), VIEWPORT);
    });
    let locality = ratio(at_middle.as_secs_f64(), at_head.as_secs_f64());

    report.section(
        "GAP BUFFER",
        "Insert characters into fresh buffers and read the same 50-line viewport at\n\
         the head and middle of a 100,000-line buffer. Growth and locality ratios are\n\
         advisory; compare them on the same labeled machine.",
        vec![
            Row::timed(
                "gapbuffer/insert-ns",
                "insert one char",
                insert_ns,
                small.display_ns_per_unit(N as u64, "ns/char"),
                "median; nine fresh-buffer samples",
            ),
            Row::new(
                "gapbuffer/insert-growth",
                "doubling characters typed",
                insert_growth,
                format!("{insert_growth:.2}x"),
                "advisory target: below 3x",
            ),
            Row::timed(
                "gapbuffer/get-lines-head-ns",
                format!("read {VIEWPORT} lines at head"),
                per_unit_ns(at_head, 1),
                at_head.display_ns_per_unit(1, "ns"),
                "one viewport",
            ),
            Row::timed(
                "gapbuffer/get-lines-mid-ns",
                format!("read {VIEWPORT} lines at line {}", LINES / 2),
                per_unit_ns(at_middle, 1),
                at_middle.display_ns_per_unit(1, "ns"),
                "one viewport",
            ),
            Row::new(
                "gapbuffer/locality",
                "  mid-file vs head",
                locality,
                format!("{locality:.2}x"),
                "advisory target: below 2.00x",
            ),
        ],
    );
}

/// Compare whole-buffer walks by offset and by streaming characters.
fn traversal(report: &mut Report) {
    const LINES: usize = 10_000;

    let buf = GapBuffer::from(text_of_lines(LINES).as_str());
    let len = buf.len();
    // Point in the middle, which is where it is while anybody is typing, and
    // the arrangement `at` has to branch around.
    let mut buf = buf;
    buf.cursor_move(LINES / 2, 0);
    let buf = buf;

    // Both sides accumulate, and neither uses `count`: a chain of slice
    // iterators knows its own length, so counting one answers without reading
    // a single character and reports a speedup of five figures.
    let expected: u64 = (0..len)
        .map(|pos| buf.at(pos).expect("inside the buffer") as u64)
        .fold(0, u64::wrapping_add);

    let indexed = time_median(|| {
        let mut sum = 0u64;
        for pos in 0..len {
            sum = sum.wrapping_add(buf.at(pos).expect("inside the buffer") as u64);
        }
        assert_eq!(sum, expected);
    });
    let streamed = time_median(|| {
        let mut sum = 0u64;
        for c in buf.chars_from(0) {
            sum = sum.wrapping_add(c as u64);
        }
        assert_eq!(sum, expected);
    });

    let indexed_ns = per_unit_ns(indexed, len as u64);
    let streamed_ns = per_unit_ns(streamed, len as u64);
    let speedup = ratio(indexed.as_secs_f64(), streamed.as_secs_f64());

    report.section(
        "BUFFER TRAVERSAL",
        "Walk the same 10,000-line buffer by offset and as a character stream. The\n\
         ratio and per-character medians help compare implementations; they are\n\
         informational and include the sample ranges shown above.",
        vec![
            Row::timed(
                "buffer/walk-at-ns",
                "per char, by offset",
                indexed_ns,
                indexed.display_ns_per_unit(len as u64, "ns/char"),
                "median; same immutable buffer",
            ),
            Row::timed(
                "buffer/walk-stream-ns",
                "per char, as a stream",
                streamed_ns,
                streamed.display_ns_per_unit(len as u64, "ns/char"),
                "median; same immutable buffer",
            ),
            Row::new(
                "buffer/traversal",
                "  by offset vs stream",
                speedup,
                format!("{speedup:.2}x"),
                "advisory target: stream at least as fast as offset access",
            ),
        ],
    );
}

fn sexp_scan(report: &mut Report) {
    const LINES: usize = 10_000;
    const CP: usize = crate::buffer::scan::LINES_PER_CHECKPOINT;

    let source = "(a (b \"c ( d\") e) ; ) in a comment\n".repeat(LINES);
    let text = GapBuffer::from(source.as_str());
    let table = SyntaxTable::default();

    // Both probes sit half a checkpoint interval past a checkpoint, one near
    // the end of the file and one halfway down. Measuring at the same *phase*
    // is what makes the locality row about locality: a probe just after a
    // checkpoint and one just before the next would differ by the interval
    // however well the cache worked.
    let len = text.len();
    let deep = text.cursor_2d_to_1d(CP * (LINES / CP - 2) + CP / 2, 0);
    let shallow = text.cursor_2d_to_1d(CP * (LINES / CP / 2) + CP / 2, 0);

    // The cache the worker would have built, built here directly: this measures
    // what a warm cache is worth, not how long the worker takes to warm it.
    let mut cache = ScanCache::default();
    cache.reset(1, "fundamental-mode");
    let mut scan = sexp::Scan::begin(&text, &table, usize::MAX, None, 1);
    let mut index = 0;
    while checkpoint_line(index) < LINES {
        let target = text.cursor_2d_to_1d(checkpoint_line(index), 0);
        scan.run_to(target);
        cache
            .record(index, scan.snapshot())
            .expect("checkpoints are built in order");
        index += 1;
    }
    let resume_at =
        |pos: usize| cache.resume_for("fundamental-mode", 1, text.cursor_1d_to_2d(pos).0);

    let cold = time_median(|| {
        assert_eq!(sexp::context_at(&text, &table, None, deep).depth, 0);
    });
    let warm_end = time_median(|| {
        assert_eq!(
            sexp::context_at(&text, &table, resume_at(deep), deep).depth,
            0
        );
    });
    let warm_head = time_median(|| {
        assert_eq!(
            sexp::context_at(&text, &table, resume_at(shallow), shallow).depth,
            0
        );
    });

    // What electric-pair asks on every bracket typed, measured near the *top*
    // of the file -- which is where the two questions differ most and where
    // the old one was worst. "Does the whole buffer balance" is `context_at`
    // at the end of the file, so typing on line 100 of ten thousand scanned
    // the other nine thousand nine hundred. The new question stops at the
    // first point of balance, which from inside a list is that list's closer,
    // wherever in the file you happen to be.
    let near_top = text.cursor_2d_to_1d(CP + CP / 2, 0);
    let whole_buffer = time_median(|| {
        assert_eq!(
            sexp::context_at(&text, &table, resume_at(near_top), len).depth,
            0
        );
    });
    let to_the_closer = time_median(|| {
        assert!(
            sexp::balance_point(&text, &table, resume_at(near_top + 1), near_top + 1).is_some()
        );
    });
    let bounded = ratio(whole_buffer.as_secs_f64(), to_the_closer.as_secs_f64());

    let speedup = ratio(cold.as_secs_f64(), warm_end.as_secs_f64());
    let locality = ratio(warm_end.as_secs_f64(), warm_head.as_secs_f64());
    let cold_ns = per_unit_ns(cold, 1);
    let warm_ns = per_unit_ns(warm_end, 1);

    report.section(
        "SEXP SCAN",
        "Compare a cold scan, a scan resumed from a checkpoint and balance queries\n\
         at different points in a 10,000-line file. Ratios indicate locality and\n\
         checkpoint effectiveness; they are advisory.",
        vec![
            Row::timed(
                "sexp/context-cold-ns",
                "context deep in, from the top",
                cold_ns,
                cold.display_ns_per_unit(1, "ns"),
                "median; scan from buffer start",
            ),
            Row::timed(
                "sexp/context-warm-ns",
                "context deep in, from a checkpoint",
                warm_ns,
                warm_end.display_ns_per_unit(1, "ns"),
                "median; warm checkpoint cache",
            ),
            Row::new(
                "sexp/speedup",
                "  cold vs warm",
                speedup,
                format!("{speedup:.1}x"),
                "advisory target: at least 10x",
            ),
            Row::timed(
                "sexp/balance-whole-buffer-ns",
                "balance, asked of the whole file",
                per_unit_ns(whole_buffer, 1),
                whole_buffer.display_ns_per_unit(1, "ns"),
                "median; same source buffer",
            ),
            Row::timed(
                "sexp/balance-bounded-ns",
                "balance, asked of the list point is in",
                per_unit_ns(to_the_closer, 1),
                to_the_closer.display_ns_per_unit(1, "ns"),
                "median; same source buffer",
            ),
            Row::new(
                "sexp/balance-bound",
                "  whole file vs enclosing list",
                bounded,
                format!("{bounded:.1}x"),
                "advisory; the bounded query should be substantially faster",
            ),
            Row::new(
                "sexp/locality",
                "  deep vs halfway down",
                locality,
                format!("{locality:.2}x"),
                "advisory target: below 2.00x",
            ),
        ],
    );
}

fn layout(report: &mut Report) {
    const FRAMES: usize = 200;

    fn leaf(id: usize) -> LayoutNode {
        LayoutNode::Leaf(Window::new(id, &format!("buf{id}")))
    }

    // Built by hand because there is still no `split-window` primitive to build
    // one from Lisp (roadmap item 3).
    let mut root = LayoutNode::Split {
        orientation: Orientation::Horizontal,
        division: Division::Ratio(0.5),
        left: Box::new(LayoutNode::Split {
            orientation: Orientation::Vertical,
            division: Division::Ratio(0.5),
            left: Box::new(leaf(1)),
            right: Box::new(leaf(2)),
        }),
        right: Box::new(LayoutNode::Split {
            orientation: Orientation::Vertical,
            division: Division::Ratio(0.5),
            left: Box::new(leaf(3)),
            right: Box::new(leaf(4)),
        }),
    };

    let mut buffers = Buffers::default();
    for id in 1..=4 {
        let name = format!("buf{id}");
        buffers.insert(
            &name.clone(),
            Buffer {
                text: GapBuffer::from("some buffer content\n"),
                name,
                current_mode: "fundamental".into(),
                file_path: None,
                is_modified: false,
                local_keymap: None,
                undo: Default::default(),
                mark: None,
                overlays: Default::default(),
                virtual_text: Default::default(),
                version: 0,
                scan: Default::default(),
                syntax: Default::default(),
                file_stamp: None,
                stale: false,
                auto_saved_at: None,
                read_only: false,
                data: Default::default(),
            },
        );
    }

    let screen = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    let mut render = |frames: usize| {
        time_median(|| {
            let mut views = Vec::new();
            let mut separators = Vec::new();
            for _ in 0..frames {
                views.clear();
                separators.clear();
                root.compute_tiled_views(
                    screen,
                    Focus {
                        id: WindowId(1),
                        tiled: true,
                    },
                    &buffers,
                    ComposeSettings {
                        mode_line_format: " %b ",
                        gutter: GutterSpec::default(),
                    },
                    &mut views,
                    &mut separators,
                );
            }
            assert_eq!(views.len(), 4);
        })
    };
    let small = render(FRAMES);
    let growth = ratio(render(FRAMES * 2).as_secs_f64(), small.as_secs_f64());
    let ns = per_unit_ns(small, FRAMES as u64);

    report.section(
        "LAYOUT",
        "Walking the window tree to compute tiled views, once per redraw. The growth\n\
         row asks that per-frame cost not drift upward as the editor runs.",
        vec![
            Row::timed(
                "layout/frame-ns",
                "compose one frame",
                ns,
                small.display_ns_per_unit(FRAMES as u64, "ns/frame"),
                "median; immutable layout and buffers",
            ),
            Row::new(
                "layout/growth",
                "doubling frames rendered",
                growth,
                format!("{growth:.2}x"),
                "advisory target: below 3x",
            ),
        ],
    );
}

fn command_path(report: &mut Report) {
    // Measure batches from a fresh editor state. Initialization and validation
    // are outside the timed interval; each reported value is the median batch
    // latency divided by its 20,000 successful insertions.
    const N: usize = 20_000;
    let samples = repeat_elapsed(|| {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let before = state
            .with_buffer("*scratch*", |b| b.text.len())
            .expect("*scratch* buffer must exist");
        let start = Instant::now();
        for _ in 0..N {
            state.handle_key_event(char_event('a'), &env);
        }
        let elapsed = start.elapsed();

        let inserted = state
            .with_buffer("*scratch*", |b| b.text.len())
            .expect("*scratch* buffer must exist")
            - before;
        let starved = state
            .get_logs()
            .iter()
            .filter(|line| line.contains("OutOfFuel"))
            .count();
        assert_eq!(
            inserted, N,
            "only {inserted} of {N} key events inserted text"
        );
        assert_eq!(starved, 0, "a command exhausted its fuel budget");
        elapsed
    });
    let ns = per_unit_ns(samples, N as u64);
    report.section(
        "COMMAND PATH",
        "Key event, keymap lookup, Lisp dispatch, buffer write, post-command hook.\n\
         Each sample starts from a clean editor state and times 20,000 successful\n\
         insertions. Setup and result validation are outside the clock.",
        vec![Row::timed(
            "command/keystroke-ns",
            "average per key in a 20k batch",
            ns,
            samples.display_ns_per_unit(N as u64, "ns/key"),
            "median of clean batches",
        )],
    );
    report.verdict(
        true,
        "every keystroke lands",
        "all measured batches inserted every character without exhausting command fuel",
    );
}

fn concurrency(report: &mut Report) {
    const WRITES: usize = 20_000;
    const NAMES: [&str; 3] = ["*a*", "*b*", "*c*"];

    let (sequential, concurrent) = repeat_pair(|| {
        let (state, _env) = create_global_env::<GapBuffer>().expect("global env must build");
        for name in NAMES {
            state.new_buffer(name, None, None);
        }

        let start = Instant::now();
        for name in NAMES {
            for _ in 0..WRITES {
                state.with_buffer_mut(name, |b| b.text.insert('x'));
            }
        }
        let sequential = start.elapsed();

        // Reset outside the measured interval. Threads wait at a barrier, so
        // thread creation and setup are not included in the concurrent sample.
        for name in NAMES {
            state
                .with_buffer_mut(name, |b| b.text = GapBuffer::default())
                .unwrap_or_else(|| panic!("{name} must exist"));
        }
        let (release, receiver) = std::sync::mpsc::channel::<()>();
        let ready = Arc::new(std::sync::Barrier::new(NAMES.len() + 1));
        let receiver = Arc::new(std::sync::Mutex::new(receiver));
        let workers: Vec<_> = NAMES
            .into_iter()
            .map(|name| {
                let state = state.clone();
                let ready = ready.clone();
                let receiver = receiver.clone();
                thread::spawn(move || {
                    ready.wait();
                    receiver
                        .lock()
                        .expect("channel mutex")
                        .recv()
                        .expect("released");
                    for _ in 0..WRITES {
                        state.with_buffer_mut(name, |b| b.text.insert('x'));
                    }
                })
            })
            .collect();
        ready.wait();
        let start = Instant::now();
        for _ in 0..NAMES.len() {
            release.send(()).expect("workers are waiting");
        }
        for worker in workers {
            worker.join().expect("worker thread panicked");
        }
        let concurrent = start.elapsed();

        let intact = NAMES
            .into_iter()
            .all(|name| state.with_buffer(name, |b| b.text.len()) == Some(WRITES));
        assert!(intact, "concurrent writes were lost");
        (sequential, concurrent)
    });
    let ratio = ratio(concurrent.as_secs_f64(), sequential.as_secs_f64());
    let writes = (WRITES * NAMES.len()) as u64;
    let write_ns = per_unit_ns(sequential, writes);

    report.section(
        "CONCURRENCY",
        "Each of nine samples compares the same 60,000 writes, sequentially and on\n\
         three threads writing separate buffers. The ratio is concurrent / sequential;\n\
         it depends on available CPU capacity and is advisory.",
        vec![
            Row::timed(
                "concurrency/write-ns",
                "one buffer write",
                write_ns,
                sequential.display_ns_per_unit(writes, "ns/write"),
                "median sequential batch",
            ),
            Row::new(
                "concurrency/ratio",
                "concurrent / sequential",
                ratio,
                format!("{ratio:.2}x"),
                "advisory; below 1.00x indicates higher concurrent throughput",
            ),
        ],
    );
    report.verdict(
        true,
        "no writes lost to a race",
        format!("all ten samples retained {WRITES} writes in each buffer"),
    );
}

fn budget(report: &mut Report) {
    const BUDGET: u32 = 60_000;
    const ELEMENTS: usize = 20_000;

    // Setup and parsing are excluded. The result check is functional; the
    // elapsed time is reported for comparison and never fails on its own.
    let samples = repeat_elapsed(|| {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let setup = Parser::new(&format!(
            "(progn (setq lst nil) (setq i 0) \
             (while (< i {ELEMENTS}) (setq lst (cons i lst)) (setq i (+ i 1))))"
        ))
        .next()
        .expect("setup must parse");
        eval(&setup, env.clone(), &state).expect("setup must evaluate");
        state.set_fuel_budget(BUDGET);
        let runaway = Parser::new("(while t (length lst))")
            .next()
            .expect("source must parse");

        let start = Instant::now();
        let result = eval(&runaway, env, &state);
        let elapsed = start.elapsed();
        assert_eq!(result, Err(EvalError::OutOfFuel), "runaway was not stopped");
        elapsed
    });
    let ms = samples.median.as_secs_f64() * 1e3;

    report.section(
        "EXECUTION BUDGET",
        "A runaway loop calls a list primitive under a deliberately small budget.\n\
         Every sample must return OutOfFuel; elapsed time is advisory.",
        vec![Row::timed(
            "budget/runaway-ms",
            format!("stop (while t (length lst)) over {ELEMENTS} elements"),
            ms,
            samples.display_ms(),
            format!("{BUDGET} fuel units; elapsed-time target is informational"),
        )],
    );
    report.verdict(
        true,
        "a runaway loop is stopped",
        "all ten measured attempts exhausted their budget and returned OutOfFuel",
    );
}
