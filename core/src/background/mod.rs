//! Where the work that blocks runs, until it moves to the WorkManager.
//!
//! # The rule
//!
//! > The shared thread may only run work that is guaranteed to return.
//!
//! Two kinds of work want to happen away from the thread the user is typing
//! on, and they are not the same kind:
//!
//! - **Bounded.** A step of a Lisp job, stopped by `worker-fuel` whether it
//!   wants to stop or not. A chunk of syntax colouring. These cannot run long
//!   -- not by convention, but because something cuts them off -- so any
//!   number of them can share one thread and take turns. They are jobs on the
//!   WorkManager now, in [`crate::work`], and its executor is the shared
//!   thread.
//!
//! - **Unbounded.** Reading a child process's output. Walking a directory
//!   tree. These take as long as the operating system takes, and no budget
//!   can shorten them. One of these on the shared thread stops every bounded
//!   job behind it.
//!
//! That second case was not hypothetical: `M-x compile` on a long build held
//! the shared thread for the whole build, and colouring, the prescanner, the
//! file watcher, the auto-saver and every Lisp worker and background job
//! stopped until it finished -- with a second shell command queued behind it,
//! unable to start. So each piece of unbounded work gets a thread of its own,
//! and this is what starts them: a shell command, and a search of files. They
//! move to the WorkManager's dedicated threads next, and this goes.
use std::sync::mpsc::Receiver;

use crate::{buffer::BufferTrait, editor::EditorState, editor::WorkerTurn};

/// Work that runs to completion, on a thread of its own.
///
/// `self: Box<Self>`: it is consumed, it keeps no state between calls, and
/// there is no next call. That is the shape of something with an end --
/// reading a process until it closes its pipe, walking a tree until there is
/// none left.
///
/// **It may block.** That is what the thread is for, and it is the reason to
/// choose this over a job: work that could be written as bounded steps should
/// be, because a step costs nothing and a thread costs a thread.
///
/// What it must still not do is take a lock across a call into the
/// interpreter, or assume the editor stood still -- both are the same rules
/// every other thread works under, and a thread of one's own relaxes neither.
pub trait ImmediateTask<B: BufferTrait>: Send + 'static {
    fn execute(self: Box<Self>, state: &EditorState<B>);
}

pub enum WorkerMessage<B: BufferTrait> {
    /// Start this now, on a thread of its own, and forget about it.
    ///
    /// "Now" is the literal claim it makes: each task gets its thread the
    /// moment its message is read, whatever the others are doing.
    RunNow(Box<dyn ImmediateTask<B>>),
}

pub struct BackgroundScheduler;

impl BackgroundScheduler {
    pub fn spawn<B: BufferTrait>(receiver: Receiver<WorkerMessage<B>>, state: EditorState<B>) {
        std::thread::spawn(move || {
            // The channel closing is what ends this thread: the last
            // `EditorState` going away drops the sender.
            while let Ok(WorkerMessage::RunNow(task)) = receiver.recv() {
                // A thread each, and nothing joins them. Two shell commands
                // really do run at the same time, and neither of them stops
                // the jobs on the WorkManager.
                //
                // `thread::spawn` rather than `Builder`, and so no handling
                // of a failure to start one: the standard library treats that
                // as unrecoverable and panics, which is the same thing this
                // file already relies on for itself. A recovery path would be
                // a second contract to keep correct for a condition under
                // which the editor is finished anyway.
                let task_state = state.clone();
                std::thread::spawn(move || {
                    // Off the command thread for its whole life: a prompt
                    // put up from here would be a window nobody is typing
                    // into.
                    let _off_the_command_thread = WorkerTurn::begin();
                    task.execute(&task_state);
                });
            }
        });
    }
}
