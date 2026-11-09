//! Keys and mouse events as the editor takes them in, how a key sequence is
//! written and read back, and keymaps: what a sequence is bound to, which
//! sequences lead to longer ones, and the transient maps consulted first. A
//! binding is a Lisp expression, `ELispExp`.
//!
//! Design: `doc/architettura-editor.typ, <tasti>`.
use crate::{ELispExp, buffer::BufferTrait};
use std::collections::{HashMap, HashSet};

/// A key, without its modifiers: a character, or one of the named keys.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// No key at all. Nothing in the editor or its front-ends makes one.
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
    /// The key that deletes forward, Delete or Del, as `<delete>` names it.
    Delete,
    Insert,
    Home,
    End,
    /// Page Up, which Emacs calls `<prior>`.
    PageUp,
    /// Page Down, which Emacs calls `<next>`.
    PageDown,
    /// A function key, F1 and on.
    F(u8),
}

/// The highest function key a binding can name: as far as a keyboard or a
/// terminal goes.
const LAST_FUNCTION_KEY: u8 = 35;

/// Which button, for the events that have one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseKind {
    Down(MouseButton),
    Up(MouseButton),
    Drag(MouseButton),
    ScrollUp,
    ScrollDown,
}

/// A mouse event, in cells of the frame: what a front-end hands to
/// `handle_mouse_event`.
///
/// # Examples
///
/// ```
/// use rsedit_core::{KeyModifiers, MouseButton, MouseEvent, MouseKind};
///
/// // A left click on the third cell of the first row.
/// let click = MouseEvent {
///     kind: MouseKind::Down(MouseButton::Left),
///     column: 2,
///     row: 0,
///     modifiers: KeyModifiers::default(),
/// };
/// assert_eq!(click.kind, MouseKind::Down(MouseButton::Left));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MouseEvent {
    pub kind: MouseKind,
    /// Zero-based cell coordinates within the frame.
    pub column: u16,
    pub row: u16,
    pub modifiers: KeyModifiers,
}

/// The modifiers held down with a key or a mouse event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// A key and its modifiers: what a front-end hands to `handle_key_event`.
///
/// # Examples
///
/// ```
/// use rsedit_core::{KeyCode, KeyEvent, KeyModifiers};
///
/// // C-x, as a front-end builds it.
/// let ctrl_x = KeyEvent {
///     code: KeyCode::Char('x'),
///     modifiers: KeyModifiers { ctrl: true, ..KeyModifiers::default() },
/// };
/// assert_ne!(ctrl_x, KeyEvent::new(KeyCode::Char('x')));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyEvent {
    /// Creates CODE, with no modifiers.
    pub fn new(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::default(),
        }
    }
}

/// Returns KEYS as a binding writes them and the echo area shows them:
/// `C-x C-f`. The inverse of `parse_key_sequence`, so a half-typed sequence
/// is spelt as the binding that completes it was written.
pub fn describe_keys(keys: &[KeyEvent]) -> String {
    keys.iter().map(describe_key).collect::<Vec<_>>().join(" ")
}

/// Returns KEY as a binding writes it.
fn describe_key(key: &KeyEvent) -> String {
    let mut out = String::new();
    if key.modifiers.ctrl {
        out.push_str("C-");
    }
    if key.modifiers.alt {
        out.push_str("M-");
    }
    // Only ever set on a key that is not a character: a frontend folds Shift
    // into the character itself, so `A` is never spelt `S-a`.
    if key.modifiers.shift {
        out.push_str("S-");
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
        KeyCode::Delete => "<delete>".to_string(),
        KeyCode::Insert => "<insert>".to_string(),
        KeyCode::Home => "<home>".to_string(),
        KeyCode::End => "<end>".to_string(),
        KeyCode::PageUp => "<prior>".to_string(),
        KeyCode::PageDown => "<next>".to_string(),
        KeyCode::F(n) => format!("<f{n}>"),
        KeyCode::None => "<none>".to_string(),
    });
    out
}

/// Returns the keys of a binding written as SEQ: one or more keys, separated
/// by spaces.
///
/// A part that does not parse fails the whole sequence: binding `C-x` when
/// `"C-x C-"` was written would define a prefix that swallows the real binding
/// underneath it.
pub(crate) fn parse_key_sequence(seq: &str) -> Option<Vec<KeyEvent>> {
    let parts: Vec<&str> = seq.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let keys: Vec<KeyEvent> = parts.iter().filter_map(|part| parse_key(part)).collect();
    if keys.len() != parts.len() {
        return None;
    }
    Some(keys)
}

/// Returns the key written as SEQ: any of the modifiers `C-`, `M-` and `S-`,
/// in any order, each once, then a key name.
///
/// `M-C-x` is `C-M-x`, as in Emacs. With a character, `S-` is its capital,
/// since that is how a shifted character arrives -- `C-S-a` is `C-A` -- and
/// with a character that has none it is no key; with a named key, `S-<tab>`,
/// it is Shift held.
pub(crate) fn parse_key(seq: &str) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers::default();
    let mut rest = seq;
    // A prefix only where something follows it: `C--` is Control and `-`.
    while rest.len() > 2 {
        let held = match &rest[..2] {
            "C-" => &mut modifiers.ctrl,
            "M-" => &mut modifiers.alt,
            "S-" => &mut modifiers.shift,
            _ => break,
        };
        if *held {
            // `C-C-x` says Control twice, which is a mistake, not a key.
            return None;
        }
        *held = true;
        rest = &rest[2..];
    }

    let key_code = match rest {
        "<ret>" | "<Return>" => KeyCode::Enter,
        "<esc>" | "<Escape>" => KeyCode::Esc,
        "tab" | "<tab>" | "<Tab>" => KeyCode::Tab,
        // What terminals call Shift-Tab, accepted under that name too.
        "<backtab>" => {
            modifiers.shift = true;
            KeyCode::Tab
        }
        "<backspace>" => KeyCode::Backspace,
        // Spelt out because a bare space is impossible to see in a key name,
        // and `C-<space>` is how `set-mark` is bound.
        "<space>" | " " => KeyCode::Char(' '),
        "<up>" => KeyCode::Up,
        "<down>" => KeyCode::Down,
        "<left>" => KeyCode::Left,
        "<right>" => KeyCode::Right,
        // Emacs' names first, and the names on the keys too.
        "<delete>" | "<deletechar>" => KeyCode::Delete,
        "<insert>" => KeyCode::Insert,
        "<home>" => KeyCode::Home,
        "<end>" => KeyCode::End,
        "<prior>" | "<pageup>" => KeyCode::PageUp,
        "<next>" | "<pagedown>" => KeyCode::PageDown,
        s if function_key(s).is_some() => KeyCode::F(function_key(s)?),
        s if s.chars().count() == 1 => {
            let c = s.chars().next()?;
            if modifiers.shift {
                // The capital, which is what arrives: no Shift left over.
                let mut upper = c.to_uppercase();
                let capital = upper.next().filter(|u| *u != c && upper.next().is_none())?;
                modifiers.shift = false;
                KeyCode::Char(capital)
            } else {
                KeyCode::Char(c)
            }
        }
        _ => return None,
    };

    Some(KeyEvent {
        code: key_code,
        modifiers,
    })
}

/// Returns the number of the function key NAME writes, `<f1>` to `<f35>`.
fn function_key(name: &str) -> Option<u8> {
    let digits = name.strip_prefix("<f")?.strip_suffix('>')?;
    // Digits only, and no leading zero: `<f01>` is not a name anybody writes,
    // and parsing would quietly take `+1` too.
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<u8>()
        .ok()
        .filter(|n| (1..=LAST_FUNCTION_KEY).contains(n))
}

/// Key sequences bound to Lisp expressions, the prefixes that lead to them,
/// and a fallback for plain characters.
///
/// The prefixes are kept rather than searched for, since every key asks
/// whether its sequence begins a binding; and one `insert` records both, so a
/// binding can never be left out of reach behind a prefix nobody registered.
#[derive(Clone, Debug)]
pub struct Keymap<B: BufferTrait> {
    bindings: HashMap<Vec<KeyEvent>, ELispExp<B>>,
    prefixes: HashSet<Vec<KeyEvent>>,
    character_handler: Option<ELispExp<B>>,
}

impl<B: BufferTrait> Default for Keymap<B> {
    fn default() -> Self {
        Self {
            bindings: HashMap::new(),
            prefixes: HashSet::new(),
            character_handler: None,
        }
    }
}

impl<B: BufferTrait> Keymap<B> {
    /// Creates an empty keymap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the fallback for unmodified characters, if this map has one.
    pub fn character_handler(&self) -> Option<&ELispExp<B>> {
        self.character_handler.as_ref()
    }

    /// Sets the fallback for unmodified characters, Unicode included, or
    /// removes it when HANDLER is nil. This map's bindings and prefixes come
    /// before it.
    pub fn set_character_handler(&mut self, handler: ELispExp<B>) {
        self.character_handler = (!handler.is_nil()).then_some(handler);
    }

    /// Returns what KEYS runs: its binding or, for a single unmodified
    /// character that begins no binding, the fallback called with it.
    pub fn binding(&self, keys: &[KeyEvent]) -> Option<ELispExp<B>> {
        if let Some(binding) = self.get(keys) {
            return Some(binding.clone());
        }
        let [event] = keys else {
            return None;
        };
        let KeyCode::Char(c) = event.code else {
            return None;
        };
        if event.modifiers.ctrl || event.modifiers.alt || self.is_prefix(keys) {
            return None;
        }
        let handler = self.character_handler.as_ref()?;
        Some(match handler {
            ELispExp::Symbol(name) => ELispExp::form(vec![
                ELispExp::symbol(name.to_string()),
                ELispExp::string(c.to_string()),
            ]),
            other => ELispExp::form(vec![
                ELispExp::symbol("funcall".into()),
                ELispExp::form(vec![ELispExp::symbol("quote".into()), other.clone()]),
                ELispExp::string(c.to_string()),
            ]),
        })
    }

    /// Binds KEYS to AST, and registers every proper prefix of KEYS: not the
    /// sequence itself, which a lookup finds with `get` first anyway.
    pub fn insert(&mut self, keys: Vec<KeyEvent>, ast: ELispExp<B>) {
        for len in 1..keys.len() {
            self.prefixes.insert(keys[..len].to_vec());
        }
        self.bindings.insert(keys, ast);
    }

    /// Binds a single key.
    pub fn insert_key(&mut self, key: KeyEvent, ast: ELispExp<B>) {
        self.insert(vec![key], ast);
    }

    /// Removes the binding of KEYS, and the prefixes only it needed. Returns
    /// whether there was one.
    pub fn remove(&mut self, keys: &[KeyEvent]) -> bool {
        if self.bindings.remove(keys).is_none() {
            return false;
        }
        // Built again from what is left: a prefix may be shared by other
        // bindings, and taking one away is rare.
        self.prefixes = self
            .bindings
            .keys()
            .flat_map(|keys| (1..keys.len()).map(move |len| keys[..len].to_vec()))
            .collect();
        true
    }

    /// Returns what KEYS is bound to, if it is a complete binding.
    pub fn get(&self, keys: &[KeyEvent]) -> Option<&ELispExp<B>> {
        self.bindings.get(keys)
    }

    /// Returns whether KEYS begins some longer binding, and so is worth
    /// waiting on rather than reporting as undefined.
    pub fn is_prefix(&self, keys: &[KeyEvent]) -> bool {
        self.prefixes.contains(keys)
    }

    /// Returns every binding in the map, in no particular order: whatever
    /// shows them sorts them.
    pub fn iter(&self) -> impl Iterator<Item = (&Vec<KeyEvent>, &ELispExp<B>)> {
        self.bindings.iter()
    }

    /// Returns how many bindings there are.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Returns whether there are none.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Fills KEYMAPS with the few bindings written in Rust: `M-x`, the arrows,
/// Return, and `self-insert` as the character handler, for every character
/// typed alone. Everything else comes from `common-keymaps.lisp` and the
/// modules.
pub fn fill_default_keymaps<B: BufferTrait>(keymaps: &mut Keymap<B>) {
    // -------------------------------- EDITOR ---------------------------------
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
        ELispExp::form(vec![ELispExp::symbol("newline".into())]),
    );

    // Every character typed alone -- `a`, `è`, `€` -- is inserted: the
    // character handler, as `(define-key nil "<char>" 'self-insert)` would
    // set it, and not a binding for each, since there are too many to bind
    // one by one. A binding of one of them, here or in a mode, still wins.
    // The call made, `(self-insert "x")`, carries its own argument; the
    // prefix argument is added to it when the command runs.
    keymaps.set_character_handler(ELispExp::symbol("self-insert".into()));
}

// ---------------------------------------------------------------------------
// Transient keymaps
// ---------------------------------------------------------------------------

/// What a transient keymap does with a key it does not bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnUnbound {
    /// Swallows it, and stays up until one of its own keys takes it down: a
    /// map that asks a question, which must bind an answer meaning "stop",
    /// `C-g` included.
    Refuse,

    /// Goes away, and lets the key through to the keymaps underneath: a map
    /// that offers something, as a bare `o` after `C-x o` does.
    Release,

    /// Lets the key through, and stays up: a map that filters, as the
    /// completion strip does, which is typed at while it narrows. It still has
    /// to bind a way out.
    Pass,
}

/// A keymap consulted before every other, for as long as it is installed: a
/// question ("press `y` or `n`") or an offer ("press `o` again"). Its answers
/// are ordinary commands, bound as everything else is.
///
/// An [`OnUnbound::Release`] map has no exit hook: it goes away inside key
/// resolution, with the sequence still locked, where the interpreter must not
/// be called. A map with a session behind it names it in `owner`, and the
/// session is ended once the key has been handled, when its map is no longer
/// the one up: `query-replace`'s question is such a map.
#[derive(Clone, Debug)]
pub struct TransientKeymap<B: BufferTrait> {
    pub keymap: Keymap<B>,
    pub on_unbound: OnUnbound,
    /// What to show while the map is live -- `[o]` for a repeat map, a question
    /// for a modal one. Empty to show nothing.
    pub message: String,
    /// What the map is the question of, when something stays open behind it
    /// and has to end with it; `None` for a map that leaves nothing behind.
    pub owner: Option<&'static str>,
}

impl<B: BufferTrait> TransientKeymap<B> {
    /// Returns a map offering KEY, which runs COMMAND and offers it again: not
    /// rebuilt by the command, but reinstalled after every command that has a
    /// repeat key.
    pub fn repeating(key: KeyEvent, command: &str) -> Self {
        let mut keymap = Keymap::new();
        keymap.insert_key(key.clone(), ELispExp::symbol(command.into()));
        Self {
            keymap,
            on_unbound: OnUnbound::Release,
            message: format!("[{}]", describe_keys(&[key])),
            owner: None,
        }
    }
}
