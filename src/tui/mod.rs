//! The terminal front end: its event loop, which draws a frame when something
//! changed and waits for the terminal, the background work or the time the
//! editor names, whichever comes first.
//!
//! Design: `doc/architettura-editor.typ, <ciclo-tui>`.
pub(crate) mod colour;
pub(crate) mod input;
pub(crate) mod render;

use colour::ColorDepth;
use input::{translate_key, translate_mouse};
use render::render_frame;

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    KeyEventKind, read,
};
use crossterm::{cursor, execute, terminal};
use rsedit_core::{BufferTrait, ELispExp, EditorState, Env, LispContext, mouse_mode};
use std::{
    io::stdout,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
    },
};

/// What wakes the event loop.
enum Wakeup {
    /// An event read from the terminal, or the error reading ended with.
    Terminal(std::io::Result<Event>),
    /// A job in the background changed something: the editor is asked again
    /// how long to wait.
    Background,
}

/// Starts the thread that reads the terminal and sends what it reads to
/// SINK, so that the loop waits on one channel for the terminal and the
/// background work. The only reader of crossterm's events; never joined, it
/// ends at its first error or its first event after the loop has gone.
fn read_terminal(sink: Sender<Wakeup>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("terminal input".into())
        .spawn(move || {
            loop {
                let event = read();
                let failed = event.is_err();
                if sink.send(Wakeup::Terminal(event)).is_err() || failed {
                    break;
                }
            }
        })
        .map(drop)
}

/// Whether the terminal is the editor's: in raw mode, with bracketed paste
/// and maybe the mouse, and not yet given back.
static TERMINAL_TAKEN: AtomicBool = AtomicBool::new(false);

/// Gives the terminal back as the shell expects it, once: the mouse and
/// bracketed paste off, the screen cleared, the cursor shown, raw mode left.
/// Every step is tried whatever the one before did, since there is nobody
/// left to tell of a failure.
fn restore_terminal() {
    if !TERMINAL_TAKEN.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = execute!(
        stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0),
        cursor::Show
    );
    let _ = terminal::disable_raw_mode();
}

/// The terminal while the editor has it: given back when this is dropped,
/// however `tui_main` is left -- a quit, a `?` on an I/O error, or a panic
/// unwinding through it.
struct TakenTerminal;

impl TakenTerminal {
    /// Puts the terminal in raw mode, with bracketed paste: a paste arrives as
    /// one event, not as keystrokes, one step of undo, and the way the system
    /// clipboard comes in.
    fn take() -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        TERMINAL_TAKEN.store(true, Ordering::SeqCst);
        // From here on the terminal is given back however this is left.
        let taken = TakenTerminal;
        execute!(stdout(), EnableBracketedPaste)?;
        Ok(taken)
    }
}

impl Drop for TakenTerminal {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Makes a panic on this thread, the one that draws, give the terminal back
/// before its message is printed: the guard's drop comes only as the panic
/// unwinds, after the message, which raw mode would print in steps across
/// the screen. A panic on another thread leaves the terminal alone: the
/// background work catches its own, and the editor goes on.
fn give_back_terminal_on_panic() {
    let drawing_thread = std::thread::current().id();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == drawing_thread {
            restore_terminal();
        }
        previous(info);
    }));
}

// TODO(TUI-8): drawn on the terminal's main screen, not the alternate one.
/// Runs the editor in the terminal until it quits: raw mode, bracketed paste,
/// the mouse while `mouse-mode` asks for it, and the loop.
pub fn tui_main<B: BufferTrait>(
    state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    give_back_terminal_on_panic();
    let _terminal = TakenTerminal::take()?;
    
    // Asked once: the terminal does not change while it runs.
    let depth = ColorDepth::detect();
    // The frame's first size.
    {
        let (cols, rows) = terminal::size()?;
        env.set_variable("frame-width".into(), ELispExp::number(cols as f64));
        env.set_variable("frame-height".into(), ELispExp::number(rows as f64));
    }

    // Whether the terminal reports the mouse: it follows `mouse-mode` at every
    // turn of the loop.
    let mut capturing = false;

    // What the loop waits for: the terminal's events, and the editor's word
    // that a job changed something.
    let (wakeups, inbox) = mpsc::channel();
    read_terminal(wakeups.clone())?;
    state.on_background_change(move || {
        // Only a message: the loop composes the frame.
        let _ = wakeups.send(Wakeup::Background);
    });

    // Whether something happened that the screen may not show: an event that
    // changed nothing, as most mouse motion, composes no frame.
    let mut dirty = true;
    // Whether the next frame is drawn even if it is the one on screen: after
    // anything the user did, so that a key repaints a terminal something else
    // wrote on. One composed because time passed is drawn only if it differs.
    let mut forced = true;
    let (cols, rows) = terminal::size()?;
    let mut frame = state.snapshot(&env, cols as usize, rows as usize);

    while state.is_running() {
        // Before the frame, so that the capture is on for the next click.
        let wanted = mouse_mode(&env);
        if wanted != capturing {
            if wanted {
                execute!(stdout(), EnableMouseCapture)?;
            } else {
                execute!(stdout(), DisableMouseCapture)?;
            }
            capturing = wanted;
        }
        if dirty {
            let (cols, rows) = terminal::size()?;
            // The capture holds locks and does no I/O; the drawing, the other
            // way round.
            let next = state.snapshot(&env, cols as usize, rows as usize);
            // A frame with text for the clipboard is drawn: the only way out
            // for the text the capture took.
            if forced || next != frame || next.clipboard.is_some() {
                render_frame(&next, depth)?;
            }
            frame = next;
            dirty = false;
            forced = false;
        }

        // The editor says how long to wait at most -- a message expiring,
        // colour arriving, a selection dragged past a window's edge -- and
        // `None` for as long as anything takes.
        let wake = [state.next_redraw_in(&env, &frame), state.drag_scroll_in()]
            .into_iter()
            .flatten()
            .min();
        let wakeup = match wake {
            Some(remaining) => match inbox.recv_timeout(remaining) {
                Ok(wakeup) => wakeup,
                // The time is up: what the editor waited for is due.
                Err(RecvTimeoutError::Timeout) => {
                    state.tick(&env);
                    dirty = true;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            },
            // The editor holds a sender, so this only ends with it.
            None => match inbox.recv() {
                Ok(wakeup) => wakeup,
                Err(_) => break,
            },
        };

        let event = match wakeup {
            // Back to the top, where the editor says how long to wait.
            Wakeup::Background => continue,
            Wakeup::Terminal(event) => event?,
        };
        match event {
            Event::Key(key_event) => match translate_key(key_event) {
                Some(event) => {
                    if key_event.kind == KeyEventKind::Press {
                        state.handle_key_event(event, &env);
                        dirty = true;
                        forced = true;
                    }
                }
                None => state.log_diagnostic(&format!(
                    "[INFO] no translation for the key {:?}",
                    key_event.code
                )),
            },

            Event::Resize(width, height) => {
                state.resize(env.clone(), width as usize, height as usize);
                dirty = true;
                forced = true;
            }

            Event::Paste(text) => {
                state.handle_paste(text, &env);
                dirty = true;
                forced = true;
            }

            Event::Mouse(mouse_event) => {
                // A frame only for a mouse event the editor acted on.
                if let Some(event) = translate_mouse(mouse_event)
                    && state.handle_mouse_event(event, &env)
                {
                    dirty = true;
                    forced = true;
                }
            }

            // Focus changes, and whatever else a terminal reports.
            _ => (),
        }
    }

    // The terminal is given back as `_terminal` goes.
    Ok(())
}
