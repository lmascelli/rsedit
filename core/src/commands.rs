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

/// The argument `C-u` and the digit keys build for the next command.
///
/// `Raw` and `Number` differ only in what `P` hands to Lisp, and that is the
/// entire reason Emacs has both `p` and `P`: a command often wants "how many",
/// but sometimes wants "did the user ask for anything at all, and how".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixArg {
    /// `C-u` repeated N times: four to the N.
    Raw(u32),
    /// Digits typed after `C-u`.
    Number(i32),
    /// `C-u -` with no digits yet.
    Negative,
}

impl PrefixArg {
    /// What `p` gives a command: a plain count.
    pub fn count(&self) -> i32 {
        match self {
            PrefixArg::Raw(times) => 4i32.saturating_pow(*times),
            PrefixArg::Number(n) => *n,
            PrefixArg::Negative => -1,
        }
    }

    /// Extend the argument with a digit key.
    ///
    /// A digit replaces `Raw`: `C-u 12` means twelve, not four then twelve.
    /// After a minus the digits build a negative number, so `C-u - 4 2` is
    /// minus forty-two rather than minus one followed by a number.
    pub fn push_digit(&mut self, digit: u32) {
        let digit = digit as i32;
        *self = match *self {
            PrefixArg::Raw(_) => PrefixArg::Number(digit),
            PrefixArg::Negative => PrefixArg::Number(-digit),
            PrefixArg::Number(n) if n < 0 => {
                PrefixArg::Number(n.saturating_mul(10).saturating_sub(digit))
            }
            PrefixArg::Number(n) => PrefixArg::Number(n.saturating_mul(10).saturating_add(digit)),
        };
    }

    /// How the argument reads in the echo area while it is being typed.
    pub fn describe(&self) -> String {
        match self {
            PrefixArg::Raw(times) => "C-u ".repeat(*times as usize).trim_end().to_string(),
            PrefixArg::Number(n) => format!("C-u {n}"),
            PrefixArg::Negative => "C-u -".to_string(),
        }
    }
}

/// What the editor knew at the moment a command was invoked.
///
/// # Why this is captured rather than read when needed
///
/// A command's arguments are not all collected at once: a prompted one arrives
/// keystrokes later, through a minibuffer that had to *switch buffers* to open.
/// By the time the last argument is in, the region belongs to a different
/// buffer and the prefix argument has been cleared for the next command.
///
/// So the answers the editor gives on the user's behalf are fixed when the
/// command starts, which is also when the user meant them. `C-u 4 M-x foo`
/// passes four to `foo` however long the prompt takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Invocation {
    pub prefix_arg: Option<PrefixArg>,
    /// The active region as it stood, ordered and clamped.
    pub region: Option<(usize, usize)>,
}

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
    /// The prefix argument as a count, defaulting to 1. Never prompts.
    Count,
    /// The raw prefix argument, nil when there was none. Never prompts.
    RawCount,
    /// The active region, as *two* arguments -- its start and its end. Never
    /// prompts, and refuses the command outright when there is no region.
    Region,
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
            // The three that answer themselves. They take no prompt, so
            // anything written after the code is a mistake worth reporting
            // rather than silently dropping.
            'p' | 'P' | 'r' if !prompt.is_empty() => Err(format!(
                "argument spec {code:?} takes no prompt: '{kind}' is answered by the editor"
            )),
            'p' => Ok(ArgSpec::Count),
            'P' => Ok(ArgSpec::RawCount),
            'r' => Ok(ArgSpec::Region),
            other => Err(format!(
                "unknown argument spec code '{other}' in {code:?} -- expected one of \
                 s (string), n (number), b (buffer name), f (file name), \
                 p (prefix count), P (raw prefix), r (region)"
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
            ArgSpec::Count => "count",
            ArgSpec::RawCount => "raw-count",
            ArgSpec::Region => "region",
        }
    }

    /// How many values this spec hands the command.
    ///
    /// Almost always one. `r` is the exception -- it is the region's two ends
    /// -- which is why the arity check counts values rather than specs: a
    /// command declared `["r"]` really does take two parameters.
    pub fn arity(&self) -> usize {
        match self {
            ArgSpec::Region => 2,
            _ => 1,
        }
    }

    /// Whether this argument has to be asked of the user.
    ///
    /// The ones that do not are answered from [`Invocation`], immediately and
    /// without a minibuffer. A command whose arguments are all of that kind --
    /// `["p"]`, which is most of them -- runs in one step.
    pub fn prompts(&self) -> bool {
        !matches!(self, ArgSpec::Count | ArgSpec::RawCount | ArgSpec::Region)
    }

    pub fn prompt(&self) -> &str {
        match self {
            ArgSpec::String { prompt }
            | ArgSpec::Number { prompt }
            | ArgSpec::Buffer { prompt }
            | ArgSpec::File { prompt } => prompt,
            ArgSpec::Count | ArgSpec::RawCount | ArgSpec::Region => "",
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
    /// What the editor knew when this command started, for the arguments it
    /// answers itself.
    pub invocation: Invocation,
}

impl<E> PendingCommand<E> {
    pub fn new(name: String, remaining: Vec<ArgSpec>, invocation: Invocation) -> Self {
        Self {
            name,
            remaining,
            collected: Vec::new(),
            invocation,
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
