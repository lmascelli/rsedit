//! Where background work runs, and the one rule that decides which thread.
//!
//! # The rule
//!
//! > The shared thread may only run work that is guaranteed to return.
//!
//! Two kinds of work want to happen away from the thread the user is typing
//! on, and they are not the same kind:
//!
//! - **Bounded.** A Lisp fiber's turn, stopped by `worker-fuel` whether it
//!   wants to stop or not. A chunk of syntax colouring. These cannot run long
//!   -- not by convention, but because something cuts them off -- so any
//!   number of them can share one thread and take turns on a timer.
//!
//! - **Unbounded.** Reading a child process's output. Walking a directory
//!   tree. These take as long as the operating system takes, and no budget
//!   can shorten them. One of these on the shared thread stops every bounded
//!   task behind it.
//!
//! That second case was not hypothetical: `M-x compile` on a long build held
//! this thread for the whole build, and colouring, the prescanner, the file
//! watcher, the auto-saver and every Lisp worker and background job stopped
//! until it finished -- with a second shell command queued behind it, unable
//! to start. So the two kinds are now separated by which message carries
//! them, and the separation is the whole of what the two traits below mean.
//!
//! # What is in here
//!
//! This file: the two traits that say which kind a piece of work is, the
//! message that carries it, and the scheduler that runs it. [`worker`]: the
//! Lisp side, where a fiber becomes a bounded task and a named job becomes
//! either kind depending on what it was given.
//!
//! The rule above is the reason these two are one directory. They were
//! `task.rs` and `worker.rs` at the root of the crate, next to `search.rs` and
//! `input.rs`, and nothing said that one of them was the mechanism and the
//! other its only Lisp-facing user.
pub mod worker;

use std::{
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use crate::{background::worker::WorkerTurn, buffer::BufferTrait, editor::EditorState};

/// Work that runs to completion, on a thread of its own.
///
/// `self: Box<Self>` rather than `&mut self`: it is consumed, it keeps no
/// state between calls, and there is no next call. That is the shape of
/// something with an end -- reading a process until it closes its pipe,
/// walking a tree until there is none left.
///
/// **It may block.** That is what the thread is for, and it is the reason to
/// choose this trait over [`ScheduledTask`]: an implementation that could be
/// written as bounded turns should be, because a turn costs nothing and a
/// thread costs a thread.
///
/// What it must still not do is take a lock across a call into the
/// interpreter, or assume the editor stood still -- both are the same rules
/// every other thread works under, and a thread of one's own relaxes neither.
pub trait ImmediateTask<B: BufferTrait>: Send + 'static {
    fn execute(self: Box<Self>, state: &EditorState<B>);
}

/// Work that takes a turn every interval until it says it is finished.
///
/// Answering `false` ends it and it is dropped. Every implementation shares
/// one thread, which is what makes a turn cheap and what makes an overrunning
/// turn everybody's problem -- see the rule at the top of this file.
pub trait ScheduledTask<B: BufferTrait>: Send + 'static {
    fn execute(&mut self, state: &EditorState<B>) -> bool;
}

pub enum WorkerMessage<B: BufferTrait> {
    /// Start this now, on a thread of its own, and forget about it.
    ///
    /// "Now" is the literal claim it makes, and it could not make it before:
    /// this used to run inline on the scheduler, so it began only once
    /// whatever that thread was doing had finished, and held it until it was
    /// done itself.
    ///
    /// There is no variant for "run once, on the shared thread". Nothing
    /// wants one: work that is bounded and one-shot is a `background-call`
    /// with a function body, which is a [`ScheduledTask`] that answers
    /// `false` after its first turn and costs no thread at all.
    RunNow(Box<dyn ImmediateTask<B>>),
    /// Add this to the shared thread's list, woken every `interval`.
    Schedule {
        task: Box<dyn ScheduledTask<B>>,
        interval: Duration,
    },
}

pub struct Job<B: BufferTrait> {
    task: Box<dyn ScheduledTask<B>>,
    interval: Duration,
    next_run: Instant,
}

pub struct BackgroundScheduler;

impl BackgroundScheduler {
    pub fn spawn<B: BufferTrait>(receiver: Receiver<WorkerMessage<B>>, state: EditorState<B>) {
        std::thread::spawn(move || {
            let mut scheduled_jobs: Vec<Job<B>> = Vec::new();

            loop {
                let now = Instant::now();
                let time_to_next_job = scheduled_jobs
                    .iter()
                    .map(|job| job.next_run.saturating_duration_since(now))
                    .min();

                let msg_result = if let Some(timeout) = time_to_next_job {
                    receiver.recv_timeout(timeout)
                } else {
                    receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
                };

                match msg_result {
                    Ok(WorkerMessage::RunNow(task)) => {
                        // A thread each, and nothing joins them. Two shell
                        // commands really do run at the same time now, and
                        // neither of them stops the scheduled jobs below.
                        //
                        // `thread::spawn` rather than `Builder`, and so no
                        // handling of a failure to start one: the standard
                        // library treats that as unrecoverable and panics,
                        // which is the same thing this file already relies on
                        // for the scheduler itself. A recovery path would be
                        // a second contract to keep correct for a condition
                        // under which the editor is finished anyway -- and
                        // the obvious one, running the task here instead, is
                        // exactly the behaviour this replaced.
                        let task_state = state.clone();
                        std::thread::spawn(move || {
                            let _off_the_command_thread = WorkerTurn::begin();
                            task.execute(&task_state);
                        });
                    }

                    Ok(WorkerMessage::Schedule { task, interval }) => {
                        scheduled_jobs.push(Job {
                            task,
                            interval,
                            next_run: Instant::now() + interval,
                        });
                    }

                    // The channel closing is what ends this thread: the last
                    // `EditorState` going away drops the sender. There was a
                    // `Shutdown` message here too, unreachable because nothing
                    // could construct one -- which only went unnoticed while
                    // the mailbox was a public field and the compiler had to
                    // assume some caller outside might.
                    Err(RecvTimeoutError::Disconnected) => {
                        break;
                    }

                    Err(RecvTimeoutError::Timeout) => {}
                }

                let current_time = Instant::now();

                // Set once around the whole pass rather than inside each
                // task: the flag is a fact about the thread, and which thread
                // this is does not change between one job and the next.
                let _off_the_command_thread = WorkerTurn::begin();
                scheduled_jobs.retain_mut(|job| {
                    if job.next_run <= current_time {
                        let keep_running = job.task.execute(&state);
                        if keep_running {
                            job.next_run = current_time + job.interval;
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    }
                });
            }
        });
    }
}
