//! What can be invoked by name, what is part-way through being invoked, and
//! what was invoked last.
//!
//! # Why this is a compartment and not five fields
//!
//! They are the life of one command, in order: the registry says what `M-x`
//! may run and what arguments it needs, the prefix argument is the count typed
//! in front of it, the pending stack is what is still being collected, and the
//! two `last_*` fields are what is left behind for the next command to ask
//! about. Every one of them is written on the way through a single keystroke,
//! and the order they are written in *is* the protocol.
//!
//! The prefix argument was itself a pair in one lock -- `(Option<PrefixArg>,
//! bool)` -- with nothing but the tuple position to say which was which. Here
//! they are two named fields, and `reading` says what it means.
//!
//! # What is deliberately not here
//!
//! Lisp. The registry holds argument *specifications* and the pending stack
//! holds collected values, but nothing here evaluates one. That matters more
//! than usual: `call-interactively` looks a command up and then evaluates,
//! and that Lisp may call `register-command` -- so every accessor copies out
//! and gives the lock back rather than handing out a guard. Holding this
//! across `eval` is the reentrancy that deadlocked `run_hook` once already.
//!
//! Hooks. They read as command machinery and are not: a hook belongs to a
//! *mode*, and the ones registered for every mode are the global half of
//! `MajorMode::hooks` exactly as the global keymap is the global half of
//! `MajorMode::keymaps`. They live with the modes.
use crate::{
    ELispExp,
    buffer::BufferTrait,
    commands::{ArgSpec, CommandRegistry, Invocation, PendingCommand, PrefixArg},
    input::{KeyCode, KeyEvent},
};
use std::sync::Arc;

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

    /// Register NAME as a command taking SPECS.
    ///
    /// Idempotent by name: re-registering replaces the previous specs, so a
    /// user can change how an existing command prompts without restarting.
    pub fn register(&mut self, name: &str, specs: Vec<ArgSpec>) {
        self.registry.insert(name.to_string(), specs);
    }

    /// The arguments to collect for NAME, or `None` if it is not a command.
    ///
    /// Owned, like everything else here. See the note at the top of the file.
    pub fn specs(&self, name: &str) -> Option<Vec<ArgSpec>> {
        self.registry.get(name).cloned()
    }

    pub fn is_command(&self, name: &str) -> bool {
        self.registry.contains_key(name)
    }

    /// Every command name, sorted, for `M-x` completion.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.registry.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    // ------------------------------------------------------------------
    // The prefix argument
    // ------------------------------------------------------------------

    /// The argument waiting for the next command, if any.
    pub fn prefix_argument(&self) -> Option<PrefixArg> {
        self.prefix_arg
    }

    /// Forget the argument. Called after every command, whether or not
    /// anything consumed it: an argument belongs to exactly one command, and
    /// one that errors must not leave its argument for the next.
    pub fn clear_prefix_argument(&mut self) {
        self.prefix_arg = None;
        self.reading_prefix = false;
    }

    /// Offer EVENT to the prefix-argument reader, and say whether it was taken.
    ///
    /// Called before the key sequence gets a look, because `C-u` and the digits
    /// that follow it are not keys the keymaps should ever see. Anything the
    /// reader does not want falls straight through -- including a digit typed
    /// when no argument is being built, which is how you can still type the
    /// number four into a buffer.
    pub fn read_prefix_argument(&mut self, event: &KeyEvent) -> bool {
        // C-u: start an argument, or multiply the one being built by four.
        if event.modifiers.ctrl && !event.modifiers.alt && event.code == KeyCode::Char('u') {
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

        // Whatever this key is, it is not part of the argument -- so the
        // argument is finished, even though it has not been used yet.
        self.reading_prefix = false;
        false
    }

    // ------------------------------------------------------------------
    // Arguments still being collected
    // ------------------------------------------------------------------

    /// Begin collecting arguments for NAME.
    pub fn push_pending(&mut self, name: String, remaining: Vec<ArgSpec>, invocation: Invocation) {
        self.pending
            .push(PendingCommand::new(name, remaining, invocation));
    }

    /// Answer every argument the editor can answer itself, in order, and
    /// return the first one that still needs the user.
    ///
    /// `None` means the command has everything it needs and is ready to run.
    ///
    /// Done in one loop with the prompted arguments rather than as a separate
    /// pass: a spec list like `["p", "sReplace with: "]` interleaves the two
    /// kinds, and two code paths that both maintain the pending stack would
    /// have to agree about it forever.
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

    /// How far through its arguments the innermost pending command is:
    /// `(command name, 1-based position of the argument being read, total)`.
    ///
    /// Used to title the prompt, so that answering the second of two questions
    /// says which command asked and which question it is.
    pub fn pending_progress(&self) -> Option<(String, usize, usize)> {
        let pending = self.pending.last()?;
        let done = pending.collected.len();
        Some((
            pending.name.clone(),
            done + 1,
            done + pending.remaining.len(),
        ))
    }

    /// The argument the innermost pending command is waiting on.
    pub fn pending_current_spec(&self) -> Option<ArgSpec> {
        self.pending
            .last()
            .and_then(|pending| pending.current().cloned())
    }

    /// Record VALUE as the innermost pending command's next argument.
    ///
    /// What to do next is [`Commands::fill_answerable_args`]'s answer, not this
    /// one's: the specs after this may be a mix of answerable and prompted,
    /// and only one place should know how to walk them.
    pub fn accept_pending_arg(&mut self, value: ELispExp<B>) {
        let Some(pending) = self.pending.last_mut() else {
            return;
        };
        if !pending.remaining.is_empty() {
            pending.remaining.remove(0);
        }
        pending.collected.push(value);
    }

    /// Remove and return the innermost pending command.
    pub fn take_pending(&mut self) -> Option<(String, Vec<ELispExp<B>>)> {
        self.pending
            .pop()
            .map(|pending| (pending.name, pending.collected))
    }

    /// Drop every pending command.
    ///
    /// Called when a fresh command starts with no minibuffer open, which means
    /// any entry still on the stack belongs to a prompt that was closed by some
    /// path other than confirm or cancel. Without this, that orphan would be
    /// fed the *next* command's input.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    // ------------------------------------------------------------------
    // What ran last
    // ------------------------------------------------------------------

    /// Whether the previous command was NAME.
    ///
    /// A predicate rather than a getter because the question asked of
    /// `last-command` is always "was it this one?", and answering it by
    /// handing out a copy of the name would allocate on a path that runs
    /// between a key being pressed and the character appearing.
    pub fn last_was(&self, name: &str) -> bool {
        self.last_name.as_deref().map(String::as_str) == Some(name)
    }

    pub fn set_last_name(&mut self, name: Option<Arc<String>>) {
        self.last_name = name;
    }

    /// The last command as a runnable form, for `repeat`.
    pub fn last_form(&self) -> Option<ELispExp<B>> {
        self.last_form.clone()
    }

    pub fn set_last_form(&mut self, form: Option<ELispExp<B>>) {
        self.last_form = form;
    }
}

/// What the editor hands a command for an argument it answers itself.
///
/// One spec can produce more than one value -- `r` is the region's *two* ends,
/// exactly as `interactive "r"` is in Emacs -- so this returns a list rather
/// than a single value.
///
/// Pure in `(spec, invocation)`: everything it needs was captured when the
/// command began, which is what lets it run with the compartment's lock held
/// and nothing else's.
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
        ArgSpec::RawCount => vec![match invocation.prefix_arg {
            None => ELispExp::nil(),
            Some(PrefixArg::Raw(times)) => {
                ELispExp::proper_list(vec![ELispExp::number(4i32.saturating_pow(times) as f64)])
            }
            Some(PrefixArg::Number(n)) => ELispExp::number(n as f64),
            Some(PrefixArg::Negative) => ELispExp::symbol("-".into()),
        }],
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
