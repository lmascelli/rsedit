//! Running a command, and collecting the arguments it needs first.
//!
//! The compartment is [`crate::managers::Commands`]. What is here is what a
//! command does to the editor around it: the undo group, the hooks, the flags
//! rolled over for the next one.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Run AST as a command: one undo group, one `post-command-hook`, one
    /// entry in `last-command`.
    ///
    /// Split out of [`Self::handle_key_event`] because a keystroke is no
    /// longer the only thing that runs a command -- a bracketed paste does
    /// too, and it has to be grouped, hooked and remembered exactly as a
    /// keystroke is. Two dispatch paths would drift; one cannot.
    pub(super) fn run_command_form(&self, mut ast: ELispExp<B>, env: &Arc<Env<EditorState<B>>>) {
        let bound_command = match &ast {
            ELispExp::Symbol(name) => Some(name.to_string()),
            ELispExp::Form(items) if items.len() == 1 => match &items[0] {
                ELispExp::Symbol(name) => Some(name.to_string()),
                _ => None,
            },
            _ => None,
        };
        // The command this key runs, whether or not the binding supplies its
        // arguments: `(self-insert "a")` names `self-insert` just as much as a
        // bare `next-line` does. `bound_command` above deliberately matches
        // only the argument-less shapes, because those are the ones to route
        // through `call-interactively`; grouping and `last-command` want the
        // wider answer, since typing is exactly the case they care about.
        //
        // Remembered for the *next* command to consult, so that during this
        // one `last_command` still names its predecessor -- which is what
        // makes "am I a repeat of myself?" answerable.
        let this_command = match &ast {
            ELispExp::Symbol(name) => Some(name.clone()),
            ELispExp::Form(items) => items.first().and_then(|head| match head {
                ELispExp::Symbol(name) => Some(name.clone()),
                _ => None,
            }),
            _ => None,
        };
        // Told to the undo history before the command runs, so that its edits
        // -- however many it makes -- land in one group. This is a thread-local
        // stamp and an atomic increment: it costs nothing on the keys that
        // edit nothing, which is most of them.
        crate::buffer::undo::begin_command(
            this_command.as_deref().map(String::as_str) == Some("self-insert"),
        );
        // Before the command runs, not after: a command asks these about its
        // predecessor, so the answer has to be in place by the time it starts.
        // Rolling over here rather than at the end also means a command that
        // fails partway cannot leave its flag set for the next one to read.
        self.roll_over_command_flags();
        if let Some(name) = bound_command {
            // The name is passed as a string rather than a quoted symbol:
            // a string literal is self-evaluating, so this needs no `quote`
            // and cannot be mistaken for a variable reference.
            ast = ELispExp::form(vec![
                ELispExp::symbol("call-interactively".into()),
                ELispExp::string(name),
            ]);
        }

        let outcome = {
            let _command = self.begin_command();
            eval(&ast, env.clone(), self)
        };
        // Before the error check, so a command that fails still consumes its
        // argument rather than leaving it for whatever runs next.
        self.clear_prefix_argument();
        if let Err(e) = outcome {
            self.report_error(&format!("{:?} {:?}", ast, e), env);
            return;
        }

        // Handle the post-command hooks
        let current_mode_name = self.with_current_buffer(|buf| buf.current_mode.clone());
        // Before `post-command-hook', and only for the command that typed a
        // character. This is where electric-pair and anything else that
        // reacts to typing hangs; running it after the general hook would put
        // the pair in after a mode had already looked at the line.
        //
        // Deliberately *not* run for a paste, which reaches the buffer as
        // `insert-pasted-text' rather than as a run of `self-insert'. A hook
        // that fires per character would auto-pair every bracket in pasted
        // code, which is exactly the mangling bracketed paste exists to stop.
        if this_command.as_deref().map(String::as_str) == Some("self-insert") {
            self.run_hook(&current_mode_name, "post-self-insert-hook", env);
        }
        self.run_hook(&current_mode_name, "post-command-hook", env);
        // Offered after the command has run and its hooks have fired, so that
        // pressing the repeat key goes through every step the first invocation
        // did. A command with no repeat key takes down whatever the previous
        // one offered, which is what ends a run of `C-x o o o`.
        // Remembered only for a command that is not itself `repeat'. Were
        // `repeat' to overwrite this with its own form, the second press would
        // repeat the repeating rather than the thing repeated, and every press
        // after that would too.
        if this_command.as_deref().map(String::as_str) != Some("repeat") {
            self.commands_mut(|commands| commands.set_last_form(Some(ast)));
        }
        self.install_repeat_keymap(this_command.as_deref().map(String::as_str));
        self.set_last_command(this_command);
    }

    /// Run every function registered under HOOK_NAME in the major mode
    /// named MODE_NAME, in registration order, each called with zero
    /// arguments. Errors from an individual hook are logged and
    /// otherwise swallowed, so one broken hook can't block the rest.
    pub(crate) fn run_hook(
        &self,
        mode_name: &str,
        hook_name: &str,
        env: &Arc<Env<EditorState<B>>>,
    ) {
        // Copy the hook list out and release the registry *before* evaluating
        // any of it.
        //
        // Holding the lock across `eval` deadlocks the editor outright: a hook
        // that calls `add-hook`, `define-key` with a mode, `make-mode` or
        // `add-syntax-rule` needs a write lock on the same registry this thread
        // is already reading, and `RwLock` is not reentrant. It is not a race --
        // it hangs every time, on one thread, from ordinary user Lisp.
        //
        // The general rule this is an instance of: never hold a lock across a
        // callback into the interpreter. Lisp can re-enter the editor through
        // any primitive, so a lock held across `eval` is a lock offered to
        // arbitrary code.
        // One acquisition for both halves. The mode's own hooks and the ones
        // registered for every mode used to be two locks read in sequence; they
        // are one compartment now, so a hook added to one half between the two
        // reads can no longer produce a list that was never true at any instant.
        let hooks = self.modes(|modes| modes.hooks_for(mode_name, hook_name));

        for hook in hooks {
            let hook_call = ELispExp::form(vec![hook.clone()]);
            let _command = self.begin_command();
            if let Err(e) = eval(&hook_call, env.clone(), self) {
                self.log_diagnostic(&format!(
                    "Hook {hook_name} ({:?}) execution failed: {:?}",
                    hook, e
                ));
            }
        }
    }

    /// Open a fresh Lisp execution budget for one top-level command -- a
    /// keystroke, a hook run, a config file being loaded.
    ///
    /// What counts as "one command" is editor *policy*, which is why this lives
    /// here rather than in `FuelMeter`: only the editor knows a keystroke is one
    /// unit of work. Nesting is safe -- the meter tracks depth and only the
    /// outermost scope refills -- so a command that re-enters the evaluator, via
    /// the Lisp-callable `eval-file` primitive for instance, keeps spending the
    /// budget it already has instead of quietly being handed a new one.
    pub(crate) fn begin_command(&self) -> FuelScope {
        // The meter is cloned out and the lock given straight back: a metered
        // scope lasts a whole command, and holding this lock for that long
        // would be holding it across the interpreter.
        self.runtime(|runtime| runtime.fuel()).begin()
    }

    /// The execution meter behind [`Self::begin_command`].
    ///
    /// Exposed for `lisp::measure`, which needs the meter to hold a scope of
    /// its own for the duration of a measurement.
    pub(crate) fn fuel_meter(&self) -> Arc<FuelMeter> {
        self.runtime(|runtime| runtime.fuel())
    }

    // ---------------------------------------------------------------
    // Argument collection for a command in flight
    // ---------------------------------------------------------------

    /// Begin collecting arguments for NAME.
    pub(crate) fn push_pending_command(
        &self,
        name: String,
        remaining: Vec<ArgSpec>,
        invocation: Invocation,
    ) {
        self.commands_mut(|commands| commands.push_pending(name, remaining, invocation));
    }

    /// Everything the editor can answer on the user's behalf, as it stands now.
    ///
    /// Taken once, when a command starts. See [`Invocation`] for why it is not
    /// read again later.
    pub(crate) fn capture_invocation(&self) -> Invocation {
        // The prefix argument is read *before* the buffer is locked: two
        // compartments, never held together.
        let prefix_arg = self.prefix_argument();
        let region = self.with_current_buffer(|buf| {
            crate::buffer::mark::region_bounds(buf.mark, buf.text.cursor_pos_1d(), buf.text.len())
        });
        Invocation { prefix_arg, region }
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
    pub(crate) fn fill_answerable_args(&self) -> Option<ArgSpec> {
        self.commands_mut(|commands| commands.fill_answerable_args())
    }

    /// How far through its arguments the innermost pending command is:
    /// `(command name, 1-based position of the argument being read, total)`.
    ///
    /// Used to title the prompt, so that answering the second of two questions
    /// says which command asked and which question it is. Without it a prompt
    /// reading `Find file:` gives no hint that `find-file` is what is waiting
    /// on the answer -- and with two arguments, no hint of which one is being
    /// asked for.
    pub(crate) fn pending_progress(&self) -> Option<(String, usize, usize)> {
        self.commands(|commands| commands.pending_progress())
    }

    /// The argument the innermost pending command is waiting on.
    pub(crate) fn pending_current_spec(&self) -> Option<ArgSpec> {
        self.commands(|commands| commands.pending_current_spec())
    }

    /// Record VALUE as the innermost pending command's next argument.
    ///
    /// What to do next is [`Self::fill_answerable_args`]'s answer, not this
    /// one's: the specs after this may be a mix of answerable and prompted,
    /// and only one place should know how to walk them.
    pub(crate) fn accept_pending_arg(&self, value: ELispExp<B>) {
        self.commands_mut(|commands| commands.accept_pending_arg(value));
    }

    /// Remove and return the innermost pending command.
    pub(crate) fn take_pending_command(&self) -> Option<(String, Vec<ELispExp<B>>)> {
        self.commands_mut(|commands| commands.take_pending())
    }

    /// Drop every pending command.
    ///
    /// Called when a fresh command starts with no minibuffer open, which means
    /// any entry still on the stack belongs to a prompt that was closed by some
    /// path other than confirm or cancel. Without this, that orphan would be
    /// fed the *next* command's input.
    pub(crate) fn clear_pending_commands(&self) {
        self.commands_mut(|commands| commands.clear_pending());
    }

    /// Whether the previous command was NAME.
    ///
    /// A predicate rather than a getter because the question asked of
    /// `last-command` is always "was it this one?", and answering it by
    /// handing out a copy of the name would allocate on a path that runs
    /// between a key being pressed and the character appearing.
    pub(crate) fn last_command_is(&self, name: &str) -> bool {
        self.commands(|commands| commands.last_was(name))
    }

    // ---------------------------------------------------------------
    // Faces and the theme
    // ---------------------------------------------------------------

    /// Roll "this command" into "the previous command" for the flags that a
    /// command needs to ask about its predecessor.
    fn roll_over_command_flags(&self) {
        self.kill_yank_mut(|kills| kills.roll_over());
    }

    /// Register FUNCTION to run under HOOK_NAME in every major mode.
    pub(crate) fn add_global_hook(&self, hook_name: &str, function: ELispExp<B>) {
        self.modes_mut(|modes| modes.add_global_hook(hook_name, function));
    }

    /// The last command as a runnable form, for `repeat`.
    pub(crate) fn last_command_form(&self) -> Option<ELispExp<B>> {
        self.commands(|commands| commands.last_form())
    }

    pub(crate) fn set_last_command(&self, name: Option<Arc<String>>) {
        self.commands_mut(|commands| commands.set_last_name(name));
    }

    /// Register NAME as a command taking SPECS.
    ///
    /// Idempotent by name: re-registering replaces the previous specs, so a
    /// user can change how an existing command prompts without restarting.
    pub(crate) fn register_command(&self, name: &str, specs: Vec<ArgSpec>) {
        self.commands_mut(|commands| commands.register(name, specs));
    }

    /// The arguments to collect for NAME, or `None` if it is not a command.
    ///
    /// Returns owned data, and every other accessor here does too. That is not
    /// incidental: `call-interactively` looks a command up and then evaluates
    /// Lisp, and Lisp can call `register-command`. Handing back a guard would
    /// mean holding this lock across `eval` -- exactly the reentrancy that
    /// deadlocked `run_hook`.
    pub(crate) fn command_specs(&self, name: &str) -> Option<Vec<ArgSpec>> {
        self.commands(|commands| commands.specs(name))
    }

    pub(crate) fn is_command(&self, name: &str) -> bool {
        self.commands(|commands| commands.is_command(name))
    }

    /// Every command name, sorted, for M-x completion.
    pub(crate) fn command_names(&self) -> Vec<String> {
        self.commands(|commands| commands.names())
    }

    /// Set how much fuel a fresh command receives, and top the current thread's
    /// remaining fuel up to it. Exposed so the `set-command-fuel` primitive --
    /// and tests that want a deliberately tiny budget -- can reach it.
    pub(crate) fn set_fuel_budget(&self, budget: u32) {
        self.runtime(|runtime| runtime.fuel()).set_budget(budget);
    }
}
