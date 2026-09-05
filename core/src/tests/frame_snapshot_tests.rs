//! `EditorState::snapshot` -- the single point where render state is captured.
//!
//! These pin the two properties the snapshot exists to provide: that a frame is
//! captured as one consistent value, and that capture holds no locks past its
//! own return.
#[cfg(test)]
mod tests {
    use crate::{
        buffer::{BufferTrait, gap_buffer::GapBuffer},
        editor::{EditorState, create_global_env},
        lisp::{LispExp, Parser, eval},
        ui::FrameSnapshot,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    const W: usize = 120;
    const H: usize = 40;

    fn editor() -> EditorState<GapBuffer> {
        create_global_env::<GapBuffer>()
            .expect("global env must build")
            .0
    }

    /// A snapshot is a plain value: no locks, no borrows, nothing pointing back
    /// into editor state. That is what will let the renderer move to its own
    /// thread without an audit of every draw call, so it is worth asserting at
    /// compile time rather than discovering later.
    #[test]
    fn a_snapshot_is_owned_data_that_can_cross_a_thread() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<FrameSnapshot>();

        let state = editor();
        let frame = state.snapshot(W, H);
        let moved = thread::spawn(move || frame.views.len())
            .join()
            .expect("the snapshot must be usable off the capturing thread");
        assert!(
            moved > 0,
            "the default layout must produce at least one view"
        );
    }

    /// Capture must be deterministic: with nothing mutating in between, two
    /// snapshots are equal. A capture that reached out to a lock a second time
    /// mid-composition could not promise this.
    #[test]
    fn two_snapshots_of_an_unchanged_editor_are_identical() {
        let state = editor();
        state.set_echo_message("steady");

        assert_eq!(
            state.snapshot(W, H),
            state.snapshot(W, H),
            "capturing twice from an unchanged editor produced two different frames"
        );
    }

    /// Everything the renderer needs is in the snapshot, taken at one instant.
    /// Before this existed the echo area was read *after* `compose_layout` had
    /// returned and dropped its locks, so a frame could show a message that was
    /// set after its own windows were composed.
    #[test]
    fn the_echo_area_is_captured_with_the_windows_not_after_them() {
        let state = editor();
        state.set_echo_message("first");
        let frame = state.snapshot(W, H);
        state.set_echo_message("second");

        assert_eq!(
            frame.echo_message, "first",
            "the snapshot picked up an echo message set after it was taken"
        );
        assert_eq!(state.snapshot(W, H).echo_message, "second");
    }

    /// The regression this refactor is for.
    ///
    /// Capture used to read `focused_window_id` once for the tiled tree and
    /// then **again for every floating window**, and to acquire `buffers` four
    /// separate times. A focus change landing between two of those reads
    /// produced a frame with two focused windows, or none -- a cursor drawn in
    /// two places at once. That is not hypothetical: `BackgroundScheduler`
    /// already runs with a clone of `EditorState`, and `(spawn ...)` adds more.
    ///
    /// This opens a floating window so the float path is actually exercised,
    /// hammers the buffers from three threads, and checks the one invariant
    /// that is *not* structurally guaranteed by the composition code: exactly
    /// one view is focused, in every frame, however the writes interleave.
    ///
    /// Note what is deliberately not asserted here. `lines.len() <=
    /// rect.height` and "the cursor sits inside its window" are both enforced
    /// by `extract_buffer_lines` and the scroll adjustment respectively, so
    /// they hold whether or not capture is atomic -- asserting them would look
    /// like a consistency check while testing nothing.
    #[test]
    fn every_frame_has_exactly_one_focused_window_under_concurrent_mutation() {
        const SNAPSHOTS: usize = 300;

        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let float = Parser::new("(make-floating-window \"*Float*\" 4 4 40 6 \"Probe\")")
            .next()
            .expect("test source must parse");
        eval(&float, env.clone(), &state).expect("the floating window must open");

        // `make-floating-window` focuses what it opens, so this is the float's
        // id; 0 is the original tiled window.
        let float_id = state.get_focused_window_id();
        assert_ne!(float_id, 0, "the float must have taken focus");

        let scratch = state
            .get_buffer("*scratch*")
            .expect("*scratch* buffer must exist");
        let stop = Arc::new(AtomicBool::new(false));

        // The thread that makes this test bite: focus moves between the tiled
        // window and the float continuously, so any capture that reads
        // `focused_window_id` more than once will eventually read two
        // different values while composing a single frame.
        let flipper = {
            let state = state.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                let mut on_float = true;
                while !stop.load(Ordering::Relaxed) {
                    state.set_focused_window_id(if on_float { float_id } else { 0 });
                    on_float = !on_float;
                    thread::yield_now();
                }
            })
        };

        // One writer, not a herd. The point is that mutation happens *during*
        // capture, which one thread achieves just as well -- and both helpers
        // yield rather than spinning, so this test does not saturate the
        // machine and skew the timing benchmarks running alongside it.
        let writer = {
            let state = state.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    state.mutate_buffer(scratch.clone(), |b| {
                        b.text.insert('x');
                        b.text.insert('\n');
                    });
                    thread::yield_now();
                }
            })
        };

        for _ in 0..SNAPSHOTS {
            let frame = state.snapshot(W, H);
            assert_eq!((frame.width, frame.height), (W, H));
            assert!(
                frame.views.len() >= 2,
                "the float path is not being exercised: {} view(s) captured",
                frame.views.len()
            );
            assert_eq!(
                frame.views.iter().filter(|v| v.is_focused).count(),
                1,
                "a frame reported {} focused windows -- focus was read at more than one \
                 instant while composing it",
                frame.views.iter().filter(|v| v.is_focused).count()
            );
            assert!(frame.focused_view().is_some());
        }

        stop.store(true, Ordering::Relaxed);
        writer.join().expect("writer thread panicked");
        flipper.join().expect("focus thread panicked");
    }
}
