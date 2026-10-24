//! `EditorState::snapshot`, where what the host draws is captured: a frame is
//! one consistent value, and capturing it holds no lock past its return.
#[cfg(test)]
mod tests {
    use crate::{
        buffer::{BufferTrait, gap_buffer::GapBuffer},
        editor::{EditorState, create_global_env},
        ui::{FrameSnapshot, WindowId},
    };
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    const W: usize = 120;
    const H: usize = 40;

    fn editor() -> (EditorState<GapBuffer>, Arc<Env<EditorState<GapBuffer>>>) {
        create_global_env::<GapBuffer>().expect("global env must build")
    }

    /// A snapshot is a plain value: no locks, no borrows, nothing pointing back
    /// into the editor's state. That is what lets a renderer draw on a thread
    /// of its own, and it is asserted at compile time.
    #[test]
    fn a_snapshot_is_owned_data_that_can_cross_a_thread() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<FrameSnapshot>();

        let (state, env) = editor();
        let frame = state.snapshot(&env, W, H);
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
        let (state, env) = editor();
        state.set_echo_message("steady");

        assert_eq!(
            state.snapshot(&env, W, H),
            state.snapshot(&env, W, H),
            "capturing twice from an unchanged editor produced two different frames"
        );
    }

    /// Everything the renderer needs is in the snapshot, taken at one instant:
    /// the echo area with the windows, so that a frame cannot show a message
    /// set after its windows were composed.
    #[test]
    fn the_echo_area_is_captured_with_the_windows_not_after_them() {
        let (state, env) = editor();
        state.set_echo_message("first");
        let frame = state.snapshot(&env, W, H);
        state.set_echo_message("second");

        assert_eq!(
            frame.echo_message, "first",
            "the snapshot picked up an echo message set after it was taken"
        );
        assert_eq!(state.snapshot(&env, W, H).echo_message, "second");
    }

    /// Every frame has exactly one focused view, however the writes of other
    /// threads fall: a capture that read the focus once for the tiled tree
    /// and again for each float would, with focus moving in between, draw a
    /// cursor in two windows or in none. The WorkManager's executor and
    /// threads act on clones of the editor, and so does `(spawn ...)`.
    ///
    /// This opens a float, so that its path runs; moves focus from one thread
    /// and writes into the buffer from another while this one captures; and
    /// checks the one property the composition does not guarantee by its
    /// shape. `lines.len() <= rect.height`, and the cursor inside its window,
    /// hold whether or not the capture is atomic, and are not asserted.
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
        assert_ne!(float_id, WindowId(0), "the float must have taken focus");

        let stop = Arc::new(AtomicBool::new(false));

        // The thread that makes this test bite: focus moves between the tiled
        // window and the float without pause, so a capture that read the focus
        // more than once would read two values in one frame.
        let flipper = {
            let state = state.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                let mut on_float = true;
                while !stop.load(Ordering::Relaxed) {
                    state.set_focused_window_id(if on_float { float_id } else { WindowId(0) });
                    on_float = !on_float;
                    thread::yield_now();
                }
            })
        };

        // One writer, not a herd: what matters is that the editor changes
        // during the capture. Both helpers yield rather than spin, so that the
        // test does not take the machine from the timings run beside it.
        let writer = {
            let state = state.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    state.with_buffer_mut("*scratch*", |b| {
                        b.text.insert('x');
                        b.text.insert('\n');
                    });
                    thread::yield_now();
                }
            })
        };

        for _ in 0..SNAPSHOTS {
            let frame = state.snapshot(&env, W, H);
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
