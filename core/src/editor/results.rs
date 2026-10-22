//! Buffer lifetime and window selection shared by result producers.

use super::*;
use crate::text::results::{RESULTS_KEY, Results};

const CONTEXT: &str = "results-presentation";

#[derive(Clone)]
struct Presentation {
    producer: String,
    started: Instant,
    origin_window: WindowId,
    origin_buffer: String,
    restore_buffer: String,
    window: Option<WindowId>,
    bottom: bool,
}

impl<B: BufferTrait> EditorState<B> {
    fn results_presentation(
        &self,
        buffer: &str,
    ) -> Option<Presentation> {
        self.with_buffer(buffer, |buf| {
            buf.data::<Presentation>(CONTEXT).cloned()
        })
        .flatten()
    }

    /// Capture the logical source, including a run started from its output.
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

    /// Acquire output without replacing the source named by PROTECTED.
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

        // Revoke the writer under the same lock used by its writes.
        // Cancellation happens outside the buffer lock.
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

    /// Display output, retaining the caller's focus unless SELECT is set.
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

    /// Select the destination shared by direct visits and next-error.
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

        // Reuse another view of the listing before creating a split.
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

    /// Close a managed strip or restore the buffer replaced by output.
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