//! Running a shell command, and putting its output in a buffer.
//!
//! The first place this editor starts a process. Everything else it does is
//! its own work on its own data; this hands a string to a shell and reads back
//! whatever comes out.
//!
//! # Why it runs on the worker thread
//!
//! A shell command takes as long as it takes. `git status` is instant, a build
//! is not, and `tail -f` never finishes at all. Running one on the thread that
//! reads the keyboard means the editor stops answering until it is done -- so
//! the command is handed to the background worker (see [`crate::task`]) and the
//! editor carries on.
//!
//! What that costs is the obvious way of reporting completion. The worker is
//! given an [`EditorState`] and no `Env`, so it cannot call a Lisp callback
//! when the command finishes, and it cannot set an echo message that expires on
//! a timer it does not run. It writes into the buffer it is already writing
//! into instead: a `--- exited 0 ---` trailer, which is also the only record
//! that survives being scrolled away from.
//!
//! # Why output arrives as it is produced
//!
//! Reading the whole of a build's output and then showing it is simpler, and it
//! makes a long command look like a hung editor. Lines are appended as they
//! come, which means the buffer is also how you tell a slow command from a
//! stuck one.
use super::*;
use crate::task::{ImmediateTask, WorkerMessage};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};

/// The buffer the first shell command writes into.
const OUTPUT_BUFFER: &str = "*Shell Output*";

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
    child
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Nothing types at it. Without this a command that reads standard
        // input inherits the terminal and fights the editor for keystrokes.
        .stdin(Stdio::null());
    child
}

/// A name no buffer currently has: `*Shell Output*`, then `<2>`, `<3>`...
///
/// Several commands may be running at once, and one buffer between them would
/// interleave their lines into something neither of them said.
fn free_output_name<B: BufferTrait>(ctx: &EditorState<B>) -> String {
    if ctx.get_buffer(OUTPUT_BUFFER).is_none() {
        return OUTPUT_BUFFER.to_string();
    }
    for n in 2.. {
        let candidate = format!("{OUTPUT_BUFFER}<{n}>");
        if ctx.get_buffer(&candidate).is_none() {
            return candidate;
        }
    }
    unreachable!("the loop above returns")
}

/// Append `text` to the buffer called `name`, read-only or not.
///
/// The buffer is read-only so that nobody types into a transcript. The flag is
/// turned off and back on *inside* one `mutate_buffer` call, so the write lock
/// is held across the whole of it and there is no moment when the buffer is
/// both visible and writable. `dired` does the same thing from Lisp, where it
/// cannot hold a lock and has to trust that nothing runs in between.
fn append<B: BufferTrait>(ctx: &EditorState<B>, name: &str, text: &str) {
    let Some(handle) = ctx.get_buffer(name) else {
        // The user killed the output buffer while the command was running,
        // which is a perfectly reasonable thing to have done.
        return;
    };
    let written = ctx.mutate_buffer(handle, |buf| {
        let was_read_only = buf.read_only;
        buf.read_only = false;
        let at = buf.text.len();
        // Cannot fail here -- the only thing `insert_text` refuses is a
        // read-only buffer, and the flag was just cleared under this same
        // lock. Checked rather than discarded so that if it ever grows another
        // reason to refuse, the output going missing is reported instead of
        // silently not appearing.
        let written = crate::primitives::edits::insert_text(buf, at, text);
        buf.read_only = was_read_only;
        written
    });
    if !written {
        ctx.log_diagnostic(&format!("Could not append shell output to {name}"));
    }
}

/// Reading one command's output into one buffer, on the worker thread.
struct ShellTask {
    child: Child,
    buffer: String,
}

impl<B: BufferTrait> ImmediateTask<B> for ShellTask {
    fn execute(mut self: Box<Self>, state: &EditorState<B>) {
        // stdout as it arrives, so a slow command shows its progress.
        if let Some(stdout) = self.child.stdout.take() {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => append(state, &self.buffer, &format!("{line}\n")),
                    // Output that is not UTF-8. Said once and then dropped:
                    // the alternative is a buffer of replacement characters
                    // that looks like the command's own output.
                    Err(why) => {
                        append(state, &self.buffer, &format!("[unreadable output: {why}]\n"));
                        break;
                    }
                }
            }
        }
        // Then stderr, whole. Kept separate and put after rather than merged
        // as it arrives: merging needs a second reader thread, and a failure
        // message buried in the middle of a build log is one nobody finds.
        // The cost is that the two are out of order relative to each other,
        // which matters less than either of them being lost.
        if let Some(mut stderr) = self.child.stderr.take() {
            let mut text = String::new();
            if stderr.read_to_string(&mut text).is_ok() && !text.is_empty() {
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                append(state, &self.buffer, &text);
            }
        }
        let status = match self.child.wait() {
            Ok(status) => match status.code() {
                Some(code) => format!("--- exited {code} ---\n"),
                // Killed by a signal, which has no exit code to report.
                None => "--- killed ---\n".to_string(),
            },
            Err(why) => format!("--- could not be waited for: {why} ---\n"),
        };
        append(state, &self.buffer, &status);
        // Last, and after the trailer is in the buffer: this is what stops the
        // renderer waking up on a timer, and it must not stop before the thing
        // it was waking up to see has been written.
        state.finish_shell_command();
    }
}

pub const SHELL_COMMAND_START_DOC: &str = "(shell-command-start COMMAND): Run COMMAND with the \
         shell, in the background, and return the name of the buffer its output is going \
         into.\n\n\
         Returns immediately -- the editor stays usable while the command runs, and output \
         appears in the buffer as it is produced. A `--- exited 0 ---' line is written when it \
         finishes, which is how you tell a finished command from a slow one, and how you learn \
         whether it worked.\n\n\
         COMMAND goes to the shell as written, so pipes, redirection and quoting all mean what \
         they mean at a prompt.\n\n\
         Each call gets a buffer of its own -- `*Shell Output*', then `*Shell Output*<2>' -- \
         because two commands sharing one would interleave their lines into something neither \
         of them said. The buffer is read-only; killing it while the command runs is allowed, \
         and the rest of the output is then dropped.\n\n\
         Returns nil (reporting it) if the command could not be started at all.\n\n\
         Example:\n\
         (shell-command-start \"git status --short\") => \"*Shell Output*\"";

primitive!(shell_command_start, args, _env, ctx, {
    let Some(ELispExp::String(command)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let child = match shell_invocation(command).spawn() {
        Ok(child) => child,
        Err(why) => {
            ctx.set_echo_message(&format!("Cannot run the shell: {why}"));
            return Ok(ELispExp::nil());
        }
    };

    let name = free_output_name(ctx);
    ctx.new_buffer(&name, None, Some("shell-output-mode".to_string()));
    append(ctx, &name, &format!("$ {command}\n"));
    if let Some(handle) = ctx.get_buffer(&name) {
        ctx.mutate_buffer(handle, |buf| buf.read_only = true);
    }

    // Counted *before* the task is sent, not inside it: the worker may not
    // reach it for a moment, and a frame drawn in that moment would decide
    // nothing was running and go back to sleep until a key was pressed.
    ctx.begin_shell_command();
    if ctx
        .worker_mailbox
        .send(WorkerMessage::RunNow(Box::new(ShellTask {
            child,
            buffer: name.clone(),
        })))
        .is_err()
    {
        ctx.finish_shell_command();
        ctx.set_echo_message("The background worker has gone");
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
         *value* has nowhere to put an answer that arrives later: the background worker cannot \
         call back into Lisp, so an asynchronous version of this could only write into a buffer, \
         which is what `shell-command-start' already does.\n\n\
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

pub const STRIP_OVERSTRIKE_DOC: &str = "(strip-overstrike TEXT): TEXT with terminal overstrike \
         sequences removed.\n\n\
         `man' marks bold by printing a character, a backspace and the same character again \
         (`e\\\\be'), and underline by printing an underscore, a backspace and the character \
         (`_\\\\be'). It is how formatting was done on a printer that could only strike the same \
         spot twice, and it is still what comes out of `man' today. Left in, every emphasised \
         word is unreadable.\n\n\
         The formatting is dropped rather than translated, because this editor gives text a \
         face through its mode's syntax rules and has no way to face an arbitrary span. A mode \
         showing this output recovers the emphasis from the structure -- a heading is a heading \
         because of where it sits, not because `man' doubled its letters.\n\n\
         Example:\n\
         (strip-overstrike \\\"N\\\\bNA\\\\bAM\\\\bME\\\\bE\\\") => \\\"NAME\\\"";

primitive!(strip_overstrike, args, _env, _ctx, {
    let Some(ELispExp::String(text)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\u{8}' {
            // The backspace says "what I printed last did not count". Dropping
            // the character before it is the whole of the rule, and it is why
            // this cannot be done with a regexp over pairs: `_\bx` and `x\bx`
            // are the same operation on different inputs.
            out.pop();
        } else {
            out.push(c);
        }
    }
    Ok(ELispExp::string(out))
});
