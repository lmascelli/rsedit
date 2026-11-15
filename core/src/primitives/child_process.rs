//! What every process the editor starts shares, whoever reads it: a group of
//! its own, stopping it -- `TERM`, then `KILL` after a grace -- waiting for it
//! without keeping a stop out, killing what is left at the editor's exit, and
//! reading its output a piece at a time. `shell` and `processes` use it.
//!
//! Design: `doc/architettura-editor.typ, <processi-figli>`.
use crate::{BufferTrait, EditorState};
use std::io::{ErrorKind, Read};
use std::process::{Command, ExitStatus};
use std::time::Duration;

/// How long a process asked to stop is given to go before it is killed.
///
/// Long enough for a build to tidy up after `TERM` -- `git` removes its lock
/// files, `make` its half-written targets -- and short enough that a process
/// that ignores it is gone before anybody wonders why it has not stopped.
/// Unix only: elsewhere there is no `TERM` to give it after.
#[cfg(unix)]
const GRACE: Duration = Duration::from_secs(2);

/// The most output taken in one read. What is already waiting in the pipe is
/// taken together, up to this, so that a process that prints fast is read in
/// a few pieces rather than one per line.
pub(crate) const BATCH: usize = 64 * 1024;

/// The process, for as long as it has not been waited for: shared between the
/// threads reading it and whoever asks it to stop.
///
/// Waited for under the same lock that a signal is sent under. Once a process
/// has been waited for, its number is free to be given to another, and a
/// signal sent to that number then would reach a stranger.
pub(crate) type Process = crate::editor::ChildProcess;

fn lock(process: &Process) -> std::sync::MutexGuard<'_, Option<std::process::Child>> {
    // A poisoned lock still guards a process that has to be stopped.
    process
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Has COMMAND start in a process group of its own, which it leads: what
/// [`stop_process`] reaches, with all it starts. Unix only: elsewhere a
/// process is stopped alone.
pub(crate) fn in_own_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// Sends each of SIGNALS, in order, to every process in the group GROUP.
///
/// Through the shell's own `kill`, because the standard library can signal a
/// process but not a group. In the form POSIX gives, `-s` and then `--` before
/// the negative number: `dash`, the `/bin/sh` of Debian, takes
/// `kill -TERM -- -123` for a number it cannot read, and signals nothing.
#[cfg(unix)]
fn signal_group(group: u32, signals: &[&str]) {
    use std::process::Stdio;
    let script: Vec<String> = signals
        .iter()
        .map(|signal| format!("kill -s {signal} -- -{group}"))
        .collect();
    let _ = Command::new("/bin/sh")
        .arg("-c")
        .arg(script.join("; "))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Asks PROCESS to stop: `TERM` to its whole group at once, and `KILL` after
/// [`GRACE`] to whatever of it is still there. `CONT` after the `TERM`, for
/// whatever of it the system had stopped: until it is continued, a stopped
/// process does not hear the `TERM`. Elsewhere than on Unix, it is killed.
///
/// What [`Stop::on_cancel`](crate::work::Stop::on_cancel) runs, on the thread
/// that asked: so it waits for nothing. The `KILL` comes from a thread of its
/// own, which finds nothing to do when the process went on the `TERM`. An
/// editor that exits sooner than that takes the thread with it, and kills what
/// is left itself: see [`kill_left_over`].
pub(crate) fn stop_process(process: &Process) {
    #[cfg(unix)]
    {
        {
            // Under the lock: see [`Process`].
            let guard = lock(process);
            let Some(child) = guard.as_ref() else {
                return;
            };
            signal_group(child.id(), &["TERM", "CONT"]);
        }
        let later = process.clone();
        let finishing = std::thread::Builder::new()
            .name("stop process".into())
            .spawn(move || {
                std::thread::sleep(GRACE);
                kill(&later);
            });
        if finishing.is_err() {
            kill(process);
        }
    }
    #[cfg(not(unix))]
    kill(process);
}

/// Kills, with their groups, the processes of CTX that have not ended: at the
/// end of [`EditorState::shutdown`], once the wait for them to go on their
/// `TERM` is over. Without it, one that ignored `TERM` outlived the editor, in
/// a group no terminal was watching, since the `KILL` after [`GRACE`] comes
/// later than an exit.
pub(crate) fn kill_left_over<B: BufferTrait>(ctx: &EditorState<B>) {
    for process in ctx.child_processes() {
        kill(&process);
    }
}

/// Kills PROCESS, and its group with it, if it has not been waited for.
pub(crate) fn kill(process: &Process) {
    let mut guard = lock(process);
    if let Some(child) = guard.as_mut() {
        #[cfg(unix)]
        signal_group(child.id(), &["KILL"]);
        let _ = child.kill();
    }
}

/// Kills, and waits for, the process of a reader that never ran.
pub(crate) fn abandon(process: &Process) {
    kill(process);
    if let Some(mut child) = lock(process).take() {
        let _ = child.wait();
    }
}

/// Waits for PROCESS to end, and takes it out: from here on, stopping it does
/// nothing.
///
/// A short sleep between looks rather than a blocking wait, so that the lock
/// is free for whoever wants to stop it meanwhile. By the time its output has
/// ended a process has almost always ended too, and the first look finds it.
pub(crate) fn reap(process: &Process) -> std::io::Result<ExitStatus> {
    let mut pause = Duration::from_millis(1);
    loop {
        {
            let mut guard = lock(process);
            let Some(child) = guard.as_mut() else {
                return Err(std::io::Error::other("it has already been waited for"));
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    guard.take();
                    return Ok(status);
                }
                Ok(None) => {}
                Err(why) => {
                    guard.take();
                    return Err(why);
                }
            }
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

/// Returns how many bytes at the end of BYTES are the start of a character
/// that has not arrived whole: what is kept for the next read, rather than
/// show one character as two broken ones.
pub(crate) fn incomplete_tail(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(4) {
        let byte = bytes[bytes.len() - back];
        // Continuation bytes are 10xxxxxx: walk back over them to the first.
        if byte & 0b1100_0000 != 0b1000_0000 {
            let length = match byte {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => 1,
            };
            return if length > back { back } else { 0 };
        }
    }
    0
}

/// Reads what the process has written into INTO -- whatever is waiting in the
/// pipe, as much as INTO holds, waiting only while nothing is -- and returns
/// how many bytes, `0` at the end of the output.
///
/// Not a line at a time: what does not end a line -- a progress bar, a
/// question asked before reading the input -- is had as it comes, rather than
/// when the line ends, which may be never.
pub(crate) fn read_some(output: &mut impl Read, into: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match output.read(into) {
            // A signal came while it waited, and nothing was read.
            Err(why) if why.kind() == ErrorKind::Interrupted => {}
            read => return read,
        }
    }
}

/// Returns what to say when PROGRAM could not be started in DIRECTORY, for
/// WHY. A directory that has gone since `cd` is said as such: the system's own
/// answer, "No such file or directory", would seem to be about the program.
pub(crate) fn cannot_run(program: &str, directory: &str, why: &std::io::Error) -> String {
    if std::path::Path::new(directory).is_dir() {
        format!("Cannot run {program}: {why}")
    } else {
        format!("Cannot run {program} in {directory}: it is not a directory")
    }
}
