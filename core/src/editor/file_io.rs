//! Bounded file operations, separate from the executor and from each other.
//! Results are applied by tick, on the command thread. A slow operation
//! keeps its reservation: a timeout is a warning, never a cancellation.
//! Design: doc/architettura-editor.typ, <file-su-disco>.
use super::*;
use std::collections::HashMap;
use std::sync::{Mutex, Weak};

use crate::buffer::disk::IoMark as Mark;
pub(crate) use crate::buffer::disk::{IO_ERROR as ERROR, IO_MARK as MARK};
use crate::buffer::io_status::{FileIssue, IoCause};
pub(crate) const TIMEOUT: &str = "file-io-timeout";
pub(crate) type Handle<B> = Arc<RwLock<Buffer<B>>>;
pub(crate) type Completion<B> = Box<dyn FnOnce(&EditorState<B>, &Arc<Env<EditorState<B>>>) + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    Read,
    Write,
    Watch,
    Recovery,
}

impl Lane {
    fn limit(self) -> usize {
        match self {
            Self::Read | Self::Watch => 2,
            Self::Write => 4,
            Self::Recovery => 1,
        }
    }
}

struct Pending<B: BufferTrait> {
    lane: Lane,
    key: String,
    label: String,
    origin: Option<Weak<RwLock<Buffer<B>>>>,
    started: Instant,
    timeout: Option<Duration>,
    warned: bool,
}

struct Finished<B: BufferTrait> {
    id: u64,
    action: Completion<B>,
}

pub(crate) struct FileIo<B: BufferTrait> {
    recovery_lock: Mutex<()>,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, Pending<B>>>,
    finished: Mutex<Vec<Finished<B>>>,
    confirmations: Mutex<HashMap<String, (u64, Completion<B>)>>,
}

impl<B: BufferTrait> Default for FileIo<B> {
    fn default() -> Self {
        Self {
            recovery_lock: Mutex::new(()),
            next: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            finished: Mutex::new(Vec::new()),
            confirmations: Mutex::new(HashMap::new()),
        }
    }
}

/// Reservation for the compatible synchronous Lisp save functions.
pub(crate) struct FileWriteGuard<B: BufferTrait> {
    state: EditorState<B>,
    id: u64,
}
impl<B: BufferTrait> Drop for FileWriteGuard<B> {
    fn drop(&mut self) {
        self.state.finish_file_io(self.id);
    }
}

pub(crate) fn timeout<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
    match env.get_variable(TIMEOUT) {
        Some(value) if value.is_nil() => None,
        Some(ELispExp::Number(n)) if n.is_finite() && n >= 0.1 => {
            Duration::try_from_secs_f64(n).ok()
        }
        _ => Some(Duration::from_secs(10)),
    }
}

impl<B: BufferTrait> EditorState<B> {
    pub(crate) fn write_recovery(
        &self,
        handle: &Handle<B>,
        path: &std::path::Path,
        version: u64,
        text: &str,
    ) -> std::io::Result<u64> {
        let _serial = self
            .file_io
            .recovery_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = format!("file-io:recovery:{}", path.display());
        if let Some(previous) = handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .data::<u64>(&key)
            && *previous >= version
        {
            return Ok(*previous);
        }
        let written = (|| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            crate::buffer::disk::write_atomically(path, text.as_bytes())
        })();
        let at = std::time::SystemTime::now();
        let mut buf = handle
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buf.note_auto_save(
            at,
            version,
            Some(path),
            written.as_ref().map(|_| ()).map_err(FileIssue::from_io),
        );
        if written.is_ok() {
            buf.put_data(&key, version);
        }
        written.map(|()| version)
    }

    pub(crate) fn prepare_recovery(
        &self,
        handle: &Handle<B>,
        file: &str,
        directory: Result<Option<String>, String>,
        version: u64,
        text: &str,
    ) -> Result<std::path::PathBuf, FileIssue> {
        let directory = directory.map_err(|why| {
            let issue = FileIssue::new(IoCause::Configuration, why);
            handle
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .note_auto_save(
                    std::time::SystemTime::now(),
                    version,
                    None,
                    Err(issue.clone()),
                );
            issue
        })?;
        let path = crate::jobs::autosave::auto_save_path(file, directory.as_deref());
        self.write_recovery(handle, &path, version, text)
            .map_err(|why| FileIssue::from_io(&why))?;
        Ok(path)
    }

    pub(crate) fn remove_recovery(
        &self,
        handle: &Handle<B>,
        path: &std::path::Path,
        saved_version: u64,
    ) -> std::io::Result<bool> {
        let _serial = self
            .file_io
            .recovery_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        {
            let buf = handle
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let latest = buf
                .data::<u64>(&format!("file-io:recovery:{}", path.display()))
                .copied();
            if buf.is_modified || latest.is_some_and(|version| version > saved_version) {
                return Ok(false);
            }
        }
        match std::fs::remove_file(path) {
            Err(why) if crate::buffer::io_status::is_missing(&why) => {}
            result => result?,
        }
        handle
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .forget_data(&format!("file-io:recovery:{}", path.display()));
        Ok(true)
    }

    pub(crate) fn file_handle(&self, name: &str) -> Option<Handle<B>> {
        self.buffers(|buffers| buffers.handle(name))
    }

    /// A handle must still name the same buffer, not one recreated under
    /// its former name while the disk was being read.
    pub(crate) fn file_handle_name(&self, handle: &Handle<B>) -> Option<String> {
        let name = handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .name
            .clone();
        self.file_handle(&name)
            .filter(|current| Arc::ptr_eq(current, handle))
            .map(|_| name)
    }

    pub(crate) fn file_io_busy(&self, key: &str) -> bool {
        self.file_io
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|pending| pending.key == key)
    }

    pub(crate) fn buffer_write_pending(&self, handle: &Handle<B>) -> bool {
        self.file_io
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|pending| {
                pending.lane == Lane::Write
                    && pending
                        .origin
                        .as_ref()
                        .and_then(Weak::upgrade)
                        .is_some_and(|other| Arc::ptr_eq(&other, handle))
            })
    }

    pub(crate) fn file_writes_pending(&self) -> bool {
        self.file_io
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|pending| pending.lane == Lane::Write)
    }

    pub(crate) fn reserve_file_write(
        &self,
        handle: Handle<B>,
        path: &str,
    ) -> Option<FileWriteGuard<B>> {
        let id = self.file_io.next.fetch_add(1, Ordering::Relaxed);
        let key = format!("file:{path}");
        let mut pending = self
            .file_io
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.values().any(|p| p.key == key)
            || pending.values().filter(|p| p.lane == Lane::Write).count() >= Lane::Write.limit()
        {
            return None;
        }
        pending.insert(
            id,
            Pending {
                lane: Lane::Write,
                key,
                label: "Saving".into(),
                origin: Some(Arc::downgrade(&handle)),
                started: Instant::now(),
                timeout: None,
                warned: false,
            },
        );
        Some(FileWriteGuard {
            state: self.clone(),
            id,
        })
    }

    /// Reserves before starting a thread. No queue or unbounded collection
    /// of blocked threads grows when a mount stops answering.
    pub(crate) fn start_file_io<R: Send + 'static>(
        &self,
        lane: Lane,
        key: String,
        label: &str,
        origin: Option<Handle<B>>,
        timeout: Option<Duration>,
        work: impl FnOnce() -> R + Send + 'static,
        done: impl FnOnce(&EditorState<B>, &Arc<Env<EditorState<B>>>, R) + Send + 'static,
    ) -> Option<u64> {
        let id = self.file_io.next.fetch_add(1, Ordering::Relaxed);
        {
            let mut pending = self
                .file_io
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pending.values().any(|p| p.key == key)
                || pending.values().filter(|p| p.lane == lane).count() >= lane.limit()
            {
                return None;
            }
            pending.insert(
                id,
                Pending {
                    lane,
                    key,
                    label: label.to_string(),
                    origin: origin.as_ref().map(Arc::downgrade),
                    started: Instant::now(),
                    timeout,
                    warned: false,
                },
            );
        }
        if matches!(lane, Lane::Read | Lane::Write)
            && let Some(handle) = &origin
        {
            let mut buf = handle
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if buf.data::<Vec<Mark>>(MARK).is_none() {
                buf.put_data(MARK, Vec::<Mark>::new());
            }
            buf.data_mut::<Vec<Mark>>(MARK)
                .expect("I/O markers")
                .push(Mark {
                    id,
                    label: label.to_string(),
                    slow: false,
                });
        }
        let panic_origin = origin.as_ref().map(Arc::downgrade);
        if self
            .spawn_work(&format!("file-io-{id}"), move |state, _, _| {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
                let action: Completion<B> = match result {
                    Ok(value) => Box::new(move |state, env| done(state, env, value)),
                    Err(_) => Box::new(move |state, _| {
                        if let Some(handle) = panic_origin.and_then(|weak| weak.upgrade()) {
                            let mut buf = handle
                                .write()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            buf.put_data(
                                ERROR,
                                FileIssue::new(
                                    IoCause::Uncertain,
                                    "file operation panicked; outcome unknown",
                                ),
                            );
                            if lane == Lane::Write {
                                buf.is_modified = true;
                            }
                        }
                        state.set_echo_message(
                            "File operation failed unexpectedly; buffer kept unsaved",
                        );
                        state.log_diagnostic("[ERROR] file I/O worker panicked");
                    }),
                };
                state
                    .file_io
                    .finished
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(Finished { id, action });
            })
            .is_none()
        {
            self.finish_file_io(id);
            return None;
        }
        Some(id)
    }

    fn finish_file_io(&self, id: u64) {
        let pending = self
            .file_io
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
        if let Some(handle) = pending
            .and_then(|pending| pending.origin)
            .and_then(|weak| weak.upgrade())
        {
            let mut buf = handle
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(marks) = buf.data_mut::<Vec<Mark>>(MARK) {
                marks.retain(|mark| mark.id != id);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn file_io_idle(&self) -> bool {
        self.file_io
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    pub(crate) fn file_io_ready(&self) -> bool {
        !self
            .file_io
            .finished
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    pub(crate) fn file_io_due_in(&self) -> Option<Duration> {
        self.file_io
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|pending| !pending.warned)
            .filter_map(|pending| {
                pending
                    .timeout
                    .map(|limit| limit.saturating_sub(pending.started.elapsed()))
            })
            .min()
    }

    pub(crate) fn run_file_io(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        let completed = std::mem::take(
            &mut *self
                .file_io
                .finished
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let mut changed = !completed.is_empty();
        for finished in completed {
            self.finish_file_io(finished.id);
            (finished.action)(self, env);
        }
        let warnings: Vec<_> = {
            let mut pending = self
                .file_io
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pending
                .iter_mut()
                .filter_map(|(id, pending)| {
                    if !pending.warned
                        && pending
                            .timeout
                            .is_some_and(|limit| pending.started.elapsed() >= limit)
                    {
                        pending.warned = true;
                        Some((
                            *id,
                            pending.label.clone(),
                            pending.key.clone(),
                            pending.origin.clone(),
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        };
        for (id, label, path, origin) in warnings {
            changed = true;
            if let Some(handle) = origin.and_then(|weak| weak.upgrade()) {
                let mut buf = handle
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if buf.data::<Vec<Mark>>(MARK).is_none() {
                    buf.put_data(MARK, Vec::<Mark>::new());
                }
                let marks = buf.data_mut::<Vec<Mark>>(MARK).expect("I/O markers");
                if let Some(mark) = marks.iter_mut().find(|mark| mark.id == id) {
                    mark.slow = true;
                } else {
                    marks.push(Mark {
                        id,
                        label: label.clone(),
                        slow: true,
                    });
                }
            }
            let message = format!("{label} still pending: {path}; result uncertain, not cancelled");
            self.set_echo_message(&message);
            self.log_diagnostic(&message);
        }
        changed
    }

    pub(crate) fn remember_file_confirmation(&self, key: String, action: Completion<B>) -> u64 {
        let id = self.file_io.next.fetch_add(1, Ordering::Relaxed);
        let mut confirmations = self
            .file_io
            .confirmations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if confirmations.len() >= 16
            && let Some(oldest) = confirmations
                .iter()
                .min_by_key(|(_, (id, _))| *id)
                .map(|(key, _)| key.clone())
        {
            confirmations.remove(&oldest);
        }
        confirmations.insert(key, (id, action));
        id
    }

    pub(crate) fn cancel_file_confirmation(&self, id: u64) {
        self.file_io
            .confirmations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|_, (number, _)| *number != id);
    }

    pub(crate) fn confirm_file_io(&self, id: u64, env: &Arc<Env<EditorState<B>>>) {
        let action = {
            let mut confirmations = self
                .file_io
                .confirmations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let key = confirmations
                .iter()
                .find(|(_, (number, _))| *number == id)
                .map(|(key, _)| key.clone());
            key.and_then(|key| confirmations.remove(&key))
                .map(|(_, action)| action)
        };
        if let Some(action) = action {
            action(self, env);
        } else {
            self.set_echo_message("This save confirmation is no longer current; save again");
        }
    }
}
