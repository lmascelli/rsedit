//! What the command registry speaks: the arguments a command declares
//! ([`ArgSpec`]), the prefix argument, what the editor knew when a command
//! began ([`Invocation`]), and a command whose arguments are still being
//! collected ([`PendingCommand`]). A command is an editor idea: to the
//! interpreter it is an ordinary function.
//!
//! Design: `doc/architettura-editor.typ, <comandi>`.
use std::collections::HashMap;

/// The argument `C-u` and the digit keys build for the next command.
///
/// `Raw` and `Number` differ only in what `P` hands to Lisp: a command often
/// wants how many, and sometimes whether and how the user asked.
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
    /// Returns what `p` gives a command: a plain count.
    pub fn count(&self) -> i32 {
        match self {
            PrefixArg::Raw(times) => 4i32.saturating_pow(*times),
            PrefixArg::Number(n) => *n,
            PrefixArg::Negative => -1,
        }
    }

    /// Extends the argument with a digit key. A digit replaces `Raw` --
    /// `C-u 12` is twelve, not four then twelve -- and after a minus the
    /// digits build a negative number: `C-u - 4 2` is minus forty-two.
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

    /// Returns how the argument reads in the echo area while it is typed.
    pub fn describe(&self) -> String {
        match self {
            PrefixArg::Raw(times) => "C-u ".repeat(*times as usize).trim_end().to_string(),
            PrefixArg::Number(n) => format!("C-u {n}"),
            PrefixArg::Negative => "C-u -".to_string(),
        }
    }
}

/// What the editor knew when a command was invoked: the prefix argument and
/// the region, fixed when the command begins, which is when the user meant
/// them. A prompted argument arrives keystrokes later, from another buffer,
/// after the prefix argument has been cleared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Invocation {
    pub prefix_arg: Option<PrefixArg>,
    /// The active region as it stood, ordered and clamped.
    pub region: Option<(usize, usize)>,
}

/// One argument the editor collects for a command before invoking it.
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
    /// Parses one Emacs-style spec: a one-character code, then the prompt.
    /// The codes are Emacs's own: `s`tring, `n`umber, `b`uffer, `f`ile, and
    /// the three the editor answers itself, `p`, `P` and `r`.
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

    /// Returns the spec written out, as `parse` reads it again:
    /// `ArgSpec::parse(&spec.code())` is `spec`.
    pub fn code(&self) -> String {
        match self {
            ArgSpec::String { prompt } => format!("s{prompt}"),
            ArgSpec::Number { prompt } => format!("n{prompt}"),
            ArgSpec::Buffer { prompt } => format!("b{prompt}"),
            ArgSpec::File { prompt } => format!("f{prompt}"),
            ArgSpec::Count => "p".to_string(),
            ArgSpec::RawCount => "P".to_string(),
            ArgSpec::Region => "r".to_string(),
        }
    }

    /// Returns the symbol Lisp sees for this kind, so that the prompting code
    /// dispatches on it without parsing the code again.
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

    /// Returns how many values this spec hands the command: one, but two for
    /// `r`, the region's ends.
    pub fn arity(&self) -> usize {
        match self {
            ArgSpec::Region => 2,
            _ => 1,
        }
    }

    /// Returns whether this argument has to be asked of the user. The others
    /// are answered from [`Invocation`] at once, so a command whose arguments
    /// are all of that kind -- `["p"]`, most of them -- runs in one step.
    pub fn prompts(&self) -> bool {
        !matches!(self, ArgSpec::Count | ArgSpec::RawCount | ArgSpec::Region)
    }

    /// Returns the prompt, empty for an argument that is not asked.
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
/// `minibuffer-read` opens a prompt and returns, and its answer arrives with a
/// later key: collecting arguments is a chain, and this is the state between
/// its links. The editor keeps them in a stack, since a command can be started
/// while another is still prompting.
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
    /// Creates a pending NAME, with REMAINING arguments to collect.
    pub fn new(name: String, remaining: Vec<ArgSpec>, invocation: Invocation) -> Self {
        Self {
            name,
            remaining,
            collected: Vec::new(),
            invocation,
        }
    }

    /// Returns the argument being prompted for.
    pub fn current(&self) -> Option<&ArgSpec> {
        self.remaining.first()
    }
}

/// Command name -> the arguments to collect for it. A `HashMap`, since
/// `call-interactively` looks a name up at every invocation.
pub type CommandRegistry = HashMap<String, Vec<ArgSpec>>;
