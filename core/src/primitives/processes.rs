//! Talking with a process: starting it with its input and output, writing to
//! it, closing its input, signalling it, asking how it is; threads of its own
//! to read its output, its errors unless they come with the output, and to
//! write its input; its output cut into pieces, lines or `Content-Length`
//! messages before it is delivered.
//!
//! Design: `doc/architettura-editor.typ, <processi-dialoganti>`.
use super::child_process::{
    BATCH, Process, Signal, abandon, cannot_run, in_own_session, incomplete_tail, read_some,
    reap, signal, stop_process,
};
use super::*;
use crate::editor::processes::{Delivery, Exchange, Form, INPUT_QUEUE, Status, status_value};
use crate::work::Stop;
use std::io::{Read, Write};
use std::process::{ChildStderr, ChildStdin, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// The longest line delivered whole: a longer one comes in pieces this long,
/// rather than be held for ever by a process that never ends it.
pub(crate) const MAX_LINE: usize = 1024 * 1024;

/// The longest message taken: a longer one is a fault of the process.
pub(crate) const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// The longest header of a message, its blank line included.
pub(crate) const MAX_HEADER: usize = 8 * 1024;

/// Returns BYTES as text, with what is not UTF-8 replaced.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Cuts what is read into lines, each without its `\n` or `\r\n`.
#[derive(Default)]
pub(crate) struct Lines {
    pending: Vec<u8>,
}

impl Lines {
    /// Takes BYTES, and returns the lines they end.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut lines = Vec::new();
        let mut start = 0;
        while let Some(end) = self.pending[start..].iter().position(|&b| b == b'\n') {
            let line = &self.pending[start..start + end];
            lines.push(text(line.strip_suffix(b"\r").unwrap_or(line)));
            start += end + 1;
        }
        self.pending.drain(..start);
        while self.pending.len() >= MAX_LINE {
            let mut cut = MAX_LINE - incomplete_tail(&self.pending[..MAX_LINE]);
            if cut == 0 {
                cut = MAX_LINE;
            }
            lines.push(text(&self.pending[..cut]));
            self.pending.drain(..cut);
        }
        lines
    }

    /// Returns the last line, when the output ended without ending it.
    pub(crate) fn finish(&mut self) -> Option<String> {
        (!self.pending.is_empty()).then(|| text(&std::mem::take(&mut self.pending)))
    }
}

/// Cuts what is read into the bodies of messages, each after a header that
/// gives its length in bytes, `Content-Length: N`, and a blank line. Other
/// fields of the header are let by; lines may end in `\r\n` or `\n`.
#[derive(Default)]
pub(crate) struct Frames {
    pending: Vec<u8>,
    /// The length of the body being read, once its header has been.
    body: Option<usize>,
}

impl Frames {
    /// Takes BYTES, and returns the bodies they complete; or what is wrong
    /// with a header, after which nothing more can be read.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.pending.extend_from_slice(bytes);
        let mut bodies = Vec::new();
        let mut at = 0;
        loop {
            match self.body {
                Some(length) => {
                    if self.pending.len() - at < length {
                        break;
                    }
                    bodies.push(text(&self.pending[at..at + length]));
                    at += length;
                    self.body = None;
                }
                None => match header(&self.pending[at..])? {
                    Some((length, used)) => {
                        self.body = Some(length);
                        at += used;
                    }
                    None => break,
                },
            }
        }
        self.pending.drain(..at);
        Ok(bodies)
    }

    /// Returns whether nothing is left half read.
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.body.is_none()
    }
}

/// Reads the header at the start of BYTES: the body's length and how many
/// bytes the header took, `None` when it has not all arrived.
fn header(bytes: &[u8]) -> Result<Option<(usize, usize)>, String> {
    let mut length = None;
    let mut at = 0;
    loop {
        let Some(end) = bytes[at..].iter().position(|&b| b == b'\n') else {
            return if bytes.len() > MAX_HEADER {
                Err(format!("a header longer than {MAX_HEADER} bytes"))
            } else {
                Ok(None)
            };
        };
        let raw = &bytes[at..at + end];
        let line = text(raw.strip_suffix(b"\r").unwrap_or(raw));
        at += end + 1;
        if at > MAX_HEADER {
            return Err(format!("a header longer than {MAX_HEADER} bytes"));
        }
        if line.is_empty() {
            let length = length.ok_or("a header without Content-Length")?;
            if length > MAX_MESSAGE {
                return Err(format!(
                    "a message of {length} bytes, more than the {MAX_MESSAGE} taken"
                ));
            }
            return Ok(Some((length, at)));
        }
        let Some((field, value)) = line.split_once(':') else {
            return Err(format!("a header line that is not a field: {line:?}"));
        };
        if field.trim().eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| format!("a Content-Length that is not a length: {value:?}"))?,
            );
        }
    }
}

/// Returns TEXT with the header that frames it as a message.
pub(crate) fn framed(text: &str) -> Vec<u8> {
    let mut bytes = format!("Content-Length: {}\r\n\r\n", text.len()).into_bytes();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// The pipes of a process just started.
struct Pipes {
    input: ChildStdin,
    /// Its standard output, and its standard error too when they were
    /// merged.
    output: Box<dyn Read + Send>,
    /// Its standard error, when it has a pipe of its own.
    errors: Option<ChildStderr>,
}

/// Variables given to one process: a value, or `None` to take the variable
/// away.
type Variables = Vec<(String, Option<String>)>;

/// How a process is started, past its program and its arguments.
struct Options {
    /// Its standard error goes into the pipe of its output, in the order it
    /// was written, rather than a pipe of its own.
    merged: bool,
    /// Variables given to it over the editor's environment.
    environment: Variables,
}

/// Starts PROGRAM with ARGUMENTS, as they are and with no shell between, in
/// DIRECTORY, with the environment of CTX and that of OPTIONS over it, and
/// pipes for its input, output and errors -- one for both of the two last
/// when OPTIONS merges them -- in a session and a group of its own.
fn start<B: BufferTrait>(
    ctx: &EditorState<B>,
    program: &str,
    arguments: &[String],
    directory: &str,
    options: &Options,
) -> std::io::Result<(std::process::Child, Pipes)> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::piped());
    let merged = if options.merged {
        let (reader, writer) = std::io::pipe()?;
        command.stdout(writer.try_clone()?).stderr(writer);
        Some(reader)
    } else {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        None
    };
    ctx.apply_environment(&mut command);
    for (name, value) in &options.environment {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    in_own_session(&mut command);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // A console program started from the graphical editor would open a
        // console window of its own: nobody types at it.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    // The writing ends of a merged pipe that the command still holds go with
    // it: until they have, the pipe never ends, however long ago the process
    // did.
    drop(command);
    let output: Option<Box<dyn Read + Send>> = match merged {
        Some(reader) => Some(Box::new(reader)),
        None => child
            .stdout
            .take()
            .map(|output| Box::new(output) as Box<dyn Read + Send>),
    };
    let errors = child.stderr.take();
    let pipes = match (child.stdin.take(), output) {
        (Some(input), Some(output)) if options.merged || errors.is_some() => Pipes {
            input,
            output,
            errors,
        },
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("its pipes were not given"));
        }
    };
    Ok((child, pipes))
}

/// Queues DELIVERY and tells the host; returns whether it was queued.
fn hand<B: BufferTrait>(
    state: &EditorState<B>,
    exchange: &Exchange,
    stop: &Stop,
    delivery: Delivery,
) -> bool {
    let queued = exchange.push(delivery, stop);
    if queued {
        state.changed_in_background();
    }
    queued
}

/// Reads the output into deliveries cut by FORM until it ends, is stopped or
/// cannot be read.
fn read_output<B: BufferTrait>(
    state: &EditorState<B>,
    exchange: &Exchange,
    stop: &Stop,
    mut output: impl Read,
    form: Form,
) {
    let mut batch = vec![0; BATCH];
    let mut lines = Lines::default();
    let mut frames = Frames::default();
    // In pieces: the start of a character whose end has not arrived.
    let mut pending = Vec::new();
    let fault = |why: String| {
        hand(state, exchange, stop, Delivery::Error(why));
        exchange.mark_stopped();
        stop_process(exchange.process());
    };
    while !stop.is_cancelled() {
        let read = match read_some(&mut output, &mut batch) {
            Ok(0) => break,
            Ok(read) => &batch[..read],
            Err(why) => return fault(format!("the output could not be read: {why}")),
        };
        let pieces = match form {
            Form::Chunks => {
                pending.extend_from_slice(read);
                let cut = pending.len() - incomplete_tail(&pending);
                let piece = text(&pending[..cut]);
                pending.drain(..cut);
                if piece.is_empty() {
                    vec![]
                } else {
                    vec![piece]
                }
            }
            Form::Lines => lines.push(read),
            Form::Messages => match frames.push(read) {
                Ok(bodies) => bodies,
                Err(why) => return fault(format!("the output is not a message: {why}")),
            },
        };
        for piece in pieces {
            if !hand(state, exchange, stop, Delivery::Output(piece)) {
                return;
            }
        }
    }
    if stop.is_cancelled() {
        return;
    }
    let last = match form {
        Form::Chunks => (!pending.is_empty()).then(|| text(&pending)),
        Form::Lines => lines.finish(),
        Form::Messages => {
            if !frames.is_empty() {
                hand(
                    state,
                    exchange,
                    stop,
                    Delivery::Error("the output ended inside a message".into()),
                );
            }
            None
        }
    };
    if let Some(last) = last {
        hand(state, exchange, stop, Delivery::Output(last));
    }
}

/// Reads the errors a line at a time until they end or the process is
/// stopped.
fn read_errors<B: BufferTrait>(
    state: &EditorState<B>,
    exchange: &Exchange,
    stop: &Stop,
    mut errors: ChildStderr,
) {
    let mut batch = vec![0; BATCH];
    let mut lines = Lines::default();
    while !stop.is_cancelled() {
        let read = match read_some(&mut errors, &mut batch) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for line in lines.push(&batch[..read]) {
            if !hand(state, exchange, stop, Delivery::Error(line)) {
                return;
            }
        }
    }
    if let Some(line) = lines.finish() {
        hand(state, exchange, stop, Delivery::Error(line));
    }
}

/// Writes what is queued for the input, in order, until the input is closed
/// or the process stops reading it.
fn write_input(mut input: ChildStdin, queued: Receiver<Vec<u8>>) {
    for bytes in queued {
        if input
            .write_all(&bytes)
            .and_then(|()| input.flush())
            .is_err()
        {
            break;
        }
    }
}

/// The life of a process, on the thread that has its name: reads its output
/// here, and its errors -- unless they come with the output -- and its input
/// on threads of their own, scoped to this one; then waits for it, and
/// delivers how it ended.
fn converse<B: BufferTrait>(
    state: &EditorState<B>,
    stop: &Stop,
    name: &str,
    exchange: Arc<Exchange>,
    pipes: Pipes,
    queued: Receiver<Vec<u8>>,
    form: Form,
) {
    stop.on_cancel({
        let exchange = exchange.clone();
        move || {
            exchange.mark_stopped();
            stop_process(exchange.process());
        }
    });
    let Pipes {
        input,
        output,
        errors,
    } = pipes;
    let status = std::thread::scope(|scope| {
        let exchange = &*exchange;
        // No thread for the errors when they come with the output.
        let reading = errors.map(|errors| {
            std::thread::Builder::new()
                .name(format!("{name} errors"))
                .spawn_scoped(scope, move || read_errors(state, exchange, stop, errors))
        });
        let writing = std::thread::Builder::new()
            .name(format!("{name} input"))
            .spawn_scoped(scope, move || write_input(input, queued));
        if reading.as_ref().is_some_and(Result::is_err) || writing.is_err() {
            hand(
                state,
                exchange,
                stop,
                Delivery::Error("the system gave no thread for the errors or the input".into()),
            );
            exchange.mark_stopped();
            stop_process(exchange.process());
        }
        read_output(state, exchange, stop, output, form);
        if let Some(Ok(reading)) = reading {
            let _ = reading.join();
        }
        let status = reap(exchange.process());
        // The process is gone: what is still queued for it goes nowhere, and
        // the writing thread, finding the queue closed, ends.
        exchange.close_input();
        if let Ok(writing) = writing {
            let _ = writing.join();
        }
        status
    });
    let status = match status {
        Ok(status) => match status.code() {
            Some(code) => Status::Exited(code),
            // Killed by a signal, which has no exit code to report.
            None => Status::Killed,
        },
        Err(why) => {
            state.log_diagnostic(&format!(
                "[ERROR] process {name} could not be waited for: {why}"
            ));
            Status::Killed
        }
    };
    exchange.push_exit(status);
    state.changed_in_background();
}

/// Returns the strings of the list ARGUMENTS, or the error for one that is not
/// a string.
fn arguments<B: BufferTrait>(
    arguments: Option<&ELispExp<B>>,
) -> Result<Vec<String>, EvalError<EditorState<B>>> {
    let Some(list) = arguments else {
        return Ok(Vec::new());
    };
    args::list(list)
        .into_iter()
        .map(|argument| match argument {
            ELispExp::String(argument) => Ok(argument.to_string()),
            other => Err(EvalError::WrongArgumentType {
                expected: "a list of strings".into(),
                got: other,
            }),
        })
        .collect()
}

/// Returns the form FORM names: `lines` when it is absent or nil.
fn form<B: BufferTrait>(form: Option<&ELispExp<B>>) -> Result<Form, EvalError<EditorState<B>>> {
    match args::optional_name(form, "Symbol naming a form")?.as_deref() {
        None | Some("lines") => Ok(Form::Lines),
        Some("chunks") => Ok(Form::Chunks),
        Some("messages") => Ok(Form::Messages),
        Some(_) => Err(EvalError::WrongArgumentType {
            expected: "chunks, lines or messages".into(),
            got: form.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

pub const PROCESS_START_DOC: &str = "(process-start NAME PROGRAM ARGS FUNCTION &optional FORM \
         &key :errors :environment): Start PROGRAM with ARGS, a list of strings passed as they \
         are with no shell, in `working-directory', and return NAME. Between commands FUNCTION \
         is called with NAME, a kind and a value: output and what it printed, cut as FORM says \
         -- lines, chunks or messages --; error and a line of its standard error; last, exit and \
         how it ended. :errors merged sends the standard error with the output, in order. \
         :environment is a list of \"NAME=VALUE\", or \"NAME\" to unset one. A NAME in use is \
         an error; nil, saying why, when PROGRAM cannot be started.\n\n\
         Example:\n\
         (process-start 'echo \"cat\" nil (lambda (name kind value) (message \"%s\" value)))";

/// The keywords `process-start` takes.
const PROCESS_START_KEYWORDS: &[&str] = &[":errors", ":environment"];

/// Returns whether ERRORS, the value of `:errors`, merges the standard error
/// into the output: `merged`, or `separate`, the default, and nil.
fn merged<B: BufferTrait>(errors: Option<&ELispExp<B>>) -> Result<bool, EvalError<EditorState<B>>> {
    match args::optional_name(errors, "Symbol naming where the errors go")?.as_deref() {
        None | Some("separate") => Ok(false),
        Some("merged") => Ok(true),
        Some(_) => Err(EvalError::WrongArgumentType {
            expected: "merged or separate".into(),
            got: errors.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// Returns the variables of ENVIRONMENT, the value of `:environment`: a list
/// of strings, each `NAME=VALUE` to set NAME, or `NAME` to take it away.
fn environment<B: BufferTrait>(
    environment: Option<&ELispExp<B>>,
) -> Result<Variables, EvalError<EditorState<B>>> {
    let refused = |got: ELispExp<B>| EvalError::WrongArgumentType {
        expected: "a list of strings NAME=VALUE or NAME".into(),
        got,
    };
    arguments(environment)
        .map_err(|why| match why {
            EvalError::WrongArgumentType { got, .. } => refused(got),
            other => other,
        })?
        .into_iter()
        .map(|entry| {
            let (name, value) = match entry.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (entry.as_str(), None),
            };
            if name.is_empty() {
                Err(refused(ELispExp::string(entry.clone())))
            } else {
                Ok((name.to_string(), value))
            }
        })
        .collect()
}

primitive!(process_start, args, _env, ctx, {
    let (args, keywords) =
        args::with_keywords("process-start", args, 4, 5, PROCESS_START_KEYWORDS)?;
    let name = args::name(args.first())?;
    let Some(ELispExp::String(program)) = args.get(1) else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[1].clone(),
        });
    };
    let arguments = arguments(args.get(2))?;
    let function = args[3].clone();
    if !super::workers::is_callable(&function) {
        return Err(EvalError::WrongArgumentType {
            expected: "a function to call".into(),
            got: function,
        });
    }
    let form = form(args.get(4))?;
    let options = Options {
        merged: merged(keywords.get(":errors"))?,
        environment: environment(keywords.get(":environment"))?,
    };
    if !ctx.dialogue_name_free(&name) {
        return Err(EvalError::RuntimeMessage(format!(
            "process-start: {name} is in use"
        )));
    }
    let directory = ctx.default_directory();
    let (child, pipes) = match start(ctx, program, &arguments, &directory, &options) {
        Ok(started) => started,
        Err(why) => {
            ctx.set_echo_message(&cannot_run(program, &directory, &why));
            return Ok(ELispExp::nil());
        }
    };
    let process: Process = Arc::new(Mutex::new(Some(child)));
    ctx.remember_child_process(&process);
    let (input, queued) = std::sync::mpsc::sync_channel(INPUT_QUEUE);
    let exchange = Arc::new(Exchange::new(process.clone(), input, form));
    let dialogue = ctx.add_dialogue(&name, exchange.clone(), function);
    let thread_name = name.clone();
    let worker = ctx.spawn_work(&name, move |state, stop, _| {
        converse(state, stop, &thread_name, exchange, pipes, queued, form)
    });
    match worker {
        Some(id) => {
            let _ = dialogue.worker.set(id);
            Ok(ELispExp::string(name))
        }
        None => {
            ctx.forget_dialogue(&name);
            abandon(&process);
            ctx.set_echo_message(&super::shell::cannot_start_thread(ctx, "the process"));
            Ok(ELispExp::nil())
        }
    }
});

pub const PROCESS_SEND_DOC: &str = "(process-send NAME STRING): Write STRING to the input of the \
         process NAME, framed by a Content-Length header when its output is read as messages, and \
         return t. Return nil, writing nothing, when there is no such process, its input is \
         closed, or so much is already waiting to be written that this would have to wait too.\n\n\
         Example:\n\
         (process-send 'echo \"hello\\n\")";

primitive!(process_send, args, _env, ctx, {
    exact_arity(args, 2)?;
    let name = args::name(args.first())?;
    let Some(ELispExp::String(string)) = args.get(1) else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[1].clone(),
        });
    };
    let Some(dialogue) = ctx.dialogue(&name) else {
        return Ok(ELispExp::nil());
    };
    let bytes = match dialogue.exchange.form() {
        Form::Messages => framed(string),
        Form::Chunks | Form::Lines => string.as_bytes().to_vec(),
    };
    Ok(ELispExp::boolean(dialogue.exchange.send(bytes)))
});

pub const PROCESS_CLOSE_INPUT_DOC: &str = "(process-close-input NAME): Close the input of the \
         process NAME once what is waiting to be written has been, so that it reads the end of \
         its input; return t, or nil when there is no such process or its input is closed \
         already.\n\n\
         Example:\n\
         (process-close-input 'echo)";

primitive!(process_close_input, args, _env, ctx, {
    exact_arity(args, 1)?;
    let name = args::name(args.first())?;
    Ok(ELispExp::boolean(
        ctx.dialogue(&name)
            .is_some_and(|dialogue| dialogue.exchange.close_input()),
    ))
});

pub const PROCESS_STOP_DOC: &str = "(process-stop NAME): Stop the process NAME, as `stop-worker' \
         does, and return t; nil when there is no such process running. Its function is still \
         called with exit, and killed.\n\n\
         Example:\n\
         (process-stop 'echo)";

primitive!(process_stop, args, _env, ctx, {
    exact_arity(args, 1)?;
    let name = args::name(args.first())?;
    let stopped = ctx.dialogue(&name).is_some_and(|dialogue| {
        dialogue.exchange.status() == Status::Running
            && dialogue
                .worker
                .get()
                .is_some_and(|&id| ctx.work().cancel(id))
    });
    Ok(ELispExp::boolean(stopped))
});

pub const PROCESS_SIGNAL_DOC: &str = "(process-signal NAME SIGNAL): Send SIGNAL to the process \
         NAME and to what it started, and return t: interrupt, as C-c at a terminal, quit, \
         hangup, terminate or kill. Return nil when there is no such process running, or when \
         this system cannot send SIGNAL: on Windows only terminate and kill, which kill it.\n\n\
         Example:\n\
         (process-signal 'shell 'interrupt)";

/// Returns the signal SIGNAL names.
fn signal_named<B: BufferTrait>(
    signal: Option<&ELispExp<B>>,
) -> Result<Signal, EvalError<EditorState<B>>> {
    let refused = || EvalError::WrongArgumentType {
        expected: "interrupt, quit, hangup, terminate or kill".into(),
        got: signal.cloned().unwrap_or_else(ELispExp::nil),
    };
    match args::optional_name(signal, "Symbol naming a signal")?.as_deref() {
        Some("interrupt") => Ok(Signal::Interrupt),
        Some("quit") => Ok(Signal::Quit),
        Some("hangup") => Ok(Signal::Hangup),
        Some("terminate") => Ok(Signal::Terminate),
        Some("kill") => Ok(Signal::Kill),
        _ => Err(refused()),
    }
}

primitive!(process_signal, args, _env, ctx, {
    exact_arity(args, 2)?;
    let name = args::name(args.first())?;
    let wanted = signal_named(args.get(1))?;
    let sent = ctx.dialogue(&name).is_some_and(|dialogue| {
        dialogue.exchange.status() == Status::Running && signal(dialogue.exchange.process(), wanted)
    });
    Ok(ELispExp::boolean(sent))
});

pub const PROCESS_STATUS_DOC: &str = "(process-status NAME): Return how the process NAME is: run, \
         (exit CODE) once it has ended by itself, killed once it was stopped or killed by a \
         signal, nil when there is no such process.\n\n\
         Example:\n\
         (eq (process-status 'echo) 'run)";

primitive!(process_status, args, _env, ctx, {
    exact_arity(args, 1)?;
    let name = args::name(args.first())?;
    Ok(ctx.dialogue(&name).map_or_else(ELispExp::nil, |dialogue| {
        status_value(&dialogue.exchange.status())
    }))
});

/// Registers the primitives that talk with a process.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("process-start", process_start, PROCESS_START_DOC);
    into.function("process-send", process_send, PROCESS_SEND_DOC);
    into.function(
        "process-close-input",
        process_close_input,
        PROCESS_CLOSE_INPUT_DOC,
    );
    into.function("process-stop", process_stop, PROCESS_STOP_DOC);
    into.function("process-signal", process_signal, PROCESS_SIGNAL_DOC);
    into.function("process-status", process_status, PROCESS_STATUS_DOC);
}
