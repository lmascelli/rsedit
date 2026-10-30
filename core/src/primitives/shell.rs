//! Running a shell command: in the background, its output appended to a
//! buffer as it comes, by a thread of its own that `stop-worker` or closing
//! the buffer stops; or at once, for its output as a string.
//!
//! Design: `doc/architettura-editor.typ, <comandi-shell>`.
use super::*;
use crate::work::{Stop, WorkId};
use std::io::{ErrorKind, PipeReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// The buffer the first shell command writes into.
const OUTPUT_BUFFER: &str = "*Shell Output*";

/// How long a command asked to stop is given to go before it is killed.
///
/// Long enough for a build to tidy up after `TERM` -- `git` removes its lock
/// files, `make` its half-written targets -- and short enough that a command
/// that ignores it is gone before anybody wonders why it has not stopped.
/// Unix only: elsewhere there is no `TERM` to give it after.
#[cfg(unix)]
const GRACE: Duration = Duration::from_secs(2);

/// The most output appended in one go. What is already waiting in the pipe is
/// taken together, up to this, so that a command that prints fast fills its
/// buffer in a few edits rather than one per line.
const BATCH: usize = 64 * 1024;

/// Builds the command that runs COMMAND through the platform's shell, so
/// that what is typed means what it would at a prompt: pipes, redirection,
/// globs and all.
fn shell_invocation(command: &str) -> Command {
    #[cfg(windows)]
    let mut child = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(command);
        c
    };
    // Nothing types at it. Without this a command that reads standard input
    // inherits the terminal and fights the editor for keystrokes.
    child.stdin(Stdio::null());
    child
}

/// Starts COMMAND with its standard output and error going into one pipe, and
/// returns the process and the pipe's reading end.
fn start(command: &str) -> std::io::Result<(Child, PipeReader)> {
    let (output, writer) = std::io::pipe()?;
    let mut invocation = shell_invocation(command);
    invocation.stdout(writer.try_clone()?).stderr(writer);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A group of its own, led by the shell: what stopping it reaches.
        invocation.process_group(0);
    }
    let child = invocation.spawn()?;
    // The writing ends the command still holds go with it. Until they have,
    // the pipe never ends, however long ago the process did.
    drop(invocation);
    Ok((child, output))
}

/// The process, for as long as it has not been waited for: shared between the
/// thread reading it and whoever asks it to stop.
///
/// Waited for under the same lock that a signal is sent under. Once a process
/// has been waited for, its number is free to be given to another, and a
/// signal sent to that number then would reach a stranger.
type Process = Arc<Mutex<Option<Child>>>;

fn lock(process: &Process) -> std::sync::MutexGuard<'_, Option<Child>> {
    // A poisoned lock still guards a process that has to be stopped.
    process.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sends each of SIGNALS, in order, to every process in the group GROUP.
///
/// Through the shell's own `kill`, because the standard library can signal a
/// process but not a group -- and the shell is what the command runs under
/// anyway. In the form POSIX gives, `-s` and then `--` before the negative
/// number: `dash`, the `/bin/sh` of Debian, takes `kill -TERM -- -123` for a
/// number it cannot read, and signals nothing.
#[cfg(unix)]
fn signal_group(group: u32, signals: &[&str]) {
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

// TODO(SHL-5): the `KILL` comes from a thread outside the WorkManager, and an
// editor that exits before it is sent leaves a command that ignores `TERM`
// running.
/// Asks PROCESS to stop: `TERM` to its whole group at once, and `KILL` after
/// [`GRACE`] to whatever of it is still there. `CONT` after the `TERM`, for
/// whatever of it the system had stopped: until it is continued, a stopped
/// process does not hear the `TERM`.
///
/// What [`Stop::on_cancel`] runs, on the thread that asked: so it waits for
/// nothing. The `KILL` comes from a thread of its own, which finds nothing to
/// do when the command went on the `TERM`.
fn stop_process(process: &Process) {
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
            .name("stop shell".into())
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

/// Kills PROCESS, and its group with it, if it has not been waited for.
fn kill(process: &Process) {
    let mut guard = lock(process);
    if let Some(child) = guard.as_mut() {
        #[cfg(unix)]
        signal_group(child.id(), &["KILL"]);
        let _ = child.kill();
    }
}

/// Waits for PROCESS to end, and takes it out: from here on, stopping it does
/// nothing.
///
/// A short sleep between looks rather than a blocking wait, so that the lock
/// is free for whoever wants to stop it meanwhile. By the time its output has
/// ended a process has almost always ended too, and the first look finds it.
fn reap(process: &Process) -> std::io::Result<ExitStatus> {
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
fn incomplete_tail(bytes: &[u8]) -> usize {
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

/// Reads what the command has written into INTO -- whatever is waiting in the
/// pipe, as much as INTO holds, waiting only while nothing is -- and returns
/// how many bytes, `0` at the end of the output.
///
/// Not a line at a time: what does not end a line -- a progress bar, a
/// question asked before reading the empty input -- is shown as it comes,
/// rather than when the line ends, which may be never.
fn read_some(output: &mut PipeReader, into: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match output.read(into) {
            // A signal came while it waited, and nothing was read.
            Err(why) if why.kind() == ErrorKind::Interrupted => {}
            read => return read,
        }
    }
}

/// One command, and the buffer it writes into.
struct ShellCommand {
    process: Process,
    output: PipeReader,
    buffer: String,
}

/// Takes one off the count of running commands when it goes, however the
/// thread ends.
struct Running<'a, B: BufferTrait>(&'a EditorState<B>);

impl<B: BufferTrait> Drop for Running<'_, B> {
    fn drop(&mut self) {
        self.0.finish_shell_command();
    }
}

impl ShellCommand {
    /// Reads the command's output into its buffer until it ends or is
    /// stopped, then waits for it and says how it went. ME is the thread's own
    /// number: what the buffer knows its writer by.
    fn run<B: BufferTrait>(self, state: &EditorState<B>, stop: &Stop, me: WorkId) {
        // Last of all, after the trailer: `shell-command-running-p` must not
        // say it is over before the buffer does.
        let _running = Running(state);
        let ShellCommand {
            process,
            mut output,
            buffer,
        } = self;
        stop.on_cancel({
            let process = process.clone();
            move || stop_process(&process)
        });
        let write = |text: &str| {
            if state.append_as_writer(&buffer, me, text) {
                state.changed_in_background();
            }
        };
        // Whether what has been written ends a line, so that the trailer
        // never lands at the end of somebody else's.
        let mut at_line_start = true;
        let mut batch = vec![0; BATCH];
        // Read and not shown yet: the start of a character whose end has not
        // arrived.
        let mut pending = Vec::new();
        while !stop.is_cancelled() {
            match read_some(&mut output, &mut batch) {
                Ok(0) => break,
                Ok(read) => {
                    pending.extend_from_slice(&batch[..read]);
                    let cut = pending.len() - incomplete_tail(&pending);
                    let text = String::from_utf8_lossy(&pending[..cut]);
                    if !text.is_empty() {
                        at_line_start = text.ends_with('\n');
                        write(&text);
                    }
                    pending.drain(..cut);
                }
                Err(why) => {
                    let gap = if at_line_start { "" } else { "\n" };
                    write(&format!("{gap}[the output could not be read: {why}]\n"));
                    at_line_start = true;
                    stop_process(&process);
                    break;
                }
            }
        }
        if !pending.is_empty() && !stop.is_cancelled() {
            let text = String::from_utf8_lossy(&pending);
            at_line_start = text.ends_with('\n');
            write(&text);
        }
        // Closed before the wait: anything of it still writing finds nobody
        // reading, and goes.
        drop(output);
        let status = reap(&process);
        let trailer = if stop.is_cancelled() {
            "--- killed ---\n".to_string()
        } else {
            match status {
                Ok(status) => match status.code() {
                    Some(code) => format!("--- exited {code} ---\n"),
                    // Killed by a signal, which has no exit code to report.
                    None => "--- killed ---\n".to_string(),
                },
                Err(why) => format!("--- could not be waited for: {why} ---\n"),
            }
        };
        let gap = if at_line_start { "" } else { "\n" };
        write(&format!("{gap}{trailer}"));
    }
}

/// Kills, and waits for, the process of a command whose thread never ran.
fn abandon(process: &Process) {
    kill(process);
    if let Some(mut child) = lock(process).take() {
        let _ = child.wait();
    }
}

pub const SHELL_COMMAND_START_DOC: &str = "(shell-command-start COMMAND &optional MODE PRODUCER): Run \
         COMMAND, a string, with the shell in the background, and return at once the name of the \
         buffer its output goes into. The buffer is read-only and in major mode MODE, \
         `shell-output-mode' unless given; its first line is the command, and its last says how \
         it ended, as --- exited 0 ---. Each call has a buffer of its own; with PRODUCER, a \
         symbol such as compile, the newest buffer that PRODUCER filled in MODE is emptied and \
         used again when `results-reuse-buffer' is set, and the command still writing into it \
         is stopped. `stop-worker' with the buffer's name stops the command, and so does \
         closing the buffer. Return nil, saying why, when the shell cannot be started.\n\n\
         Example:\n\
         (shell-command-start \"git status --short\")\n\
         (shell-command-start \"cargo build\" 'compilation-mode)";

primitive!(shell_command_start, args, env, ctx, {
    let Some(ELispExp::String(command)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let mode = args::optional_name(args.get(1), "Symbol naming a mode")?
        .unwrap_or_else(|| "shell-output-mode".to_string());
    let producer = args::optional_name(args.get(2), "Symbol naming a producer")?;
    let (child, output) = match start(command) {
        Ok(started) => started,
        Err(why) => {
            ctx.set_echo_message(&format!("Cannot run the shell: {why}"));
            return Ok(ELispExp::nil());
        }
    };

    let process: Process = Arc::new(Mutex::new(Some(child)));

    let name = if let Some(producer) = producer {
        ctx.prepare_results_buffer(&producer, OUTPUT_BUFFER, &mode, None, &env)
    } else {
        let name = ctx.free_name_for_work(OUTPUT_BUFFER);

        ctx.new_buffer(&name, None, Some(mode), &env)
            .filter(|name| ctx.has_buffer(name))
    };

    let Some(name) = name else {
        abandon(&process);
        return Ok(ELispExp::nil());
    };

    ctx.append_to_buffer(&name, &format!("$ {command}\n"));

    ctx.with_buffer_mut(&name, |buf| {
        buf.read_only = true;
    });

    let task = ShellCommand {
        process: process.clone(),
        output,
        buffer: name.clone(),
    };
    // Counted before the thread starts, so that the count is up the moment
    // this returns rather than once the thread gets going.
    ctx.begin_shell_command();
    if ctx
        .spawn_writer(&name, move |state, stop, me| task.run(state, stop, me))
        .is_none()
    {
        // Refused once the editor has shut down, or when the system gives no
        // thread: the task went without running, so nothing reads the process
        // and nothing will.
        ctx.finish_shell_command();
        abandon(&process);
        // TODO(DOC-1): the message speaks of a shutdown in both cases.
        ctx.set_echo_message("The editor's background work has shut down");
        return Ok(ELispExp::nil());
    }
    Ok(ELispExp::string(name))
});

pub const SHELL_COMMAND_RUNNING_P_DOC: &str = "(shell-command-running-p): Return how many shell \
         commands are running, or nil when none is.\n\n\
         Example:\n\
         (if (shell-command-running-p) (message \"A command is still running\"))";

primitive!(shell_command_running_p, _args, _env, ctx, {
    let running = ctx.shell_commands_running();
    Ok(if running == 0 {
        ELispExp::nil()
    } else {
        ELispExp::number(running as f64)
    })
});

pub const SHELL_COMMAND_TO_STRING_DOC: &str = "(shell-command-to-string COMMAND): Run COMMAND \
         with the shell, wait for it to end, and return what it printed on its standard output, \
         newline and all; nil, saying why, when the shell cannot be started. What it printed on \
         standard error is left out, and how it ended makes no difference. The editor waits with \
         it, so it is for a command that answers at once: `shell-command-start' runs one that may \
         not.\n\n\
         Example:\n\
         (shell-command-to-string \"git rev-parse --short HEAD\")";

primitive!(shell_command_to_string, args, _env, ctx, {
    let Some(ELispExp::String(command)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    match shell_invocation(command).output() {
        Ok(output) => Ok(ELispExp::string(
            String::from_utf8_lossy(&output.stdout).to_string(),
        )),
        Err(why) => {
            ctx.set_echo_message(&format!("Cannot run the shell: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

pub const PARSE_OVERSTRIKE_DOC: &str = "(parse-overstrike TEXT): Return TEXT without the \
         overstrike man prints for a terminal, and where it was, as (PLAIN SPANS). A character, a \
         backspace and the same character again is bold; an underscore, a backspace and a \
         character is underlined. Each of SPANS is (START END bold) or (START END underline), a \
         run of characters counted in PLAIN from 0.\n\n\
         Example:\n\
         (parse-overstrike (shell-command-to-string \"printf 'N\\\\bNA\\\\bAM\\\\bME\\\\bE'\"))\n\
         => (\"NAME\" ((0 4 bold)))";

primitive!(parse_overstrike, args, _env, _ctx, {
    let Some(ELispExp::String(text)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };

    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Bold,
        Underline,
    }

    let mut plain = String::with_capacity(text.len());
    // What each character of `plain` is emphasised as, by character index.
    let mut marks: Vec<Option<Kind>> = Vec::new();
    // The character just written, so a backspace knows what it is undoing.
    let mut previous: Option<char> = None;

    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{8}' {
            plain.push(c);
            marks.push(None);
            previous = Some(c);
            continue;
        }
        // A backspace says the last character did not count. What replaces it
        // says which kind of emphasis this was: the same character again is
        // bold, an underscore before it was underline.
        let Some(struck) = previous else {
            // A backspace with nothing before it. Nothing to undo, and nothing
            // to mark.
            continue;
        };
        plain.pop();
        marks.pop();
        let Some(&next) = chars.peek() else {
            // Trailing backspace: the character it removed is simply gone.
            previous = None;
            continue;
        };
        chars.next();
        let kind = if struck == '_' && next != '_' {
            Kind::Underline
        } else if next == '_' && struck != '_' {
            // `x\b_`, which some formatters emit for the same thing.
            Kind::Underline
        } else {
            Kind::Bold
        };
        // The character that survives is the one that is not the underscore,
        // so that an underlined `e` reads as `e` rather than as `_`.
        let shown = if kind == Kind::Underline && next == '_' {
            struck
        } else {
            next
        };
        plain.push(shown);
        marks.push(Some(kind));
        previous = Some(shown);
    }

    // Runs of the same kind become one span, because a word emphasised letter
    // by letter is one emphasised word -- and an overlay per character would
    // be a thousand overlays for a manual page.
    let mut spans = Vec::new();
    let mut run: Option<(usize, Kind)> = None;
    for (index, mark) in marks.iter().enumerate() {
        match (run, mark) {
            (Some((_, kind)), Some(here)) if kind == *here => {}
            (Some((start, kind)), _) => {
                spans.push((start, index, kind));
                run = mark.map(|kind| (index, kind));
            }
            (None, Some(kind)) => run = Some((index, *kind)),
            (None, None) => {}
        }
    }
    if let Some((start, kind)) = run {
        spans.push((start, marks.len(), kind));
    }

    let spans = spans
        .into_iter()
        .map(|(start, end, kind)| {
            ELispExp::proper_list(vec![
                ELispExp::number(start as f64),
                ELispExp::number(end as f64),
                ELispExp::symbol(
                    match kind {
                        Kind::Bold => "bold",
                        Kind::Underline => "underline",
                    }
                    .to_string(),
                ),
            ])
        })
        .collect();

    Ok(ELispExp::proper_list(vec![
        ELispExp::string(plain),
        ELispExp::proper_list(spans),
    ]))
});

/// Registers the primitives that run a shell command.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // A function and not a command: run from M-x it would start the command
    // and show nothing. `shell-command` is the command that shows it.
    into.function(
        "shell-command-start",
        shell_command_start,
        SHELL_COMMAND_START_DOC,
    );
    into.function(
        "shell-command-to-string",
        shell_command_to_string,
        SHELL_COMMAND_TO_STRING_DOC,
    );
    into.function("parse-overstrike", parse_overstrike, PARSE_OVERSTRIKE_DOC);
    into.function(
        "shell-command-running-p",
        shell_command_running_p,
        SHELL_COMMAND_RUNNING_P_DOC,
    );
}
