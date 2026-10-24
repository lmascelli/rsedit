//! From a key to a command: the sequence being typed, the keymaps that resolve
//! it, the prefix argument's reader, the help key, and a sequence captured
//! instead of run.
//!
//! Design: `doc/architettura-editor.typ, <tasto>`.

use super::*;

/// The variable naming what is waiting to be told the next key sequence, and
/// nil when nothing is.
///
/// Setting it is what arms a capture: one fact in one place, which Lisp reads
/// and sets like any other variable, for one lookup per key.
pub const KEY_CAPTURE_FUNCTION: &str = "*key-capture-function*";

/// The variable naming the key that opens help, and that, half-way through a
/// sequence, asks what the prefix so far goes on with. Read only once a
/// sequence has turned out to be bound to nothing.
pub const HELP_KEY: &str = "help-key";

/// What `help-key` is when nothing says otherwise.
const DEFAULT_HELP_KEY: &str = "C-h";

/// The function asked to describe a prefix. Without a help module it is not
/// there, and a prefix followed by the help key is undefined like any other
/// sequence: the feature missing, not broken.
const DESCRIBE_PREFIX: &str = "describe-prefix-keys";

/// What the keymaps say of the sequence typed so far.
enum Bound<B: BufferTrait> {
    /// Run this.
    Command(ELispExp<B>),
    /// Part of a longer sequence: the keys are kept.
    Prefix,
    /// Bound to nothing, and nothing is to be said: a transient map's
    /// `Refuse`.
    Refused,
    /// Bound to nothing: say so.
    Unbound,
}

/// What to do once a sequence has resolved.
enum Dispatch<B: BufferTrait> {
    /// Run this.
    Run(ELispExp<B>),
    /// Nothing to do, and nothing to say.
    Nothing,
    /// Bound to nothing. Whether that is a question or a mistake is the
    /// caller's to tell, since it needs the environment.
    Unbound {
        /// The whole sequence, written as a binding is.
        described: String,
        /// The sequence without its last key, when that much is a live prefix:
        /// `C-x` and then the help key asks about `C-x`. `None` when the
        /// sequence was going nowhere from its start.
        prefix: Option<String>,
        /// The key that ended it.
        last: KeyEvent,
    },
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns the keymaps of the buffer the keys are typed into, the ones
    /// after the transient keymap: the buffer's local keymap, when
    /// `set-local-keymap` gave it one, then its major mode's.
    ///
    /// The buffer is read, and let go, before the modes are taken: buffers
    /// come before modes.
    fn keymap_modes(&self) -> Vec<String> {
        self.with_current_buffer(|buf| {
            let mut names = Vec::with_capacity(2);
            if let Some(ELispExp::Symbol(name) | ELispExp::String(name)) =
                buf.data::<ELispExp<B>>("lisp:local-keymap")
                && name.as_str() != "nil"
            {
                names.push(name.to_string());
            }
            names.push(buf.current_mode.clone());
            names
        })
    }

    /// Returns whether the buffer's local keymap binds EVENT itself, and so
    /// claims it from the prefix argument's reader.
    ///
    /// Asked below the transient keymap, where the local keymap is: a map that
    /// refuses the key does not stop the buffer from claiming it from the
    /// reader, and then refuses it itself.
    fn local_key_claims(&self, event: &KeyEvent) -> bool {
        let names = self.keymap_modes();
        let modes: Vec<&str> = names.iter().map(String::as_str).collect();
        self.modes(|registry| registry.binding_below_transient(&modes, std::slice::from_ref(event)))
            .is_some_and(|binding| binding.source == crate::managers::BindingSource::Local)
    }

    /// Returns what KEYS would run if typed now, and the keymap it comes from.
    ///
    /// The walk the editor itself takes -- see [`Modes::binding_for`] -- so
    /// that what this reports and what would happen agree. `None` when nothing
    /// would run, a sequence that only starts a longer one included.
    pub(crate) fn key_binding(&self, keys: &[KeyEvent]) -> Option<Binding<B>> {
        let names = self.keymap_modes();
        let modes: Vec<&str> = names.iter().map(String::as_str).collect();
        self.modes(|registry| registry.binding_for(&modes, keys))
    }

    /// Returns whether KEYS start a longer binding.
    fn key_is_prefix(&self, keys: &[KeyEvent]) -> bool {
        let names = self.keymap_modes();
        let modes: Vec<&str> = names.iter().map(String::as_str).collect();
        self.modes(|registry| registry.is_prefix(&modes, keys))
    }

    /// Returns every binding in force in the current buffer, sorted, each
    /// sequence once.
    pub(crate) fn key_bindings(&self) -> Vec<Binding<B>> {
        let names = self.keymap_modes();
        let modes: Vec<&str> = names.iter().map(String::as_str).collect();
        self.modes(|registry| registry.bindings(&modes))
    }

    /// Offers COMMAND's repeat key, if it has one, as a `Release` keymap that
    /// binds the key to COMMAND.
    ///
    /// Run after every command, so that a command reached by its repeat key
    /// offers it again.
    pub(super) fn install_repeat_keymap(&self, command: Option<&str>) {
        let Some(key) = command.and_then(|name| self.repeat_key(name)) else {
            // Nothing to offer, and nothing to take down: a `Release` map has
            // gone already, when key resolution handed the key on, which is
            // what ends a run of `C-x o o o`. Taking a map down here would be
            // wrong: a `Refuse` map, a question, is answered by its own
            // commands, and this runs after each of them.
            return;
        };
        self.set_transient_keymap(TransientKeymap::repeating(
            key,
            command.expect("a repeat key was found, so there is a command"),
        ));
    }

    /// Abandons what is half-typed: a key sequence, a prefix argument.
    ///
    /// What `keyboard-quit` does to the keys. A command already running is not
    /// stopped by it: keys are read on the thread that runs commands, between
    /// one command and the next.
    pub(crate) fn abandon_pending_input(&self) {
        self.pending_keys
            .write()
            .expect("Failed to acquire write lock on pending_keys")
            .clear();
        self.clear_prefix_argument();
    }

    /// Returns what the editor is part-way through reading, written as a
    /// binding is and ending in a dash for the key to come: `C-u 4 C-x-`.
    ///
    /// The argument and the sequence together, since they compose: one without
    /// the other would misreport what the next key does.
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

    /// Adds EVENT to the sequence being typed, and returns what to do: run a
    /// command; nothing, when more keys are to come or the key was refused;
    /// or report a sequence bound to nothing. The sequence is cleared unless
    /// it is a prefix.
    ///
    /// It returns rather than dispatching: a key that only lengthens a
    /// sequence is not a command, and must not reach the undo group, the flags
    /// rolled over or `last-command`, which are all past the point where this
    /// returns.
    fn resolve_key_sequence(&self, event: KeyEvent) -> Dispatch<B> {
        let mut pending = self
            .pending_keys
            .write()
            .expect("Failed to acquire write lock on pending_keys");
        pending.push(event);

        // Read before the modes are taken: buffers come before modes.
        let names = self.keymap_modes();
        let current_modes: Vec<&str> = names.iter().map(String::as_ref).collect();

        // One acquisition answers the whole question: the transient keymap,
        // the buffer's, and the global one.
        let mut release_transient = false;
        let bound = self.modes(|modes| {
            // A transient keymap comes before every other, for as long as it is
            // installed. See `TransientKeymap`.
            if let Some(map) = modes.transient() {
                let hit = map.keymap.get(&pending).cloned();
                let prefix = map.keymap.is_prefix(&pending);
                match (hit, prefix, map.on_unbound) {
                    (Some(ast), _, _) => return Bound::Command(ast),
                    // Part-way through one of the map's own sequences.
                    (None, true, _) => return Bound::Prefix,
                    // Refused, and nothing said: the map's message is in the
                    // frame, and anything written to the echo area would be
                    // drawn under it.
                    (None, false, OnUnbound::Refuse) => return Bound::Refused,
                    // Handed on, and the map stays: the key goes on to the
                    // keymaps below, and the map is asked again next time --
                    // how the completion strip is typed at without its
                    // swallowing the letter or going away.
                    (None, false, OnUnbound::Pass) => {}
                    // Handed on, and the map goes: the key goes on as though
                    // the map had never been there, which makes the offer free
                    // to ignore. Taken down once this lock is given back, since
                    // that needs the write side.
                    (None, false, OnUnbound::Release) => release_transient = true,
                }
            }

            let hit = modes.binding_below_transient(&current_modes, &pending);
            let prefix = modes.is_prefix_below_transient(&current_modes, &pending);
            match (hit, prefix) {
                (Some(binding), _) => Bound::Command(binding.target),
                (None, true) => Bound::Prefix,
                (None, false) => Bound::Unbound,
            }
        });
        if release_transient {
            // The bindings of a map nobody can reach would stay until the next
            // map came.
            self.clear_transient_keymap();
        }

        // What the keymaps had to say is said, and their lock given back:
        // reporting writes the echo area, under none of it.
        let described = describe_keys(&pending);
        // The sequence without its last key, when that much leads somewhere:
        // worked out now, while the keys are at hand, so that `C-x` and then
        // the help key is a question about `C-x`.
        let reaching_past = match (&bound, pending.len()) {
            (Bound::Unbound, len) if len >= 2 => {
                let prefix = pending[..len - 1].to_vec();
                let names = self.keymap_modes();
                let current_modes: Vec<&str> = names.iter().map(String::as_str).collect();
                self.modes(|modes| modes.is_prefix(&current_modes, &prefix))
                    .then(|| describe_keys(&prefix))
            }
            _ => None,
        };
        let last = pending.last().cloned();
        if !matches!(bound, Bound::Prefix) {
            pending.clear();
        }
        drop(pending);

        match bound {
            Bound::Command(ast) => Dispatch::Run(ast),
            // Nothing to say: the sequence is in `pending_input`, which the
            // frame carries, and which does not expire as a message does.
            Bound::Prefix | Bound::Refused => Dispatch::Nothing,
            Bound::Unbound => match last {
                Some(last) => Dispatch::Unbound {
                    described,
                    prefix: reaching_past,
                    last,
                },
                None => Dispatch::Nothing,
            },
        }
    }

    /// Returns whether KEY is the last key of `help-key`: a help key of two
    /// keys -- `C-c h`, for a terminal that sends Backspace as `C-h` -- still
    /// has one key to press half-way through a sequence.
    ///
    /// Asked only of a sequence already bound to nothing, so the variable is
    /// read on a mistyped key and never on a typed one.
    fn is_help_key(&self, key: &KeyEvent, env: &Arc<Env<EditorState<B>>>) -> bool {
        let spelling = match env.get_variable(HELP_KEY) {
            Some(ELispExp::String(text)) => text.to_string(),
            _ => DEFAULT_HELP_KEY.to_string(),
        };
        crate::input::parse_key_sequence(&spelling)
            .and_then(|keys| keys.last().cloned())
            .is_some_and(|help| &help == key)
    }

    /// Reads the next key sequence and hands it to WATCHER instead of running
    /// it, as (WATCHER KEYS TARGET SOURCE): KEYS written as a binding is,
    /// TARGET the form that would have run, quoted, or nil, and SOURCE the
    /// name of the keymap it came from, or nil when nothing is bound.
    ///
    /// The sequence goes down the real resolution -- the same pending keys, so
    /// that a half-typed `C-x` shows as always, and the same questions of the
    /// same keymaps -- and turns aside only where it would be run: what a key
    /// does has one true answer, the one the editor would act on.
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

        // `C-u` reaches no keymap, since the argument's reader takes it first,
        // and that is what is worth reporting. Asked with the reader's own
        // predicate, so that the two agree.
        let prefix_argument = keys.len() == 1
            && Commands::<B>::is_prefix_argument_key(&keys[0])
            && !self.local_key_claims(&keys[0]);

        let binding = if prefix_argument {
            None
        } else {
            self.key_binding(&keys)
        };
        if binding.is_none() && !prefix_argument && self.key_is_prefix(&keys) {
            // Part-way through a sequence: the keys are kept, and the next one
            // awaited, as the editor would.
            return;
        }
        pending.clear();
        drop(pending);

        // Disarmed before the watcher runs: a watcher that opens a buffer,
        // prompts or fails must not leave the next key captured too.
        env.set_root_variable(KEY_CAPTURE_FUNCTION.into(), ELispExp::nil());

        let source = match (&binding, prefix_argument) {
            (Some(binding), _) => ELispExp::string(binding.source.name().to_string()),
            (None, true) => ELispExp::string("prefix-argument".to_string()),
            (None, false) => ELispExp::nil(),
        };
        let target = match &binding {
            // Quoted, and as data: the target is a form like `(find-file)`, the
            // watcher wants the form and not the file it would open, and a
            // keymap holds syntax, which the Lisp given it could not take
            // apart. See `binding_form`.
            Some(binding) => ELispExp::form(vec![
                ELispExp::symbol("quote".into()),
                risp::form_to_data(&binding.target),
            ]),
            None => ELispExp::nil(),
        };
        self.run_command_form(
            ELispExp::form(vec![
                ELispExp::symbol("funcall".into()),
                // Quoted too: a watcher named by a symbol --
                // `(read-key-sequence 'describe-this)` -- would otherwise be
                // evaluated as a variable, and fail as unbound. A lambda
                // passed by value works either way.
                ELispExp::form(vec![ELispExp::symbol("quote".into()), watcher]),
                ELispExp::string(described),
                target,
                source,
            ]),
            env,
        );
    }

    /// Handles a key pressed: the host calls it for each key event, in order,
    /// on the thread that runs the commands.
    ///
    /// The key is offered to a capture, if one is armed, then to the prefix
    /// argument's reader, then to the keymaps; what it completes runs as a
    /// command. A keyboard macro being recorded gets the key wherever it is
    /// used up, even when it runs nothing.
    pub fn handle_key_event(&self, event: KeyEvent, env: &Arc<Env<EditorState<B>>>) {
        // Something is waiting to be told the next sequence instead of having
        // it run. See `capture_key_sequence`.
        if let Some(watcher) = env.get_variable(KEY_CAPTURE_FUNCTION)
            && watcher.is_truthy()
        {
            // Recorded though it runs no command, which is why a macro records
            // keys and not commands: `zap-to-char` reads its character this
            // way, and a recorder of commands would replay a wait for a
            // character that never came.
            self.record_keys(std::slice::from_ref(&event), None);
            self.capture_key_sequence(event, watcher, env);
            return;
        }

        // The prefix argument's reader comes next, unless the buffer's own
        // keymap claims the key: a key the reader takes is not a command, and
        // reaches no keymap.
        if !(Commands::<B>::is_prefix_argument_key(&event) && self.local_key_claims(&event))
            && self.read_prefix_argument(&event)
        {
            // Recorded too: without it a macro would replay `C-u 5 C-n` as a
            // bare `C-n`.
            self.record_keys(std::slice::from_ref(&event), None);
            return;
        }

        // The keys of the sequence this one completes, read before resolving
        // it, which uses them up: a recorder wants the sequence whole, so that
        // `C-x )` is one thing to leave out rather than two.
        let mut sequence = self.pending_key_events();
        sequence.push(event.clone());

        // A key may be a whole binding, the start of a longer one, or neither:
        // see `resolve_key_sequence`.
        let mut ast = match self.resolve_key_sequence(event.clone()) {
            Dispatch::Run(ast) => {
                self.record_keys(&sequence, command_named_by(&ast).as_deref());
                ast
            }
            // Part of a sequence: its keys are pending, and arrive with the one
            // that completes it.
            Dispatch::Nothing => return,
            Dispatch::Unbound {
                described,
                prefix,
                last,
            } => {
                // A prefix and then the help key is a question -- what does
                // `C-x` go on with? -- which a command of the help module
                // answers. Without one the sequence is undefined like any
                // other: the feature absent, not broken.
                if let Some(prefix) = prefix
                    && self.is_help_key(&last, env)
                    && env.get_function(DESCRIBE_PREFIX).is_some()
                {
                    self.run_command_form(
                        ELispExp::form(vec![
                            ELispExp::symbol(DESCRIBE_PREFIX.into()),
                            ELispExp::string(prefix),
                        ]),
                        env,
                    );
                } else {
                    // Recorded though it does nothing: a macro does what was
                    // pressed, and a replay that left the mistake out would
                    // differ from the recording without saying so.
                    self.record_keys(&sequence, None);
                    self.set_echo_message(&format!("{described} is undefined"));
                    self.log_diagnostic(&format!("[INFO] Keymap not bound {described}"));
                }
                return;
            }
        };

        // A key bound to a lambda calls it, with no arguments, since a key has
        // none to give.
        if let ELispExp::Lambda(ref lambda) = ast {
            if lambda.params.len() != 0 {
                self.log_diagnostic(&format!("Keymap {event:?} associated to a lambda with some parameters. Associate it to a lambda with 0 parameters."));
                return;
            } else {
                ast = ELispExp::form(vec![ELispExp::symbol("funcall".into()), ast]);
            }
        }

        self.run_command_form(ast, env);
    }

    /// Inserts TEXT, a bracketed paste, as one command: one undo group, one
    /// `post-command-hook`, and no `post-self-insert-hook`.
    ///
    /// The terminal says a paste is a paste, and typed key by key it would be
    /// undone a character at a time, and paired bracket by bracket.
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

    /// Returns the keys of the sequence being typed. Cloned out: the one
    /// caller is on the key path, and takes the same lock again to resolve the
    /// sequence.
    fn pending_key_events(&self) -> Vec<KeyEvent> {
        self.pending_keys
            .read()
            .expect("read lock on pending_keys")
            .clone()
    }
}

/// Returns the command a key's binding names, if it names one: a bare symbol,
/// or a form whose head is one. The shapes `run_command_form` reads for the
/// command it runs; here they decide whether a recorder keeps the keys.
fn command_named_by<B: BufferTrait>(ast: &ELispExp<B>) -> Option<String> {
    match ast {
        ELispExp::Symbol(name) => Some(name.to_string()),
        ELispExp::Form(items) => items.first().and_then(|head| match head {
            ELispExp::Symbol(name) => Some(name.to_string()),
            _ => None,
        }),
        _ => None,
    }
}
