//! Editor benchmarks, classified by the operation they measure.
//!
//! One operation is exact: a keystroke, whose cost is paid in the interpreter
//! and can therefore be counted in fuel. Everything else here -- the gap buffer,
//! layout composition, buffer locking -- runs inside Rust with nothing driving
//! it, so there is nothing to count and only a clock to reach for.
use super::metrics::{per_unit_ns, ratio, time_fastest, x_ref};
use super::report::{Report, Row};
use crate::{
    buffer::{Buffer, BufferTrait, gap_buffer::GapBuffer},
    editor::create_global_env,
    input::{KeyCode, KeyEvent, KeyModifiers},
    lisp::{EvalError, Parser, eval, measure},
    ui::*,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

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
    // The whole path for one keystroke: key event, keymap lookup, Lisp dispatch
    // of `(self-insert "a")`, the primitive, the gap-buffer write, the
    // post-command hook. Measured against document size, because that is what
    // decides whether a large file still feels immediate -- and because a
    // keystroke's cost being *independent* of document size is the single
    // property that makes the editor usable, so it is worth stating exactly
    // rather than inferring from a ratio.
    const SIZES: [usize; 3] = [1, 10_000, 100_000];

    let mut rows = Vec::new();
    let mut costs = Vec::new();
    for lines in SIZES {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let scratch = state
            .get_buffer("*scratch*")
            .expect("*scratch* buffer must exist");
        if lines > 1 {
            let text = text_of_lines(lines);
            state.mutate_buffer(scratch.clone(), |b| b.text = GapBuffer::from(text.as_str()));
        }
        // One keystroke outside the measurement, so any first-use lazy setup is
        // not billed to the document size that happens to be measured first.
        state.handle_key_event(char_event('a'), &env);

        let before = scratch.read().unwrap().text.len();
        let (_, spent) = measure(state.fuel_meter(), || {
            state.handle_key_event(char_event('a'), &env);
        });
        assert_eq!(
            scratch.read().unwrap().text.len() - before,
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
        "One keystroke, end to end, at three document sizes. A flat column is the\n\
         result that matters -- and being fuel, it is flat exactly, not approximately.\n\
         Note also what a keystroke costs against the budget a command is given: a\n\
         runaway loop has to burn millions of units before it is stopped, while\n\
         ordinary typing spends a handful.",
        rows,
    );

    let flat = costs.windows(2).all(|w| w[0] == w[1]);
    report.verdict(
        flat,
        "keystroke cost is size-independent",
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

pub(super) fn timing(report: &mut Report, calibration: f64) {
    gap_buffer(report, calibration);
    layout(report, calibration);
    command_path(report, calibration);
    concurrency(report);
    budget(report);
}

fn gap_buffer(report: &mut Report, calibration: f64) {
    const N: usize = 50_000;

    // The gap buffer's whole reason for existing: inserting at the cursor must
    // not care how much text is already in the buffer. If insertion ever became
    // a memmove per keystroke, this doubles to ~4x.
    let insert = |n: usize| {
        time_fastest(|| {
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
    let insert_growth = ratio(insert(N * 2).as_secs_f64(), small.as_secs_f64());
    let insert_ns = per_unit_ns(small, N as u64);

    // `get_lines` walks from character zero to find the requested start line --
    // there is no cached line index -- so drawing one screenful costs more the
    // deeper into the file it sits. `render_screen` calls it every keystroke.
    const LINES: usize = 100_000;
    const VIEWPORT: usize = 50;
    let buf = GapBuffer::from(text_of_lines(LINES).as_str());
    let at_head = time_fastest(|| {
        assert_eq!(buf.get_lines(0, VIEWPORT).len(), VIEWPORT);
    });
    let at_middle = time_fastest(|| {
        let mid = LINES / 2;
        assert_eq!(buf.get_lines(mid, mid + VIEWPORT).len(), VIEWPORT);
    });
    let locality = ratio(at_middle.as_secs_f64(), at_head.as_secs_f64());

    report.section(
        "GAP BUFFER",
        "Insertion at the cursor, and reading a viewport out. The locality row is the\n\
         one number in this file that is a known problem rather than a control: it is\n\
         what \"linear scan from character zero\" looks like at 100,000 lines. The\n\
         bound below is not asking the operation to be fast, only to stay linear --\n\
         anything quadratic overshoots it by orders of magnitude. If a cached line\n\
         index lands, this collapses toward 1.0x and the bound should be tightened\n\
         hard to lock the win in.",
        vec![
            Row::timed(
                "gapbuffer/insert-ns",
                "insert one char",
                insert_ns,
                format!("{insert_ns:.1} ns"),
                x_ref(insert_ns, calibration),
            ),
            Row::new(
                "gapbuffer/insert-growth",
                "doubling characters typed",
                insert_growth,
                format!("{insert_growth:.2}x"),
                "linear is ~2x; must stay under 3x",
            ),
            Row::timed(
                "gapbuffer/get-lines-head-ns",
                format!("read {VIEWPORT} lines at head"),
                per_unit_ns(at_head, 1),
                format!("{:.0} ns", per_unit_ns(at_head, 1)),
                "one viewport",
            ),
            Row::timed(
                "gapbuffer/get-lines-mid-ns",
                format!("read {VIEWPORT} lines at line {}", LINES / 2),
                per_unit_ns(at_middle, 1),
                format!("{:.0} ns", per_unit_ns(at_middle, 1)),
                "one viewport",
            ),
            Row::new(
                "gapbuffer/locality",
                "  mid-file vs head",
                locality,
                format!("{locality:.0}x"),
                "linear at this size; must stay under 2500x",
            ),
        ],
    );

    report.verdict(
        insert_growth < 3.0,
        "gap-buffer insertion is linear",
        format!(
            "doubling the characters typed cost {insert_growth:.2}x rather than ~2x -- \
             insertion is no longer independent of buffer size"
        ),
    );
    report.verdict(
        locality < 2_500.0,
        "get_lines is still merely linear",
        format!(
            "reading a viewport at line {} costs {locality:.0}x reading one at the head",
            LINES / 2
        ),
    );
}

fn layout(report: &mut Report, calibration: f64) {
    const FRAMES: usize = 200;

    fn leaf(id: usize) -> LayoutNode {
        LayoutNode::Leaf(Window {
            id,
            buffer_name: format!("buf{id}"),
            scroll_x: 0,
            scroll_y: 0,
        })
    }

    // Built by hand because there is still no `split-window` primitive to build
    // one from Lisp (roadmap item 3).
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

    let mut buffers = HashMap::new();
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

    let mut render = |frames: usize| {
        time_fastest(|| {
            let mut views = Vec::new();
            for _ in 0..frames {
                views.clear();
                root.compute_tiled_views(screen.clone(), 1, &buffers, &mut views);
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
                format!("{ns:.0} ns"),
                x_ref(ns, calibration),
            ),
            Row::new(
                "layout/growth",
                "doubling frames rendered",
                growth,
                format!("{growth:.2}x"),
                "linear is ~2x; must stay under 3x",
            ),
        ],
    );
    report.verdict(
        growth < 3.0,
        "layout composition is linear",
        format!("doubling the frame count cost {growth:.2}x rather than ~2x"),
    );
}

fn command_path(report: &mut Report, calibration: f64) {
    // Whether typing *feels* immediate is a wall-clock question, so the exact
    // keystroke cost in the cost report's command-path section gets a companion
    // here. The
    // linearity that used to be asserted with a noisy ratio is now stated
    // exactly over there, so this row is tracked rather than bounded.
    const N: usize = 20_000;

    let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
    let scratch = state
        .get_buffer("*scratch*")
        .expect("*scratch* buffer must exist");
    let before = scratch.read().unwrap().text.len();

    let start = Instant::now();
    for _ in 0..N {
        state.handle_key_event(char_event('a'), &env);
    }
    let elapsed = start.elapsed();

    // Every keystroke must have landed. Without this the benchmark would
    // happily average real insertions with the far cheaper `OutOfFuel` failure
    // path and report a flatteringly small number -- which is exactly what it
    // did before fuel was refilled per command.
    let inserted = scratch.read().unwrap().text.len() - before;
    let starved = state
        .get_logs()
        .iter()
        .filter(|line| line.contains("OutOfFuel"))
        .count();

    let ns = per_unit_ns(elapsed, N as u64);
    report.section(
        "COMMAND PATH",
        "Key event, keymap lookup, Lisp dispatch, buffer write, post-command hook.\n\
         The per-keystroke figure is what a user waits for between pressing a key and\n\
         seeing the character; the exact work behind it is the command-path section\n\
         of the cost report.",
        vec![Row::timed(
            "command/keystroke-ns",
            "one keystroke, end to end",
            ns,
            format!("{ns:.0} ns"),
            x_ref(ns, calibration),
        )],
    );
    report.verdict(
        inserted == N && starved == 0,
        "every keystroke lands",
        format!(
            "{inserted} of {N} keystrokes reached the buffer, {starved} exhausted their budget"
        ),
    );
}

fn concurrency(report: &mut Report) {
    // Buffers are locked individually, so threads writing to *different*
    // buffers should not get in each other's way. Comparing wall clock against
    // a fixed threshold would just measure the machine, so this runs the same
    // total work twice -- sequentially on one thread, then spread across three
    // threads on three buffers -- and compares.
    const WRITES: usize = 20_000;
    const NAMES: [&str; 3] = ["*a*", "*b*", "*c*"];
    /// Repetitions of each arm. Both are timed by their fastest run, for the
    /// same reason every other benchmark here is: interference only ever adds
    /// time. Taking a single sample of each left the ratio spread over
    /// 0.63-0.89 against a 1.00 bound, which is not enough margin to trust.
    const REPS: usize = 3;

    let (state, _env) = create_global_env::<GapBuffer>().expect("global env must build");
    for name in NAMES {
        state.new_buffer(name, None, None);
    }
    let handle = |name: &str| {
        state
            .get_buffer(name)
            .unwrap_or_else(|| panic!("{name} must exist"))
    };
    // Replaced wholesale rather than emptied a character at a time: deleting
    // backwards through a gap buffer moves the gap on every step, so clearing
    // 60,000 characters that way took close to two minutes.
    let reset = || {
        for name in NAMES {
            state.mutate_buffer(handle(name), |b| b.text = GapBuffer::default());
        }
    };

    let mut sequential = Duration::MAX;
    let mut concurrent = Duration::MAX;
    for _ in 0..REPS {
        reset();
        let start = Instant::now();
        for name in NAMES {
            let buf = handle(name);
            for _ in 0..WRITES {
                state.mutate_buffer(buf.clone(), |b| b.text.insert('x'));
            }
        }
        sequential = sequential.min(start.elapsed());

        reset();
        // Threads are spawned before the clock starts, so what is timed is the
        // contention and not `thread::spawn`. They idle on a channel until
        // released, which is cheap and does not touch any buffer lock.
        let (release, go) = std::sync::mpsc::channel::<()>();
        let ready = Arc::new(std::sync::Barrier::new(NAMES.len() + 1));
        let go = Arc::new(std::sync::Mutex::new(go));
        let workers: Vec<_> = NAMES
            .into_iter()
            .map(|name| {
                let state = state.clone();
                let ready = ready.clone();
                let go = go.clone();
                thread::spawn(move || {
                    let buf = state.get_buffer(name).expect("buffer must exist");
                    ready.wait();
                    go.lock().expect("channel mutex").recv().expect("released");
                    for _ in 0..WRITES {
                        state.mutate_buffer(buf.clone(), |b| b.text.insert('x'));
                    }
                })
            })
            .collect();
        ready.wait();

        let start = Instant::now();
        for _ in 0..NAMES.len() {
            release.send(()).expect("workers are waiting");
        }
        for w in workers {
            w.join().expect("worker thread panicked");
        }
        concurrent = concurrent.min(start.elapsed());
    }

    let intact = NAMES
        .into_iter()
        .all(|name| handle(name).read().unwrap().text.len() == WRITES);
    let speedup = ratio(concurrent.as_secs_f64(), sequential.as_secs_f64());
    let write_ns = per_unit_ns(sequential, (WRITES * 3) as u64);

    report.section(
        "CONCURRENCY",
        "The same total work done sequentially on one thread, then spread across three\n\
         threads writing to three separate buffers. Per-buffer locks make the\n\
         concurrent run faster; a single coarse lock around the buffer table would\n\
         make it *slower* than sequential, since every write would serialise anyway\n\
         and pay contention on top. Three threads on separate buffers should approach\n\
         0.33x; 1.00x is the line between fine-grained and coarse.",
        vec![
            Row::timed(
                "concurrency/write-ns",
                "one buffer write",
                write_ns,
                format!("{write_ns:.0} ns"),
                "sequential",
            ),
            Row::new(
                "concurrency/speedup",
                "3 threads vs sequential",
                speedup,
                format!("{speedup:.2}x"),
                "under 1.00x means the locks are fine-grained",
            ),
        ],
    );
    report.verdict(
        intact,
        "no writes lost to a race",
        format!("each of the three buffers holds all {WRITES} of its writes"),
    );
    report.verdict(
        speedup < 1.0,
        "separate buffers do not serialise",
        format!(
            "spreading the work across three threads cost {speedup:.2}x doing it \
             sequentially -- at or above 1.00x, writes are contending on a shared lock"
        ),
    );
}

fn budget(report: &mut Report) {
    // The execution budget exists so a mistyped `(while t ...)` cannot hang the
    // editor. That is a promise about *time*, which makes this the one place in
    // the suite where an absolute wall-clock bound is the right thing to assert
    // -- generously, since the promise is "about a second", not a number.
    //
    // It is also where the per-element charging in the cost report earns its
    // keep. With primitives priced at one step regardless of argument length,
    // this same loop on a 20,000-element list ran for 12.2 seconds against this
    // budget, and over two minutes on a 100,000-element one: the budget bounded
    // the number of steps taken, not the work done.
    const BUDGET: u32 = 60_000;
    const ELEMENTS: usize = 20_000;

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
    let result = eval(&runaway, env.clone(), &state);
    let elapsed = start.elapsed();

    let stopped = result == Err(EvalError::OutOfFuel);
    let ms = elapsed.as_secs_f64() * 1e3;

    report.section(
        "EXECUTION BUDGET",
        "A runaway loop calling an O(n) primitive on a long list, under a deliberately\n\
         small budget. The budget's promise is about elapsed time, so this is the one\n\
         row in the suite where an absolute duration is the thing being asserted.",
        vec![Row::timed(
            "budget/runaway-ms",
            format!("stop (while t (length lst)) over {ELEMENTS} elements"),
            ms,
            format!("{ms:.2} ms"),
            format!("{BUDGET} units of budget; must stop under 1000 ms"),
        )],
    );
    report.verdict(
        stopped,
        "a runaway loop is stopped",
        "an infinite loop still exhausts its budget and returns OutOfFuel",
    );
    report.verdict(
        ms < 1_000.0,
        "the budget bounds elapsed time",
        format!(
            "a runaway loop over {ELEMENTS} elements was stopped in {ms:.2} ms -- if this \
             climbs into seconds, a primitive is being charged per call rather than per element"
        ),
    );
}
