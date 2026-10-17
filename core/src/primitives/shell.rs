//! Running a shell command, and putting its output in a buffer.
//!
//! The first place this editor starts a process. Everything else it does is
//! its own work on its own data; this hands a string to a shell and reads back
//! whatever comes out.
//!
//! # Why it runs on a thread of its own
//!
//! A shell command takes as long as it takes. `git status` is instant, a build
//! is not, and `tail -f` never finishes at all. Running one on the thread that
//! reads the keyboard means the editor stops answering until it is done -- so
//! each command is read on a dedicated thread of the WorkManager (see
//! [`crate::work`]), named after the buffer it writes into, and the editor
//! carries on.
//!
//! What that costs is the obvious way of reporting completion. The thread has
//! an [`EditorState`] and no `Env`, so it cannot call a Lisp callback when the
//! command finishes, and it cannot set an echo message that expires on a timer
//! it does not run. It writes into the buffer it is already writing into
//! instead: a `--- exited 0 ---` trailer, which is also the only record that
//! survives being scrolled away from.
//!
//! # What is read, and how
//!
//! Standard output and standard error go into *one* pipe, so they arrive in
//! the order the command wrote them, as at a prompt. Read one after the other,
//! as they were, a command that wrote more than a pipe holds to standard error
//! before closing standard output -- a build full of warnings -- waited for
//! ever on a reader that was waiting for it.
//!
//! What comes out is bytes, shown as they arrive -- a line not ended yet, a
//! progress bar or a question, as far as it has got -- with anything that is
//! not UTF-8 replaced. Read as text, one byte of Latin-1 in a file name ended
//! the reading, and the command died of the pipe closing under it.
//!
//! # Stopping one
//!
//! A command is stopped by name -- `stop-worker` with its buffer's name -- or
//! by closing its buffer. It runs in a process group of its own, so the
//! request reaches everything it started and not only the shell: `TERM` to
//! the whole group, and `KILL` a moment later to whatever is still there. The
//! trailer then says `--- killed ---`.
//!
//! What has left the group is not reached: a daemon starts a session of its
//! own. One that kept the output it was given holds the reading open, and
//! the trailer waits until it lets go.
//!
//! A group of its own is also a group the terminal does not belong to. A
//! command has nothing to read from -- its standard input is empty -- and one
//! that opens the terminal itself, a password prompt, is stopped by the
//! system rather than taking the keystrokes meant for the editor. It waits
//! there until it is stopped from here, which is at once: the `TERM` is
//! followed by `CONT`, without which a stopped process would not hear it
//! before the `KILL`.
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
const GRACE: Duration = Duration::from_secs(2);

/// The most output appended in one go. What is already waiting in the pipe is
/// taken together, up to this, so that a command that prints fast fills its
/// buffer in a few edits rather than one per line.
const BATCH: usize = 64 * 1024;

/// Build the command that runs `command` through the platform's shell.
///
/// Through a shell rather than split into words here, because the whole value
/// of `M-!` is that what you type is what you would have typed at a prompt --
/// pipes, redirection, globs and all. Splitting on spaces would make
/// `grep -r "two words" .` mean something else, and re-implementing a shell's
/// quoting rules to avoid that is how you get a shell that is subtly not the
/// one the user knows.
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

/// Start COMMAND with its standard output and error going into one pipe, and
/// hand back the process and the pipe's reading end.
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

/// Send each of SIGNALS, in order, to every process in the group GROUP.
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

/// Ask PROCESS to stop: `TERM` to its whole group at once, and `KILL` after
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

/// Kill PROCESS, and its group with it, if it has not been waited for.
fn kill(process: &Process) {
    let mut guard = lock(process);
    if let Some(child) = guard.as_mut() {
        #[cfg(unix)]
        signal_group(child.id(), &["KILL"]);
        let _ = child.kill();
    }
}

/// Wait for PROCESS to end, and take it out: from here on, stopping it does
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

/// How many bytes at the end of BYTES are the start of a character that has
/// not arrived whole: what is kept for the next read, rather than show one
/// character as two broken ones.
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

/// Read what the command has written into INTO: whatever is waiting in the
/// pipe, as much as INTO holds, and wait only while nothing is. How many
/// bytes, `0` at the end of the output.
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
    /// Read the command's output into its buffer until it ends or is stopped,
    /// then wait for it and say how it went. ME is the thread's own number:
    /// what the buffer knows its writer by.
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

/// The process started for a command whose thread never ran: killed, and
/// waited for, here.
fn abandon(process: &Process) {
    kill(process);
    if let Some(mut child) = lock(process).take() {
        let _ = child.wait();
    }
}

pub const SHELL_COMMAND_START_DOC: &str = "(shell-command-start COMMAND &optional MODE): Run COMMAND \
         with the shell, in the background, and return the name of the buffer its output is \
         going into.\n\n\
         Returns immediately -- the editor stays usable while the command runs, and output \
         appears in the buffer as it is produced. A `--- exited 0 ---' line is written when it \
         finishes, which is how you tell a finished command from a slow one, and how you learn \
         whether it worked.\n\n\
         COMMAND goes to the shell as written, so pipes, redirection and quoting all mean what \
         they mean at a prompt. Standard output and standard error arrive together, in the \
         order the command wrote them; output that is not UTF-8 is shown with the bytes that \
         cannot be read replaced.\n\n\
         Each call gets a buffer of its own -- `*Shell Output*', then `*Shell Output*<2>' -- \
         because two commands sharing one would interleave their lines into something neither \
         of them said. Each is also read on a thread of its own, named after its buffer, so a \
         `git status' started during a build shows its output at once, and syntax colouring \
         and every other background job go on while a long command runs. The buffer is \
         read-only.\n\n\
         `(stop-worker BUFFER)' stops the command writing into BUFFER, and so does closing \
         BUFFER: the command and everything it started are sent TERM, and KILL two seconds \
         later if they are still there. The buffer then ends with `--- killed ---'.\n\n\
         MODE, if given, is the major mode the output buffer is put in, so a module can give the \
         transcript keys and colouring of its own -- which is the whole of how `compile' differs \
         from `M-!'. It defaults to `shell-output-mode'.\n\n\
         Returns nil (reporting it) if the command could not be started at all.\n\n\
         Example:\n\
         (shell-command-start \"git status --short\") => \"*Shell Output*\"\n\
         (shell-command-start \"cargo build\" \'compilation-mode)";

primitive!(shell_command_start, args, env, ctx, {
    let Some(ELispExp::String(command)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let mode = args::optional_name(args.get(1), "Symbol naming a mode")?
        .unwrap_or_else(|| "shell-output-mode".to_string());
    let (child, output) = match start(command) {
        Ok(started) => started,
        Err(why) => {
            ctx.set_echo_message(&format!("Cannot run the shell: {why}"));
            return Ok(ELispExp::nil());
        }
    };

    let name = ctx.free_name_for_work(OUTPUT_BUFFER);
    ctx.new_buffer(&name, None, Some(mode), &env);
    ctx.append_to_buffer(&name, &format!("$ {command}\n"));
    ctx.with_buffer_mut(&name, |buf| buf.read_only = true);

    let process: Process = Arc::new(Mutex::new(Some(child)));
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
        // Only as the editor shuts down: the task went without running, so
        // nothing reads the process and nothing will.
        ctx.finish_shell_command();
        abandon(&process);
        ctx.set_echo_message("The editor's background work has shut down");
        return Ok(ELispExp::nil());
    }
    Ok(ELispExp::string(name))
});

pub const SHELL_COMMAND_RUNNING_P_DOC: &str = "(shell-command-running-p): How many shell commands \
         are still running, or nil when none are.\n\n\
         A count rather than a flag because several can run at once. Mostly useful for a mode \
         line, and for asking before quitting.\n\n\
         Example:\n\
         (if (shell-command-running-p) (message \"still working\"))";

primitive!(shell_command_running_p, _args, _env, ctx, {
    let running = ctx.shell_commands_running();
    Ok(if running == 0 {
        ELispExp::nil()
    } else {
        ELispExp::number(running as f64)
    })
});

pub const SHELL_COMMAND_TO_STRING_DOC: &str = "(shell-command-to-string COMMAND): Run COMMAND with \
         the shell, wait for it, and return everything it wrote to standard output as a \
         string.\n\n\
         **This blocks the editor until the command finishes.** Use it for commands that are \
         bounded and quick -- `man ls', `git rev-parse HEAD' -- and `shell-command-start' for \
         anything that might not be. There is no timeout: `shell-command-to-string \\\"sleep \
         600\\\"' stops the editor for ten minutes, and that is the caller's to avoid.\n\n\
         Blocking is the point rather than an oversight. A caller that wants the output as a \
         *value* has nowhere to put an answer that arrives later: the thread reading a command \
         cannot call back into Lisp, so an asynchronous version of this could only write into a \
         buffer, which is what `shell-command-start' already does.\n\n\
         Standard error is not included, and a non-zero exit is not an error here -- whatever \
         was printed is returned either way. Returns nil (reporting it) if the command could \
         not be started at all.\n\n\
         Example:\n\
         (shell-command-to-string \\\"git rev-parse --short HEAD\\\")";

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

pub const PARSE_OVERSTRIKE_DOC: &str = "(parse-overstrike TEXT): TEXT with its terminal \
         overstrike removed, together with where the emphasis was. Returns (PLAIN SPANS), where \
         SPANS is a list of (START END KIND) over PLAIN and KIND is `bold' or `underline'.\n\n\
         `man' marks bold by printing a character, a backspace and the same character again \
         (`e\\be'), and underline by printing an underscore, a backspace and the character \
         (`_\\be'). It is how emphasis was done on a printer that could only strike the same \
         spot twice, and it is still what comes out of `man' today. Left in, every emphasised \
         word is unreadable.\n\n\
         Both halves come from one pass, and both are returned. An earlier version of this threw \
         the spans away and kept only the text, which made the emphasis unrecoverable without \
         parsing the whole thing a second time -- and two passes over the same string have to \
         agree about offsets, which is exactly the sort of thing that disagrees on a multi-byte \
         character.\n\n\
         Offsets are in characters, into PLAIN, so they can be handed straight to \
         `make-overlay'.\n\n\
         Example:\n\
         (parse-overstrike page) => (\"NAME\" ((0 4 bold)))";

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

/// Register this module's primitives: running a child process and collecting its output.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command(
        "shell-command-start",
        shell_command_start,
        &["sShell command: "],
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
