//! From a keystroke to a command: the prefix argument, the sequence being
//! typed, and the keymaps that resolve it.

use super::*;

/// The variable naming what is waiting to be told the next key sequence.
///
/// The arming *is* the variable: one fact in one place, readable and settable
/// from Lisp like any other, and nil when nothing is capturing. It costs one
/// variable lookup per keystroke, which is a hash probe beside the three
/// compartment locks the same keystroke already takes.
pub const KEY_CAPTURE_FUNCTION: &str = "*key-capture-function*";

/// What the keymaps had to say about the key sequence typed so far.
///
/// Four answers rather than the `(Option<ELispExp>, bool)` pair this replaced.
/// That pair could represent `(Some(command), true)` -- bound *and* a prefix --
/// which is not a thing a keymap can mean, and it had no way at all to say
/// "bound to nothing, and say nothing about it", which the transient map's
/// `Refuse` needs; that case had to return early from the middle of the lookup
/// instead.
enum Bound<B: BufferTrait> {
    /// Run this.
    Command(ELispExp<B>),
    /// Part-way through a sequence. The keys are kept.
    Prefix,
    /// Bound to nothing, and nothing is to be said about it.
    Refused,
    /// Bound to nothing. Say so.
    Unbound,
}

impl<B: BufferTrait> EditorState<B> {
    /// Handle a key event. An UI provider is responsible to call this function
    /// every time it want to make the editor react to an user input.
    /// Offer EVENT to the prefix-argument reader, and say whether it was taken.
    ///
    /// Called before the key sequence gets a look, because `C-u` and the digits
    /// that follow it are not keys the keymaps should ever see. Anything the
    /// reader does not want falls straight through -- including a digit typed
    /// when no argument is being built, which is how you can still type the
    /// number four into a buffer.
    fn read_prefix_argument(&self, event: &KeyEvent) -> bool {
        self.commands_mut(|commands| commands.read_prefix_argument(event))
    }

    /// The mode of the buffer the keys would be typed into.
    ///
    /// Taken before the modes lock, everywhere: buffers come before modes in
    /// the canonical order.
    fn keymap_mode(&self) -> String {
        self.with_current_buffer(|buf| buf.current_mode.clone())
    }

    /// What KEYS would run if they were typed now, and which map it is from.
    ///
    /// The same walk the editor itself does -- see [`Modes::binding_for`] --
    /// so what this reports and what would happen cannot disagree. `None`
    /// means nothing would run, which includes a sequence that is only the
    /// start of a longer one.
    pub(crate) fn key_binding(&self, keys: &[KeyEvent]) -> Option<Binding<B>> {
        let mode = self.keymap_mode();
        self.modes(|modes| modes.binding_for(Some(&mode), keys))
    }

    /// Whether KEYS is the start of a longer binding.
    pub(crate) fn key_is_prefix(&self, keys: &[KeyEvent]) -> bool {
        let mode = self.keymap_mode();
        self.modes(|modes| modes.is_prefix(Some(&mode), keys))
    }

    /// Every binding in effect in the current buffer, sorted, each sequence
    /// once.
    pub(crate) fn key_bindings(&self) -> Vec<Binding<B>> {
        let mode = self.keymap_mode();
        self.modes(|modes| modes.bindings(Some(&mode)))
    }

    /// Install a keymap that is consulted before every other until it goes
    /// away. See [`TransientKeymap`].
    pub(crate) fn set_transient_keymap(&self, map: TransientKeymap<B>) {
        self.modes_mut(|modes| modes.set_transient(map));
    }

    /// Take the map down. Idempotent, so a command that ends one can call it
    /// without first asking whether one is up.
    pub(crate) fn clear_transient_keymap(&self) {
        self.modes_mut(|modes| modes.clear_transient());
    }

    /// What the installed map wants shown, or empty when none is installed.
    pub(crate) fn transient_message(&self) -> String {
        self.modes(|modes| modes.transient_message())
    }

    /// Whether a transient keymap is installed. Only for reporting.
    pub(crate) fn transient_keymap_active(&self) -> bool {
        self.modes(|modes| modes.transient_active())
    }

    /// Say that COMMAND may be repeated by pressing KEYS on its own afterwards.
    ///
    /// Declared rather than inferred. The tempting rule -- "after a sequence
    /// ending in K, a bare K repeats" -- would make `C-x C-f` followed by `f`
    /// re-open `find-file`, which is not a convenience.
    pub(crate) fn set_repeat_key(&self, command: &str, key: KeyEvent) {
        self.modes_mut(|modes| modes.set_repeat_key(command, key));
    }

    /// The key that repeats COMMAND, if it has one.
    fn repeat_key(&self, command: &str) -> Option<KeyEvent> {
        self.modes(|modes| modes.repeat_key(command))
    }

    /// Offer to repeat COMMAND, if it said it could be.
    ///
    /// Run after every command, which is what makes repeating work by the same
    /// path whether the command was reached by its full sequence or by the
    /// repeat key it offered last time.
    pub(super) fn install_repeat_keymap(&self, command: Option<&str>) {
        let Some(key) = command.and_then(|name| self.repeat_key(name)) else {
            // Nothing to offer, and nothing to take down either: a `Release`
            // map was already dismissed by key resolution before this command
            // ran, which is what ends a run of `C-x o o o`.
            //
            // Taking one down here as well would be actively wrong. A `Refuse`
            // map -- a question -- is answered by *its own* commands, and this
            // runs after every one of them: clearing here would dismiss the
            // question the moment it was answered, before the next one could
            // be asked.
            return;
        };
        self.set_transient_keymap(TransientKeymap::repeating(
            key,
            command.expect("a repeat key was found, so there is a command"),
        ));
    }

    /// The argument waiting for the next command, if any.
    pub(crate) fn prefix_argument(&self) -> Option<PrefixArg> {
        self.commands(|commands| commands.prefix_argument())
    }

    /// Abandon whatever is half-finished: a key sequence, a prefix argument.
    ///
    /// The text-level half of `keyboard-quit`. Interrupting a *running* command
    /// is roadmap #24 and needs more than this; abandoning something not yet
    /// started needs only this, and is what a mistyped `C-x` calls for.
    pub(crate) fn abandon_pending_input(&self) {
        self.pending_keys
            .write()
            .expect("Failed to acquire write lock on pending_keys")
            .clear();
        self.clear_prefix_argument();
    }

    /// What the editor is part-way through reading, spelt the way a binding
    /// is written and ending in a dash for the key still to come.
    ///
    /// The argument and the key sequence appear together -- `C-u 4 C-x-` --
    /// because they compose, and showing only one of them would misreport
    /// what pressing the next key will do.
    pub(crate) fn pending_input(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(arg) = self.prefix_argument() {
            parts.push(arg.describe());
        }
        let keys = self
            .pending_keys
            .read()
            .expect("Failed to acquire read lock on pending_keys");
        if !keys.is_empty() {
            parts.push(describe_keys(&keys));
        }
        if parts.is_empty() {
            return String::new();
        }
        format!("{}-", parts.join(" "))
    }

    /// Forget the argument. Called after every command, whether or not
    /// anything consumed it: an argument belongs to exactly one command, and
    /// one that errors must not leave its argument for the next.
    pub(crate) fn clear_prefix_argument(&self) {
        self.commands_mut(|commands| commands.clear_prefix_argument());
    }

    /// Add EVENT to the sequence being typed, and say what to run.
    ///
    /// `Some(ast)` means the sequence is now a complete binding and has been
    /// cleared ready for the next one. `None` means there is nothing to run --
    /// either because more keys are expected, or because the sequence is bound
    /// to nothing.
    ///
    /// # Why this returns rather than dispatching
    ///
    /// A key that only lengthens a sequence is **not a command**. It must not
    /// reach `begin_command`, `roll_over_command_flags` or `set_last_command`,
    /// all of which live in the caller below the point this returns `None`.
    /// Letting `C-x` through any of them would silently end the undo group
    /// being typed into, and split a run of kills into two ring entries --
    /// neither of which looks like a key-handling bug when you go looking.
    fn resolve_key_sequence(&self, event: KeyEvent) -> Option<ELispExp<B>> {
        let mut pending = self
            .pending_keys
            .write()
            .expect("Failed to acquire write lock on pending_keys");
        pending.push(event);

        // Asked before the modes are taken: buffers come before modes in the
        // canonical order.
        let current_mode = self.with_current_buffer(|buf| buf.current_mode.clone());

        // One acquisition answers the whole question -- the transient map, the
        // mode's keymap and the global one. It used to be three locks and an
        // atomic gate in front of the first of them, with a written protocol
        // keeping the gate and the map it guarded in step. See [`Modes`].
        let mut release_transient = false;
        let bound = self.modes(|modes| {
            // A transient keymap is consulted before every other, for as long
            // as it is installed. See `TransientKeymap`.
            if let Some(map) = modes.transient() {
                let hit = map.keymap.get(&pending).cloned();
                let prefix = map.keymap.is_prefix(&pending);
                match (hit, prefix, map.on_unbound) {
                    (Some(ast), _, _) => return Bound::Command(ast),
                    // Part-way through one of the map's own sequences.
                    (None, true, _) => return Bound::Prefix,
                    // Refused, and nothing said about it: the map's message is
                    // still in the frame, and anything written to the echo
                    // area would be drawn under it.
                    (None, false, OnUnbound::Refuse) => return Bound::Refused,
                    // Handed on, and the map stays. The key carries on to the
                    // keymaps below and the map is consulted again next time,
                    // which is what lets the completion strip be typed at
                    // without either swallowing the letter or dismissing
                    // itself.
                    (None, false, OnUnbound::Pass) => {}
                    // Handed on, and the map goes. The key carries on exactly
                    // as though it had never been there -- which is what makes
                    // the offer free to ignore. Taken down after this lock is
                    // given back, since dropping it needs the write side.
                    (None, false, OnUnbound::Release) => release_transient = true,
                }
            }

            // The mode's own keymap wins, then the global one -- and a mode
            // that binds a prefix keeps the sequence alive even when only the
            // global map completes it. The same two calls answer `key-binding`
            // for a help command, so what is reported and what runs are one
            // walk rather than two that have to be kept in step.
            let hit = modes.binding_below_transient(Some(&current_mode), &pending);
            let prefix = modes.is_prefix_below_transient(Some(&current_mode), &pending);
            match (hit, prefix) {
                (Some(binding), _) => Bound::Command(binding.target),
                (None, true) => Bound::Prefix,
                (None, false) => Bound::Unbound,
            }
        });
        if release_transient {
            // Holding the bindings of a map nobody can reach would be a small
            // leak that lasted until the next one was installed.
            self.clear_transient_keymap();
        }

        // Everything the keymaps had to say, said, and the lock given back --
        // so reporting, which writes the echo area, happens under none of it.
        let described = describe_keys(&pending);
        if !matches!(bound, Bound::Prefix) {
            pending.clear();
        }
        drop(pending);

        match bound {
            Bound::Command(ast) => Some(ast),
            // Nothing to say: the sequence is in `pending_input`, which the
            // frame carries and which does not expire the way a message does.
            Bound::Prefix | Bound::Refused => None,
            Bound::Unbound => {
                self.set_echo_message(&format!("{described} is undefined"));
                self.log_diagnostic(&format!("[INFO] Keymap not bound {described}"));
                None
            }
        }
    }

    /// Read the next key sequence and hand it to WATCHER instead of running
    /// it.
    ///
    /// # Why this sits on the real resolution path
    ///
    /// The question "what does this key do" has exactly one truthful answer,
    /// and it is the one the editor would have acted on: the transient map
    /// before the mode's, the mode's before the global one, this buffer's
    /// mode and not another's. Anything that reconstructs that from the
    /// keymaps is a second implementation of the precedence rules, and the
    /// day the order changes it reports the old one -- confidently, with no
    /// test failing, in the command whose entire job is to be right about it.
    ///
    /// So capture does not intercept the keys ahead of resolution. It
    /// accumulates into the same pending sequence -- so a half-typed `C-x`
    /// shows in the echo area as it always does -- asks the same two
    /// questions of the same maps, and diverts only at the very end, in place
    /// of dispatch.
    ///
    /// WATCHER is called as (WATCHER KEYS TARGET SOURCE): KEYS as a binding is
    /// written, TARGET the form that would have run (quoted, so a caller sees
    /// what it is rather than what it does) or nil, and SOURCE the name of the
    /// map it came from, or nil when nothing is bound.
    fn capture_key_sequence(
        &self,
        event: KeyEvent,
        watcher: ELispExp<B>,
        env: &Arc<Env<EditorState<B>>>,
    ) {
        let mut pending = self
            .pending_keys
            .write()
            .expect("Failed to acquire write lock on pending_keys");
        pending.push(event);
        let keys = pending.clone();
        let described = describe_keys(&keys);

        // `C-u` never reaches a keymap: the argument reader takes it first,
        // which is exactly what is worth saying about it. Asked through the
        // reader's own predicate so the two cannot drift.
        let prefix_argument = keys.len() == 1 && Commands::<B>::is_prefix_argument_key(&keys[0]);

        let binding = if prefix_argument {
            None
        } else {
            self.key_binding(&keys)
        };
        if binding.is_none() && !prefix_argument && self.key_is_prefix(&keys) {
            // Part-way through a sequence: keep the keys and keep waiting,
            // the same as the editor would.
            return;
        }
        pending.clear();
        drop(pending);

        // Disarmed before the watcher runs, not after: a watcher that opens a
        // buffer, prompts, or fails part-way must not leave the next keystroke
        // captured too.
        env.set_variable(KEY_CAPTURE_FUNCTION.into(), ELispExp::nil());

        let source = match (&binding, prefix_argument) {
            (Some(binding), _) => ELispExp::string(binding.source.name().to_string()),
            (None, true) => ELispExp::string("prefix-argument".to_string()),
            (None, false) => ELispExp::nil(),
        };
        let target = match &binding {
            // Quoted: the target is a form like `(find-file)`, and what the
            // watcher wants is the form itself, not the file it would open.
            Some(binding) => ELispExp::form(vec![
                ELispExp::symbol("quote".into()),
                binding.target.clone(),
            ]),
            None => ELispExp::nil(),
        };
        self.run_command_form(
            ELispExp::form(vec![
                ELispExp::symbol("funcall".into()),
                watcher,
                ELispExp::string(described),
                target,
                source,
            ]),
            env,
        );
    }

    pub fn handle_key_event(&self, event: KeyEvent, env: &Arc<Env<EditorState<B>>>) {
        // Something is waiting to be told what the next key sequence is
        // rather than have it run. See `capture_key_sequence`.
        if let Some(watcher) = env.get_variable(KEY_CAPTURE_FUNCTION)
            && watcher.is_truthy()
        {
            self.capture_key_sequence(event, watcher, env);
            return;
        }

        // The argument reader gets first refusal. A key it takes is not a
        // command and never reaches a keymap.
        if self.read_prefix_argument(&event) {
            return;
        }

        // A key may be the whole of a binding, the start of a longer one, or
        // neither. Deciding which comes first, and two of the three answers
        // return before anything below runs -- see `resolve_key_sequence`.
        let Some(mut ast) = self.resolve_key_sequence(event.clone()) else {
            return;
        };

        if let ELispExp::Lambda(ref lambda) = ast {
            if lambda.params.len() != 0 {
                self.log_diagnostic(&format!("Keymap {event:?} associated to a lambda with some parameters. Associate it to a lambda with 0 parameters."));
                return;
            } else {
                ast = ELispExp::form(vec![ELispExp::symbol("funcall".into()), ast]);
            }
        }

        // A key bound to a bare command invocation -- `(next-line)`, as
        // `define-key` stores a symbol -- is routed through
        // `call-interactively` so that the editor collects whatever arguments
        // the command declared. Without this a command taking arguments simply
        // cannot be bound to a key: `find-file` needs a path, and a keystroke
        // has none to give it, which is why it had no binding at all.
        //
        // Routing every such binding through one Lisp entry point, rather than
        // only those that need arguments, is deliberate: it is the single
        // place to observe or advise command execution, which is what a macro
        // recorder or a `repeat` command would later hook.
        //
        // A binding that already supplies its arguments -- `(self-insert "a")`,
        // which is every ordinary keystroke -- is left exactly as it was, so
        // the typing path is untouched and still costs what it always did.
        // A binding may name its command either way: `define-key` wraps a
        // symbol into a one-element form, while the keymaps built in Rust
        // (`install_minibuffer`) store the bare symbol. Both mean "run this
        // command".
        //
        // The bare-symbol case was previously unreachable code: evaluating a
        // symbol looks up a *variable*, so Enter, Escape and Tab in the
        // minibuffer all failed with `UnboundVariable` in the real editor. No
        // test caught it because they all call `(minibuffer-confirm)` through
        // `eval` rather than pressing the key.
        self.run_command_form(ast, env);
    }

    /// Insert TEXT as a single bracketed paste.
    ///
    /// # Why paste is not typing
    ///
    /// Without bracketed paste a terminal delivers a paste as the keystrokes
    /// it looks like, and the editor cannot tell the difference: N characters
    /// become N `self-insert' commands, N undo entries, N runs of every hook.
    /// Pasting a function and then pressing `undo' would walk back through it
    /// one character at a time, and auto-pairing would double every bracket in
    /// it.
    ///
    /// The terminal knows the difference and says so, so this takes the whole
    /// paste as one command: one undo group, one syntax invalidation, one
    /// `post-command-hook', and no `post-self-insert-hook' at all.
    pub fn handle_paste(&self, text: String, env: &Arc<Env<EditorState<B>>>) {
        if text.is_empty() {
            return;
        }
        self.run_command_form(
            ELispExp::form(vec![
                ELispExp::symbol("insert-pasted-text".into()),
                ELispExp::string(text),
            ]),
            env,
        );
    }
}
