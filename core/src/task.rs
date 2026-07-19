use std::{
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use crate::{buffer::BufferTrait, editor::EditorState};

pub trait ImmediateTask<B: BufferTrait>: Send + 'static {
    fn execute(self: Box<Self>, state: &EditorState<B>);
}

pub trait ScheduledTask<B: BufferTrait>: Send + 'static {
    fn execute(&mut self, state: &EditorState<B>) -> bool;
}

pub enum WorkerMessage<B: BufferTrait> {
    RunNow(Box<dyn ImmediateTask<B>>),
    Schedule {
        task: Box<dyn ScheduledTask<B>>,
        interval: Duration,
    },
    Shutdown,
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
                        task.execute(&state);
                    }

                    Ok(WorkerMessage::Schedule { task, interval }) => {
                        scheduled_jobs.push(Job {
                            task,
                            interval,
                            next_run: Instant::now() + interval,
                        });
                    }

                    Ok(WorkerMessage::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                        break;
                    }

                    Err(RecvTimeoutError::Timeout) => {}
                }

                let current_time = Instant::now();

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
