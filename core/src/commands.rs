//! The command registry: which named functions the user may invoke by name,
//! and what arguments the editor should collect for them.
//!
//! # Why this lives in the editor and not the interpreter
//!
//! "Command" is an editor concept, not a Lisp one. A command is something a
//! user can reach with M-x or bind to a key, and whose arguments the *editor*
//! knows how to ask for -- a file name with completion, a buffer name, a
//! number. None of that means anything to an interpreter that has no idea it
//! is embedded in an editor.
//!
//! So the interpreter is untouched by any of this. A command is an ordinary
//! function that happens to have an entry in this registry, which means
//! `(find-file "/tmp/x")` from Lisp is a plain call with no ceremony, and
//! nothing on the `Lambda` type or in `eval` has to know commands exist.
//!
//! # Why the specs are parsed once
//!
//! Registration takes Emacs-style code strings (`"fFind file: "`) and parses
//! them here, at definition time. A typo like `"zFind file: "` is then an error
//! when the editor boots rather than a surprise the first time somebody runs
//! the command.
use std::collections::HashMap;

/// One argument the editor collects on the user's behalf before invoking a
/// command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgSpec {
    /// Free text.
    String { prompt: String },
    /// Free text, parsed as a number before the command sees it.
    Number { prompt: String },
    /// A buffer name, with completion over the live buffers.
    Buffer { prompt: String },
    /// A file name, with completion over the filesystem.
    File { prompt: String },
}

impl ArgSpec {
    /// Parse one Emacs-style spec: a single-character code, then the prompt.
    ///
    /// The codes deliberately match Emacs' own so that muscle memory and
    /// documentation carry over: `s`tring, `n`umber, `b`uffer, `f`ile.
    pub fn parse(code: &str) -> Result<Self, String> {
        let mut chars = code.chars();
        let kind = chars
            .next()
            .ok_or_else(|| "an argument spec cannot be empty".to_string())?;
        let prompt = chars.as_str().to_string();
        match kind {
            's' => Ok(ArgSpec::String { prompt }),
            'n' => Ok(ArgSpec::Number { prompt }),
            'b' => Ok(ArgSpec::Buffer { prompt }),
            'f' => Ok(ArgSpec::File { prompt }),
            other => Err(format!(
                "unknown argument spec code '{other}' in {code:?} -- expected one of \
                 s (string), n (number), b (buffer name), f (file name)"
            )),
        }
    }

    /// The symbol Lisp sees for this kind, so the prompting code can dispatch
    /// on it without re-parsing the original code string.
    pub fn kind(&self) -> &'static str {
        match self {
            ArgSpec::String { .. } => "string",
            ArgSpec::Number { .. } => "number",
            ArgSpec::Buffer { .. } => "buffer",
            ArgSpec::File { .. } => "file",
        }
    }

    pub fn prompt(&self) -> &str {
        match self {
            ArgSpec::String { prompt }
            | ArgSpec::Number { prompt }
            | ArgSpec::Buffer { prompt }
            | ArgSpec::File { prompt } => prompt,
        }
    }
}

/// A command whose arguments are still being collected.
///
/// # Why this exists at all
///
/// `minibuffer-read` does not block: it opens a prompt and returns, and its
/// callback runs on a *later* keystroke. Collecting two arguments therefore
/// cannot be a loop -- it is a chain, and the state between links has to live
/// somewhere. This is that somewhere.
///
/// A stack rather than a single slot, because a command can be started while
/// another one is still prompting (M-x from inside a prompt), and the two must
/// unwind in the order they were begun.
#[derive(Debug)]
pub struct PendingCommand<E> {
    /// The command to apply once every argument is in.
    pub name: String,
    /// Arguments still to read, in order.
    pub remaining: Vec<ArgSpec>,
    /// Values read so far, in the order the command expects them.
    pub collected: Vec<E>,
}

impl<E> PendingCommand<E> {
    pub fn new(name: String, remaining: Vec<ArgSpec>) -> Self {
        Self {
            name,
            remaining,
            collected: Vec::new(),
        }
    }

    /// The argument currently being prompted for.
    pub fn current(&self) -> Option<&ArgSpec> {
        self.remaining.first()
    }
}

/// Name -> the arguments to collect for it.
///
/// A `HashMap` rather than a list: `call-interactively` looks a name up on
/// every invocation, while M-x enumerates only when the user opens the prompt,
/// so the lookup is the operation worth making cheap.
pub type CommandRegistry = HashMap<String, Vec<ArgSpec>>;
