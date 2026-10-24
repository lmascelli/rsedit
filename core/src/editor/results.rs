//! Where a listing goes -- occur, grep, compile: which buffer takes a new
//! run's output, where it is shown, which window a visit opens, and what
//! quitting the listing gives back.
//!
//! Design: `doc/architettura-editor.typ, <risultati>`.

use super::*;
use crate::text::results::{RESULTS_KEY, Results};

/// The key a listing's [`Presentation`] is kept under, in its buffer's data.
const CONTEXT: &str = "results-presentation";

/// How a listing was asked for and where it is shown: kept in the listing's
/// buffer, so that the next run, a visit and quitting each know.
#[derive(Clone)]
struct Presentation {
    /// What made the listing -- `occur`, `grep`, `compile` -- and so which
    /// listings a new run may take the place of.
    producer: String,
    /// When the run began: of several listings, the newest is reused.
    started: Instant,
    /// The window the listing was asked from, where a visit goes back to,
    /// and the buffer it showed: kept from the first run when a run starts
    /// from the listing itself.
    origin_window: WindowId,
    origin_buffer: String,
    /// The buffer the output took the place of, which quitting shows again.
    restore_buffer: String,
    /// The window showing the listing, once it is shown.
    window: Option<WindowId>,
    /// Whether that window is a strip at the bottom of the frame, opened for
    /// the listing, rather than the window it was asked from.
    bottom: bool,
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns how the listing in BUFFER was asked for and shown, if it is one.
    fn results_presentation(
        &self,
        buffer: &str,
    ) -> Option<Presentation> {
        self.with_buffer(buffer, |buf| {
            buf.data::<Presentation>(CONTEXT).cloned()
        })
        .flatten()
    }

    /// Returns a presentation for a run of PRODUCER asked from the focused
    /// window: a run started from a listing in its own window keeps the
    /// listing's origin.
    fn new_results_presentation(
        &self,
        producer: &str,
    ) -> Presentation {
        let window = self.get_focused_window_id();

        let buffer = self
            .focused_window_buffer()
            .unwrap_or_else(|| self.get_current_buffer_name());

        let previous = self.results_presentation(&buffer);

        let (origin_window, origin_buffer) = match previous {
            Some(previous) if previous.window == Some(window) => {
                (previous.origin_window, previous.origin_buffer)
            }
            _ => (window, buffer.clone()),
        };

        Presentation {
            producer: producer.into(),
            started: Instant::now(),
            origin_window,
            origin_buffer,
            restore_buffer: buffer,
            window: None,
            bottom: false,
        }
    }

    /// Returns the buffer a run of PRODUCER writes into, emptied, read-only,
    /// in MODE: the newest listing of the same producer and mode when
    /// `results-reuse-buffer` says so, stopping the work that wrote into it,
    /// and otherwise a new buffer named after STEM. PROTECTED, the buffer the
    /// run reads, is never the one taken. `None` when the buffer could not
    /// be had.
    pub(crate) fn prepare_results_buffer(
        &self,
        producer: &str,
        stem: &str,
        mode: &str,
        protected: Option<&str>,
        env: &Arc<Env<Self>>,
    ) -> Option<String> {
        let mut presentation =
            self.new_results_presentation(producer);

        let reusable = if results_reuse_buffer(env) {
            self.buffer_names()
                .into_iter()
                .filter_map(|name| {
                    if protected == Some(name.as_str()) {
                        return None;
                    }

                    let previous =
                        self.results_presentation(&name)?;

                    let eligible = self
                        .with_buffer(&name, |buf| {
                            buf.file_path.is_none()
                                && buf.current_mode == mode
                        })
                        .unwrap_or(false);

                    (eligible && previous.producer == producer)
                        .then_some((previous.started, name))
                })
                .max_by_key(|(started, _)| *started)
                .map(|(_, name)| name)
        } else {
            None
        };

        let name = match reusable {
            Some(name) => name,
            None => {
                let name = self.free_name_for_work(stem);

                self.new_buffer(
                    &name,
                    None,
                    Some(mode.into()),
                    env,
                )?;

                name
            }
        };

        if let Some(previous) =
            self.results_presentation(&name)
        {
            presentation.window = previous.window;
            presentation.bottom = previous.bottom;

            if presentation.restore_buffer == name {
                presentation.restore_buffer =
                    previous.restore_buffer;
            }
        }

        // The writer is taken away under the lock it writes under, and its
        // work stopped once the buffer's lock is given back.
        let (writer, marked) = self.with_buffer_mut(
            &name,
            |buf| {
                let writer = buf.writer.take();

                let marked = buf
                    .data::<Results>(RESULTS_KEY)
                    .and_then(|results| results.marked.clone());

                buf.adopt_text("");
                buf.text.cursor_move(0, 0);
                buf.forget_data(RESULTS_KEY);
                buf.forget_data("lisp:results-refresh");
                buf.read_only = true;
                buf.put_data(CONTEXT, presentation);

                (writer, marked)
            },
        )?;

        if let Some(writer) = writer {
            self.work().cancel(writer);
        }

        if let Some(marked) = marked {
            self.with_buffer_mut(&marked, |buf| {
                buf.overlays
                    .remove_category(Some("results-target"))
            });
        }

        Some(name)
    }

    /// Shows the listing BUFFER, and returns the window it is in: the focused
    /// window, when `results-display-location` is `same-window`; otherwise
    /// the strip at the bottom that shows a listing of the same producer, or
    /// a new one HEIGHT rows high. Focus goes back where it was unless
    /// SELECT. `None` when there is no such buffer.
    pub(crate) fn display_results(
        &self,
        buffer: &str,
        height: usize,
        select: bool,
        env: &Arc<Env<Self>>,
    ) -> Option<WindowId> {
        if !self.has_buffer(buffer) {
            return None;
        }

        let from = self.get_focused_window_id();

        let mut presentation = self
            .results_presentation(buffer)
            .unwrap_or_else(|| {
                self.new_results_presentation("results")
            });

        let same = results_display_same_window(env);

        let existing = if same {
            None
        } else {
            self.buffer_names()
                .into_iter()
                .filter_map(|name| {
                    let previous =
                        self.results_presentation(&name)?;

                    let window = previous.window?;

                    let showing = self.windows(|windows| {
                        windows.buffer_of(window)
                    });

                    (previous.bottom
                        && previous.producer
                            == presentation.producer
                        && showing.as_deref()
                            == Some(name.as_str()))
                    .then_some((previous.started, window))
                })
                .max_by_key(|(started, _)| *started)
                .map(|(_, window)| window)
        };

        let window = if same {
            from
        } else if let Some(window) = existing {
            window
        } else {
            self.open_bottom_window(buffer, height.max(1))
        };

        self.select_window(window);
        self.switch_to_buffer(buffer);

        presentation.window = Some(window);
        presentation.bottom = !same;

        self.with_buffer_mut(buffer, |buf| {
            buf.put_data(CONTEXT, presentation);
        });

        if !select {
            self.select_window(from);
        }

        Some(window)
    }

    /// Gives focus to the window a visit to an entry of LISTING opens in, and
    /// returns false when there was none to give: the listing's own window
    /// when `results-visit-same-window` says so, and otherwise the window
    /// the listing was asked from, the focused one, another one, or a new
    /// split -- never the listing's, and with PREVIEW never the focused one.
    /// A visit and `next-error` both go there.
    pub(crate) fn select_results_target(
        &self,
        listing: &str,
        preview: bool,
        env: &Arc<Env<Self>>,
    ) -> bool {
        if !preview && results_visit_same_window(env) {
            return true;
        }

        let from = self.get_focused_window_id();

        let windows = self.windows(|windows| {
            windows
                .root()
                .window_ids()
                .into_iter()
                .filter_map(|id| {
                    windows.buffer_of(id).map(|name| (id, name))
                })
                .collect::<Vec<_>>()
        });

        let usable = |id: WindowId| {
            windows.iter().any(|(window, name)| {
                *window == id
                    && name != listing
                    && (!preview || *window != from)
            })
        };

        if let Some(presentation) =
            self.results_presentation(listing)
        {
            if usable(presentation.origin_window) {
                return self.select_window(
                    presentation.origin_window,
                );
            }
        }

        if usable(from) {
            return true;
        }

        if let Some((window, _)) =
            windows.iter().find(|(id, _)| usable(*id))
        {
            return self.select_window(*window);
        }

        // Another window on the listing, before splitting for a new one.
        if let Some((window, _)) =
            windows.iter().find(|(id, _)| *id != from)
        {
            return self.select_window(*window);
        }

        match self.split_focused_window(
            Orientation::Horizontal,
            Division::Ratio(0.5),
        ) {
            Some(window) => self.select_window(window),
            None => false,
        }
    }

    /// Leaves the listing in the focused window: a strip opened for it closes,
    /// and focus goes back to the window it was asked from; otherwise the
    /// window shows again the buffer the output took the place of, or the one
    /// it was asked from, or the most recent. False when there was nothing
    /// to go back to.
    pub(crate) fn quit_results(&self) -> bool {
        let buffer = self.get_current_buffer_name();
        let here = self.get_focused_window_id();
        let presentation =
            self.results_presentation(&buffer);

        if let Some(presentation) = &presentation {
            if presentation.bottom
                && presentation.window == Some(here)
                && self.delete_window_by_id(here)
            {
                self.select_window(
                    presentation.origin_window,
                );
                return true;
            }
        }

        let previous = presentation
            .into_iter()
            .flat_map(|presentation| {
                [
                    presentation.restore_buffer,
                    presentation.origin_buffer,
                ]
            })
            .find(|name| {
                name != &buffer && self.has_buffer(name)
            })
            .or_else(|| self.most_recent_buffer(&buffer));

        previous.is_some_and(|name| {
            self.switch_to_buffer(&name)
        })
    }
}