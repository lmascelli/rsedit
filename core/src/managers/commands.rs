//! What can be invoked by name, what is part-way through being invoked, and
//! what was invoked last: the life of one command, written in order on the way
//! through a keystroke. Nothing here evaluates, and every accessor copies out,
//! so that the Lisp a command runs can register commands of its own.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use crate::{
    ELispExp,
    buffer::BufferTrait,
    commands::{ArgSpec, CommandRegistry, Invocation, PendingCommand, PrefixArg},
    input::{KeyCode, KeyEvent},
};
use std::sync::Arc;

/// The command registry, the commands still collecting arguments, the prefix
/// argument, and what ran last.
pub struct Commands<B: BufferTrait> {
    /// Which named functions the user may invoke by name, and what arguments
    /// the editor collects for each.
    registry: CommandRegistry,
    /// Commands whose arguments are still being collected, innermost last.
    pending: Vec<PendingCommand<ELispExp<B>>>,
    /// The argument being built for the next command.
    prefix_arg: Option<PrefixArg>,
    /// Whether the digit keys are still being read into it. Separate from the
    /// argument because "no argument" and "an argument that is finished" are
    /// different states, and a digit means something different in each.
    reading_prefix: bool,
    /// Name of the command that ran immediately before the current one.
    last_name: Option<Arc<String>>,
    /// The last command as a form that can be evaluated again, which is what
    /// `repeat` re-runs.
    last_form: Option<ELispExp<B>>,
}

impl<B: BufferTrait> Default for Commands<B> {
    fn default() -> Self {
        Self {
            registry: CommandRegistry::new(),
            pending: Vec::new(),
            prefix_arg: None,
            reading_prefix: false,
            last_name: None,
            last_form: None,
        }
    }
}

impl<B: BufferTrait> Commands<B> {
    // ------------------------------------------------------------------
    // The registry
    // ------------------------------------------------------------------

    /// Registers NAME as a command taking SPECS, replacing what it took
    /// before: how a command prompts can change without a restart.
    pub fn register(&mut self, name: &str, specs: Vec<ArgSpec>) {
        self.registry.insert(name.to_string(), specs);
    }

    /// Returns the arguments to collect for NAME, copied out, or `None` if it
    /// is not a command.
    pub fn specs(&self, name: &str) -> Option<Vec<ArgSpec>> {
        self.registry.get(name).cloned()
    }

    /// Returns whether NAME is a command.
    pub fn is_command(&self, name: &str) -> bool {
        self.registry.contains_key(name)
    }

    /// Returns every command's name, sorted, for `M-x` completion.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.registry.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    // ------------------------------------------------------------------
    // The prefix argument
    // ------------------------------------------------------------------

    /// Returns the argument waiting for the next command, if any.
    pub fn prefix_argument(&self) -> Option<PrefixArg> {
        self.prefix_arg
    }

    /// Forgets the argument. Called after every command, whether or not
    /// anything used it: an argument belongs to one command, and one that
    /// fails must not leave it to the next.
    pub fn clear_prefix_argument(&mut self) {
        self.prefix_arg = None;
        self.reading_prefix = false;
    }

    /// Gives the next command ARG, as if it had been typed. `M-x` needs it:
    /// the argument typed before `M-x` belongs to the command it reads.
    pub fn set_prefix_argument(&mut self, arg: Option<PrefixArg>) {
        self.prefix_arg = arg;
        self.reading_prefix = false;
    }

    /// Returns whether EVENT is the key that starts a prefix argument. The
    /// key handling asks too, before the reader runs: a key capture reports
    /// `C-u` as an argument, and a buffer's own keymap may claim the key.
    pub fn is_prefix_argument_key(event: &KeyEvent) -> bool {
        event.modifiers.ctrl && !event.modifiers.alt && event.code == KeyCode::Char('u')
    }

    /// Offers EVENT to the prefix-argument reader, and returns whether it was
    /// taken.
    ///
    /// Called before the keymaps see the key: `C-u` and the digits after it
    /// are not keys for a keymap. Anything else falls through, a digit typed
    /// when no argument is being built included.
    pub fn read_prefix_argument(&mut self, event: &KeyEvent) -> bool {
        // C-u: start an argument, or multiply the one being built by four.
        if Self::is_prefix_argument_key(event) {
            self.prefix_arg = Some(match (self.prefix_arg, self.reading_prefix) {
                (Some(PrefixArg::Raw(times)), true) => PrefixArg::Raw(times + 1),
                _ => PrefixArg::Raw(1),
            });
            self.reading_prefix = true;
            return true;
        }

        if self.reading_prefix
            && let KeyCode::Char(c) = event.code
            && !event.modifiers.ctrl
            && !event.modifiers.alt
        {
            if let Some(digit) = c.to_digit(10) {
                self.prefix_arg
                    .get_or_insert(PrefixArg::Raw(1))
                    .push_digit(digit);
                return true;
            }
            // A minus is only a sign, and only before any digits.
            if c == '-' && matches!(self.prefix_arg, Some(PrefixArg::Raw(_))) {
                self.prefix_arg = Some(PrefixArg::Negative);
                return true;
            }
        }

        // Not part of the argument, which is finished, though not yet used.
        self.reading_prefix = false;
        false
    }

    // ------------------------------------------------------------------
    // Arguments still being collected
    // ------------------------------------------------------------------

    /// Begins collecting arguments for NAME.
    pub fn push_pending(&mut self, name: String, remaining: Vec<ArgSpec>, invocation: Invocation) {
        self.pending
            .push(PendingCommand::new(name, remaining, invocation));
    }

    /// Answers, in order, every argument the editor can answer itself, and
    /// returns the first one that still needs the user; `None` when the
    /// command has everything it needs.
    ///
    /// One loop for both kinds, since a spec list like
    /// `["p", "sReplace with: "]` mixes them.
    pub fn fill_answerable_args(&mut self) -> Option<ArgSpec> {
        let pending = self.pending.last_mut()?;
        while let Some(spec) = pending.remaining.first() {
            if spec.prompts() {
                return Some(spec.clone());
            }
            let values = answer_spec::<B>(spec, &pending.invocation);
            pending.remaining.remove(0);
            pending.collected.extend(values);
        }
        None
    }

    /// Returns how far through its arguments the innermost pending command
    /// is: its name, the position of the argument being read counting from 1,
    /// and how many there are. The prompt's title says it.
    pub fn pending_progress(&self) -> Option<(String, usize, usize)> {
        let pending = self.pending.last()?;
        let done = pending.collected.len();
        Some((
            pending.name.clone(),
            done + 1,
            done + pending.remaining.len(),
        ))
    }

    /// Returns the argument the innermost pending command is waiting on.
    pub fn pending_current_spec(&self) -> Option<ArgSpec> {
        self.pending
            .last()
            .and_then(|pending| pending.current().cloned())
    }

    /// Records VALUE as the innermost pending command's next argument. What
    /// comes next is [`Commands::fill_answerable_args`]'s to say.
    pub fn accept_pending_arg(&mut self, value: ELispExp<B>) {
        let Some(pending) = self.pending.last_mut() else {
            return;
        };
        if !pending.remaining.is_empty() {
            pending.remaining.remove(0);
        }
        pending.collected.push(value);
    }

    /// Removes and returns the innermost pending command.
    pub fn take_pending(&mut self) -> Option<(String, Vec<ELispExp<B>>)> {
        self.pending
            .pop()
            .map(|pending| (pending.name, pending.collected))
    }

    /// Drops every pending command.
    ///
    /// Called when a command starts with no minibuffer open: what is left
    /// belongs to a prompt closed some other way than confirm or cancel, and
    /// would otherwise be fed the next command's input.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    // ------------------------------------------------------------------
    // What ran last
    // ------------------------------------------------------------------

    /// Returns whether the previous command was NAME: the question
    /// `last-command` is asked, answered without copying the name.
    pub fn last_was(&self, name: &str) -> bool {
        self.last_name.as_deref().map(String::as_str) == Some(name)
    }

    /// Records NAME as the command that just ran.
    pub fn set_last_name(&mut self, name: Option<Arc<String>>) {
        self.last_name = name;
    }

    /// Returns the last command as a runnable form, for `repeat`.
    pub fn last_form(&self) -> Option<ELispExp<B>> {
        self.last_form.clone()
    }

    /// Records FORM as what `repeat` runs.
    pub fn set_last_form(&mut self, form: Option<ELispExp<B>>) {
        self.last_form = form;
    }
}

/// Returns the prefix argument as `P` gives it: nil, a one-element list for
/// `C-u`, a number, or the symbol `-`.
pub(crate) fn raw_prefix<B: BufferTrait>(arg: Option<PrefixArg>) -> ELispExp<B> {
    match arg {
        None => ELispExp::nil(),
        Some(PrefixArg::Raw(times)) => {
            ELispExp::proper_list(vec![ELispExp::number(4i32.saturating_pow(times) as f64)])
        }
        Some(PrefixArg::Number(n)) => ELispExp::number(n as f64),
        Some(PrefixArg::Negative) => ELispExp::symbol("-".into()),
    }
}

/// Returns what [`raw_prefix`] gives, read back; `None` for a value it never
/// gives.
pub(crate) fn prefix_from_raw<B: BufferTrait>(value: &ELispExp<B>) -> Option<Option<PrefixArg>> {
    let whole = |n: f64| (n.fract() == 0.0 && n.abs() <= i32::MAX as f64).then_some(n as i32);
    match value {
        _ if value.is_nil() => Some(None),
        ELispExp::Number(n) => whole(*n).map(|n| Some(PrefixArg::Number(n))),
        ELispExp::Symbol(name) if name.as_str() == "-" => Some(Some(PrefixArg::Negative)),
        ELispExp::Cons(_) => match value.split_list() {
            (items, tail) if items.len() == 1 && tail.is_nil() => match items[0] {
                ELispExp::Number(n) => {
                    let n = whole(n)?;
                    (1..16u32)
                        .find(|times| 4i32.checked_pow(*times) == Some(n))
                        .map(|times| Some(PrefixArg::Raw(times)))
                }
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Returns what the editor hands a command for an argument it answers
/// itself: one value, or two for `r`, the region's ends, as `interactive "r"`
/// gives in Emacs. Everything it needs was captured when the command began.
fn answer_spec<B: BufferTrait>(spec: &ArgSpec, invocation: &Invocation) -> Vec<ELispExp<B>> {
    match spec {
        // `p`: a plain count, and one when the user asked for nothing. This is
        // what makes `(forward-char)` and `C-u 4 C-f` the same code path.
        ArgSpec::Count => vec![ELispExp::number(
            invocation.prefix_arg.map(|arg| arg.count()).unwrap_or(1) as f64,
        )],
        // `P`: the argument as given, so a command can tell "no argument" from
        // "the argument 1", and a bare `C-u` from `C-u 4`. A bare `C-u` is a
        // one-element list, as in Emacs, which is why `p` and `P` both exist.
        ArgSpec::RawCount => vec![raw_prefix(invocation.prefix_arg)],
        // `r`: start then end. `call-interactively` refuses the command before
        // this is reached when there is no region, so the fallback is
        // unreachable rather than a silent default.
        ArgSpec::Region => {
            let (start, end) = invocation.region.unwrap_or((0, 0));
            vec![ELispExp::number(start as f64), ELispExp::number(end as f64)]
        }
        prompted => {
            debug_assert!(prompted.prompts(), "an unprompted spec with no answer");
            Vec::new()
        }
    }
}
