use crate::{ELispExp, buffer::BufferTrait};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    None,
    Char(char),
    Backspace,
    Enter,
    Esc,
    Tab,
    Left,
    Right,
    Up,
    Down,
    // Add more as needed (Tab, F1...)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub caps_lock_as_ctrl: bool, // Placeholder for future GUI frontends!
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyEvent {
    pub fn new(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::default(),
        }
    }
}

/// How a key sequence is written in a binding and shown to the user.
///
/// The inverse of the parser in `primitives::parse_key_sequence`, so that what
/// the echo area shows while a sequence is half-typed is spelt the same way
/// the binding that will complete it was written.
pub fn describe_keys(keys: &[KeyEvent]) -> String {
    keys.iter().map(describe_key).collect::<Vec<_>>().join(" ")
}

fn describe_key(key: &KeyEvent) -> String {
    let mut out = String::new();
    if key.modifiers.ctrl {
        out.push_str("C-");
    }
    if key.modifiers.alt {
        out.push_str("M-");
    }
    out.push_str(&match key.code {
        KeyCode::Char(' ') => "<space>".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Backspace => "<backspace>".to_string(),
        KeyCode::Enter => "<ret>".to_string(),
        KeyCode::Esc => "<esc>".to_string(),
        KeyCode::Tab => "<tab>".to_string(),
        KeyCode::Left => "<left>".to_string(),
        KeyCode::Right => "<right>".to_string(),
        KeyCode::Up => "<up>".to_string(),
        KeyCode::Down => "<down>".to_string(),
        KeyCode::None => "<none>".to_string(),
    });
    out
}

/// Key sequences bound to expressions, and the prefixes that lead to them.
///
/// # Why the prefixes are stored rather than searched for
///
/// Every keystroke asks two questions of a keymap: "is this sequence a
/// binding?" and "is it the start of one?". The first is a hash lookup. The
/// second, done honestly, is a scan of every binding in the map -- on the path
/// between a key being pressed and a character appearing, several hundred
/// times over for the self-insert bindings alone.
///
/// So the answer is maintained instead of computed. The set changes only when
/// a binding is defined, which happens at startup and when a user edits their
/// configuration; the question is asked on every key.
///
/// # Why this is a type and not two fields
///
/// Inserting a binding without registering its prefixes leaves a `C-x C-f`
/// that can never be reached, because `C-x` alone would be reported undefined
/// and the sequence thrown away. Putting both behind one `insert` makes that
/// state unrepresentable rather than merely discouraged.
#[derive(Clone, Debug)]
pub struct Keymap<B: BufferTrait> {
    bindings: HashMap<Vec<KeyEvent>, ELispExp<B>>,
    prefixes: HashSet<Vec<KeyEvent>>,
}

impl<B: BufferTrait> Default for Keymap<B> {
    fn default() -> Self {
        Self {
            bindings: HashMap::new(),
            prefixes: HashSet::new(),
        }
    }
}

impl<B: BufferTrait> Keymap<B> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind KEYS to AST, and register every proper prefix of KEYS.
    ///
    /// *Proper*: the sequence itself is not registered. Nothing observable
    /// depends on that -- a lookup checks `get` before `is_prefix`, so a
    /// complete binding is found as a binding either way -- but leaving it out
    /// keeps roughly a hundred single-key bindings from each taking a second
    /// slot in the set for no purpose.
    pub fn insert(&mut self, keys: Vec<KeyEvent>, ast: ELispExp<B>) {
        for len in 1..keys.len() {
            self.prefixes.insert(keys[..len].to_vec());
        }
        self.bindings.insert(keys, ast);
    }

    /// Bind a single key. Most bindings are one key long, and writing every
    /// one of them as a one-element vector would obscure the few that are not.
    pub fn insert_key(&mut self, key: KeyEvent, ast: ELispExp<B>) {
        self.insert(vec![key], ast);
    }

    /// What KEYS is bound to, if it is a complete binding.
    pub fn get(&self, keys: &[KeyEvent]) -> Option<&ELispExp<B>> {
        self.bindings.get(keys)
    }

    /// Whether KEYS is the beginning of some longer binding, and so worth
    /// waiting on rather than reporting as undefined.
    pub fn is_prefix(&self, keys: &[KeyEvent]) -> bool {
        self.prefixes.contains(keys)
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Fills a keymaps map with all the ascii char self-insert char so the editor
/// can handle the typing of letters, digits and most of symbols.
pub fn fill_default_keymaps<B: BufferTrait>(keymaps: &mut Keymap<B>) {
    // -------------------------------- EDITOR ---------------------------------
    keymaps.insert_key(
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        },
        ELispExp::form(vec![ELispExp::symbol("quit".into())]),
    );

    // M-x is bound here rather than in a `.lisp` file because it is the only
    // way to reach a command by name: without it the command system exists but
    // is unreachable, which is not something a configuration file should be
    // able to take away.
    keymaps.insert_key(
        KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers {
                alt: true,
                ..Default::default()
            },
        },
        ELispExp::symbol("command-execute-prompt".into()),
    );

    // -------------------------------- BUFFER ---------------------------------
    keymaps.insert_key(
        KeyEvent {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        },
        ELispExp::form(vec![ELispExp::symbol("save-buffer".into())]),
    );

    keymaps.insert_key(
        KeyEvent {
            code: KeyCode::Left,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::form(vec![ELispExp::symbol("backward-char".into())]),
    );
    keymaps.insert_key(
        KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::form(vec![ELispExp::symbol("forward-char".into())]),
    );
    keymaps.insert_key(
        KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::form(vec![ELispExp::symbol("previous-line".into())]),
    );
    keymaps.insert_key(
        KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::form(vec![ELispExp::symbol("next-line".into())]),
    );
    keymaps.insert_key(
        KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::form(vec![ELispExp::symbol("insert-newline".into())]),
    );

    for c in ' '..='~' {
        let event = KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::default(), // No modifiers (Ctrl/Alt off)
        };
        keymaps.insert_key(
            event,
            ELispExp::form(vec![
                ELispExp::symbol("self-insert".into()),
                ELispExp::string(c.into()),
            ]),
        );
    }
}

// ---------------------------------------------------------------------------
// Keymaps that last for a moment
// ---------------------------------------------------------------------------

/// What a transient keymap does with a key it does not bind.
///
/// The two answers are genuinely different features wearing the same
/// mechanism, and getting them the wrong way round is maddening in both
/// directions -- so the choice is made once, where the map is installed, rather
/// than guessed at per key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnUnbound {
    /// Swallow it. The map stays up until one of its own keys takes it down.
    ///
    /// For a map that is a *question*: a half-finished operation must not be
    /// walked away from by pressing something unrelated, because nothing would
    /// then say it was still running. Every map of this kind has to bind an
    /// answer meaning "stop" -- `C-g` included, since a modal map takes that
    /// too.
    Refuse,
    /// Dismiss the map, and let the key through to the keymaps underneath as
    /// though the map had never been there.
    ///
    /// For a map that is an *offer*: after `C-x o`, a bare `o` moves to the
    /// next window, and anything else means the user is done cycling and wants
    /// that other thing to happen. Swallowing it would make the convenience
    /// cost more than it saves.
    Release,
}

/// A keymap consulted before every other, for as long as it is installed.
///
/// # Why one mechanism for two things
///
/// "Press `y` or `n`" and "press `o` again to keep going" look like different
/// features, and underneath they are the same one: a keymap that is consulted
/// first and then goes away. What differs is only [`OnUnbound`] -- whether a
/// key the map does not bind is refused or handed on.
///
/// Being a *keymap* rather than a bespoke read loop is what makes the answers
/// ordinary commands: they are bound the way everything else is bound, they
/// appear in `describe-key`-style listings, and each is its own command for
/// undo grouping. A bespoke loop would have needed all of that reinvented.
///
/// # No exit hook, deliberately
///
/// A [`OnUnbound::Release`] map is dismissed by a key that was *not* pressed
/// for it, and nothing of the map's own runs at that moment. There is therefore
/// nowhere safe to hang cleanup: the dismissal happens inside key resolution,
/// under the keymap locks, and calling into the interpreter there is the
/// deadlock the editor warns about everywhere else.
///
/// So a `Release` map must carry no state that needs cleaning up -- which
/// repeat maps do not. Anything with a session behind it uses `Refuse` and ends
/// through its own bindings, where a command can clean up properly.
#[derive(Clone, Debug)]
pub struct TransientKeymap<B: BufferTrait> {
    pub keymap: Keymap<B>,
    pub on_unbound: OnUnbound,
    /// What to show while the map is live -- `[o]` for a repeat map, a question
    /// for a modal one. Empty to show nothing.
    pub message: String,
}

impl<B: BufferTrait> TransientKeymap<B> {
    /// A map offering one key, which runs COMMAND and offers it again.
    ///
    /// The map is *not* rebuilt by the command it runs: it is reinstalled after
    /// every command that has a repeat key, so pressing `o` re-runs
    /// `other-window` and re-offers `o` by the same path that offered it first.
    /// One rule, applied once, rather than a command that has to remember to
    /// keep itself alive.
    pub fn repeating(key: KeyEvent, command: &str) -> Self {
        let mut keymap = Keymap::new();
        keymap.insert_key(key.clone(), ELispExp::symbol(command.into()));
        Self {
            keymap,
            on_unbound: OnUnbound::Release,
            message: format!("[{}]", describe_keys(&[key])),
        }
    }
}
