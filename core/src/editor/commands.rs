//! The commands' half of the facade: running a command, with what that does to
//! the editor around it -- the undo group, the hooks, the flags rolled over,
//! the argument used up -- and collecting the arguments a command needs.
//!
//! Design: `doc/architettura-editor.typ, <esecuzione>`.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    // ------------------------------------------------------------------
    // Running a command
    // ------------------------------------------------------------------

    /// Runs AST as a command: one undo group, `pre-command-hook` before it and
    /// `post-command-hook` after, and one entry in `last-command`.
    ///
    /// Every way of running a command comes through here -- a key, a paste, a
    /// mouse event, a captured sequence, a replayed macro -- so that each is
    /// grouped, hooked and remembered as a key is.
    pub(super) fn run_command_form(&self, mut ast: ELispExp<B>, env: &Arc<Env<EditorState<B>>>) {
        // A binding that supplies no arguments -- `next-line`, `(next-line)` --
        // names a command that may declare some, and goes through
        // `call-interactively`, which collects them. One that supplies its own,
        // `(self-insert "a")` as every ordinary key does, runs as it is.
        // TODO(CMD-5): so it never sees the prefix argument: `C-u 3 x` inserts
        // one x.
        let bound_command = match &ast {
            ELispExp::Symbol(name) => Some(name.to_string()),
            ELispExp::Form(items) if items.len() == 1 => match &items[0] {
                ELispExp::Symbol(name) => Some(name.to_string()),
                _ => None,
            },
            _ => None,
        };
        // The command the binding names, whether or not it supplies the
        // arguments: `(self-insert "a")` names `self-insert` as much as
        // `next-line` does, and the undo group and `last-command` want it.
        let this_command = match &ast {
            ELispExp::Symbol(name) => Some(name.clone()),
            ELispExp::Form(items) => items.first().and_then(|head| match head {
                ELispExp::Symbol(name) => Some(name.clone()),
                _ => None,
            }),
            _ => None,
        };
        // Told to the undo history before the command runs, so that all of its
        // edits land in one group: a thread-local stamp and an atomic, nothing
        // for the keys that edit nothing.
        crate::buffer::undo::begin_command(
            this_command.as_deref().map(String::as_str) == Some("self-insert"),
        );
        // Before the command, which asks these about its predecessor; and so a
        // command that fails cannot leave its flag up for the next one.
        self.roll_over_command_flags();
        if let Some(name) = bound_command {
            // The name as a string, which evaluates to itself: no `quote`, and
            // no chance of its being read as a variable.
            ast = ELispExp::form(vec![
                ELispExp::symbol("call-interactively".into()),
                ELispExp::string(name),
            ]);
        }

        let outcome = {
            let _command = self.begin_command();
            let mode = self.with_current_buffer(|buf| buf.current_mode.clone());
            self.run_hook(&mode, "pre-command-hook", env);
            eval(&ast, env.clone(), self)
        };
        // Before the error is looked at: a command that fails still uses its
        // argument up.
        self.clear_prefix_argument();
        if let Err(e) = outcome {
            self.report_error(&format!("{:?} {:?}", ast, e), env);
            return;
        }

        let current_mode_name = self.with_current_buffer(|buf| buf.current_mode.clone());
        // Only after a typed character, and before `post-command-hook`:
        // electric-pair hangs here, and pairs before a mode looks at the line.
        // A paste arrives as `insert-pasted-text`, and is not paired again.
        if this_command.as_deref().map(String::as_str) == Some("self-insert") {
            self.run_hook(&current_mode_name, "post-self-insert-hook", env);
        }
        self.run_hook(&current_mode_name, "post-command-hook", env);
        // Not for `repeat` itself, which would then repeat the repeating.
        if this_command.as_deref().map(String::as_str) != Some("repeat") {
            self.commands_mut(|commands| commands.set_last_form(Some(ast)));
        }
        // After the hooks, so that the repeat key goes through every step the
        // command went through.
        self.install_repeat_keymap(this_command.as_deref().map(String::as_str));
        // For the next command: during this one, `last-command` names the one
        // before, which is what lets a command ask whether it repeats itself.
        self.set_last_command(this_command);
    }

    // ------------------------------------------------------------------
    // The prefix argument
    // ------------------------------------------------------------------

    /// Offers EVENT to the prefix argument's reader, and returns whether it
    /// took it. The key handler asks before any keymap sees the key.
    pub(super) fn read_prefix_argument(&self, event: &KeyEvent) -> bool {
        self.commands_mut(|commands| commands.read_prefix_argument(event))
    }

    /// Returns the argument waiting for the next command, if any.
    pub(crate) fn prefix_argument(&self) -> Option<PrefixArg> {
        self.commands(|commands| commands.prefix_argument())
    }

    /// Forgets the argument: after every command, whether or not it used it,
    /// since an argument belongs to one command, and one that failed must not
    /// leave it to the next.
    pub(crate) fn clear_prefix_argument(&self) {
        self.commands_mut(|commands| commands.clear_prefix_argument());
    }

    /// Gives the next command ARG, as though it had been typed.
    pub(crate) fn set_prefix_argument(&self, arg: Option<PrefixArg>) {
        self.commands_mut(|commands| commands.set_prefix_argument(arg));
    }

    // ------------------------------------------------------------------
    // The arguments of a command in flight
    // ------------------------------------------------------------------

    /// Starts collecting the arguments of NAME: REMAINING, for INVOCATION.
    pub(crate) fn push_pending_command(
        &self,
        name: String,
        remaining: Vec<ArgSpec>,
        invocation: Invocation,
    ) {
        self.commands_mut(|commands| commands.push_pending(name, remaining, invocation));
    }

    /// Returns what the editor can answer for the user as a command starts:
    /// the prefix argument, and the region. Read once, then: see
    /// [`Invocation`].
    pub(crate) fn capture_invocation(&self) -> Invocation {
        // The argument is read before the buffer is locked: one compartment at
        // a time.
        let prefix_arg = self.prefix_argument();
        let region = self.with_current_buffer(|buf| {
            crate::buffer::mark::region_bounds(buf.mark, buf.text.cursor_pos_1d(), buf.text.len())
        });
        Invocation { prefix_arg, region }
    }

    /// Answers every argument the editor can answer itself, in order, and
    /// returns the first that needs the user: `None` when the command has
    /// everything it needs.
    ///
    /// One loop with the prompted ones, since a list of specs interleaves the
    /// two kinds: `["p", "sReplace with: "]`.
    pub(crate) fn fill_answerable_args(&self) -> Option<ArgSpec> {
        self.commands_mut(|commands| commands.fill_answerable_args())
    }

    /// Returns how far the innermost pending command has got: its name, the
    /// position of the argument being read, from 1, and how many there are.
    /// It titles the prompt, so that the second of two questions says whose
    /// it is and which.
    pub(crate) fn pending_progress(&self) -> Option<(String, usize, usize)> {
        self.commands(|commands| commands.pending_progress())
    }

    /// Returns the argument the innermost pending command is waiting for.
    pub(crate) fn pending_current_spec(&self) -> Option<ArgSpec> {
        self.commands(|commands| commands.pending_current_spec())
    }

    /// Records VALUE as the innermost pending command's next argument. What
    /// comes next is [`Self::fill_answerable_args`]'s to say.
    pub(crate) fn accept_pending_arg(&self, value: ELispExp<B>) {
        self.commands_mut(|commands| commands.accept_pending_arg(value));
    }

    /// Removes the innermost pending command, and returns its name and its
    /// arguments.
    pub(crate) fn take_pending_command(&self) -> Option<(String, Vec<ELispExp<B>>)> {
        self.commands_mut(|commands| commands.take_pending())
    }

    /// Drops every pending command. Called when a command starts with no
    /// prompt open: a command left by a prompt closed some other way than by
    /// confirming or cancelling must not be fed the next command's input.
    pub(crate) fn clear_pending_commands(&self) {
        self.commands_mut(|commands| commands.clear_pending());
    }

    // ------------------------------------------------------------------
    // The last command, and the registry
    // ------------------------------------------------------------------

    /// Returns whether the previous command was NAME: a question rather than a
    /// copy of the name, which would allocate between a key and its character
    /// appearing.
    pub(crate) fn last_command_is(&self, name: &str) -> bool {
        self.commands(|commands| commands.last_was(name))
    }

    /// Returns the previous command as a form that runs it again, for
    /// `repeat`.
    pub(crate) fn last_command_form(&self) -> Option<ELispExp<B>> {
        self.commands(|commands| commands.last_form())
    }

    /// Records NAME as the command that ran last.
    pub(crate) fn set_last_command(&self, name: Option<Arc<String>>) {
        self.commands_mut(|commands| commands.set_last_name(name));
    }

    /// Registers NAME as a command taking SPECS, replacing the specs it had: a
    /// command's prompts can change without a restart.
    pub(crate) fn register_command(&self, name: &str, specs: Vec<ArgSpec>) {
        self.commands_mut(|commands| commands.register(name, specs));
    }

    /// Returns the arguments NAME collects, or `None` when it is not a command.
    ///
    /// Owned, like everything here: `call-interactively` looks a command up and
    /// then evaluates Lisp, which may register one.
    pub(crate) fn command_specs(&self, name: &str) -> Option<Vec<ArgSpec>> {
        self.commands(|commands| commands.specs(name))
    }

    /// Returns whether NAME is a command.
    pub(crate) fn is_command(&self, name: &str) -> bool {
        self.commands(|commands| commands.is_command(name))
    }

    /// Returns every command's name, sorted, for `M-x` to complete.
    pub(crate) fn command_names(&self) -> Vec<String> {
        self.commands(|commands| commands.names())
    }
}
