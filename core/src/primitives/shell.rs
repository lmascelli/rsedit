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
    if !ctx.has_buffer(OUTPUT_BUFFER) {
        return OUTPUT_BUFFER.to_string();
    }
    for n in 2.. {
        let candidate = format!("{OUTPUT_BUFFER}<{n}>");
        if !ctx.has_buffer(&candidate) {
            return candidate;
        }
    }
    unreachable!("the loop above returns")
}

/// Append `text` to the buffer called `name`, read-only or not.
///
/// The buffer is read-only so that nobody types into a transcript. The flag is
/// turned off and back on *inside* one `with_buffer_mut` call, so the write
/// lock is held across the whole of it and there is no moment when the buffer
/// is both visible and writable. `dired` does the same thing from Lisp, where
/// it cannot hold a lock and has to trust that nothing runs in between.
fn append<B: BufferTrait>(ctx: &EditorState<B>, name: &str, text: &str) {
    // `None` when the user killed the output buffer while the command was
    // running, which is a perfectly reasonable thing to have done.
    let written = ctx.with_buffer_mut(name, |buf| {
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
    if written == Some(false) {
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
                        append(
                            state,
                            &self.buffer,
                            &format!("[unreadable output: {why}]\n"),
                        );
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
         MODE, if given, is the major mode the output buffer is put in, so a module can give the \
         transcript keys and colouring of its own -- which is the whole of how `compile' differs \
         from `M-!'. It defaults to `shell-output-mode'.\n\n\
         Returns nil (reporting it) if the command could not be started at all.\n\n\
         Example:\n\
         (shell-command-start \"git status --short\") => \"*Shell Output*\"\n\
         (shell-command-start \"cargo build\" \'compilation-mode)";

primitive!(shell_command_start, args, _env, ctx, {
    let Some(ELispExp::String(command)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let mode = match args.get(1) {
        None => "shell-output-mode".to_string(),
        Some(exp) if exp.is_nil() => "shell-output-mode".to_string(),
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => name.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol naming a mode".into(),
                got: other.clone(),
            });
        }
    };
    let child = match shell_invocation(command).spawn() {
        Ok(child) => child,
        Err(why) => {
            ctx.set_echo_message(&format!("Cannot run the shell: {why}"));
            return Ok(ELispExp::nil());
        }
    };

    let name = free_output_name(ctx);
    ctx.new_buffer(&name, None, Some(mode));
    append(ctx, &name, &format!("$ {command}\n"));
    ctx.with_buffer_mut(&name, |buf| buf.read_only = true);

    // Counted *before* the task is sent, not inside it: the worker may not
    // reach it for a moment, and a frame drawn in that moment would decide
    // nothing was running and go back to sleep until a key was pressed.
    ctx.begin_shell_command();
    if !ctx.send_to_worker(WorkerMessage::RunNow(Box::new(ShellTask {
        child,
        buffer: name.clone(),
    }))) {
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
