//! Reading a password: `read-passwd-mode`, the mode of a prompt whose answer
//! is never in its buffer; the answer itself, a [`Secret`] kept beside the
//! buffer where nothing else in the editor looks; and the cache that keeps a
//! password for a while, by key, as Emacs' `password-cache` does. The
//! primitives are in `primitives/password.rs`.
//!
//! What is typed at such a prompt goes into the secret, and the buffer only
//! shows it: a `read-hide-char` for each character, nothing, or the password
//! itself while it is shown. The buffer is read-only and its text is put
//! there without an undo record, so that no command of the editor's own --
//! a kill, a copy, an undo, a recorded macro, the history -- can take the
//! password from it.
//!
//! Design: `doc/architettura-editor.typ, <password>`.
use crate::{
    BufferTrait, ELispExp, EditorState,
    buffer::Buffer,
    feature::minibuffer::PROMPT_BUFFER,
    input::{KeyCode, KeyEvent, KeyModifiers},
    modes::MajorMode,
};
use risp::Env;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The major mode of a prompt that reads a password.
pub(crate) const PASSWORD_MODE: &str = "read-passwd-mode";

/// The key the open password prompt's [`PasswordPrompt`] is kept under, in
/// the data of the prompt's buffer: it goes when the buffer goes.
pub(crate) const PROMPT_DATA: &str = "read-passwd";

/// The key of a buffer's local keymap in its data, as `set-local-keymap`
/// writes it.
const LOCAL_KEYMAP: &str = "lisp:local-keymap";

/// The name of the Lisp variable holding what is drawn for each character of
/// a password while it is hidden.
pub(crate) const READ_HIDE_CHAR: &str = "read-hide-char";

/// What is drawn for each character of a hidden password when nothing says
/// otherwise.
pub(crate) const DEFAULT_HIDE_CHAR: char = '*';

/// The name of the Lisp variable that turns the password cache on and off.
pub(crate) const PASSWORD_CACHE: &str = "password-cache";

/// The name of the Lisp variable holding how long a password stays in the
/// cache, in seconds.
pub(crate) const PASSWORD_CACHE_EXPIRY: &str = "password-cache-expiry";

/// How many seconds a password stays in the cache when nothing says
/// otherwise: Emacs' 16.
pub(crate) const DEFAULT_CACHE_EXPIRY: f64 = 16.0;

/// The question a password is asked again with, by `read-passwd` given
/// CONFIRM.
pub(crate) const CONFIRM_PROMPT: &str = "Confirm password: ";

/// What is said when the second answer is not the first.
pub(crate) const NOT_REPEATED: &str = "Password not repeated accurately; please start over";

// ---------------------------------------------------------------------------
// The secret
// ---------------------------------------------------------------------------

/// How many characters a [`Secret`] has room for before it first grows: more
/// than most passwords, so that most never move.
const INITIAL_ROOM: usize = 64;

/// Text that must not outlive its use: characters kept where nothing else in
/// the editor looks, and overwritten when they go.
///
/// Every character it ever held is overwritten with NUL -- when it is
/// deleted, when the storage grows and the old storage is given back, and
/// when the secret is dropped -- with volatile writes, which the compiler may
/// not leave out. What leaves it is the caller's: [`Secret::reveal`] makes an
/// ordinary string.
pub(crate) struct Secret {
    chars: Vec<char>,
}

impl Default for Secret {
    fn default() -> Self {
        Self::new()
    }
}

impl Secret {
    /// Returns an empty secret.
    pub(crate) fn new() -> Self {
        Self {
            chars: Vec::with_capacity(INITIAL_ROOM),
        }
    }

    /// Returns a secret holding TEXT.
    pub(crate) fn from_text(text: &str) -> Self {
        let mut secret = Self::new();
        secret.insert(0, text);
        secret
    }

    /// Returns how many characters it holds.
    pub(crate) fn len(&self) -> usize {
        self.chars.len()
    }

    /// Returns whether it holds nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Inserts TEXT at character AT, clamped to the end, and returns the
    /// offset after it.
    pub(crate) fn insert(&mut self, at: usize, text: &str) -> usize {
        let at = at.min(self.len());
        let count = text.chars().count();
        self.make_room(count);
        // Room is made first, so no insertion moves the storage and leaves
        // a copy behind.
        for (offset, ch) in text.chars().enumerate() {
            self.chars.insert(at + offset, ch);
        }
        at + count
    }

    /// Deletes the characters from FROM up to TO, in either order and
    /// clamped to the end, and overwrites the room they leave.
    pub(crate) fn delete(&mut self, from: usize, to: usize) {
        let end = from.max(to).min(self.len());
        let start = from.min(to).min(end);
        if start == end {
            return;
        }
        // The characters after the range move down over it, and their old
        // places are past the end now: overwritten there.
        self.chars.drain(start..end);
        wipe_spare(&mut self.chars);
    }

    /// Deletes everything, and overwrites where it was.
    pub(crate) fn clear(&mut self) {
        wipe_all(&mut self.chars);
    }

    /// Returns the text, as an ordinary string: what is done with it, and
    /// whether it is overwritten afterwards, is the caller's.
    pub(crate) fn reveal(&self) -> String {
        // Exactly as long as it will be, so that it never grows and leaves a
        // copy behind.
        let mut text = String::with_capacity(self.chars.iter().map(|c| c.len_utf8()).sum());
        text.extend(self.chars.iter());
        text
    }

    /// Returns the text in UTF-8, followed by SUFFIX, as bytes to write to a
    /// process.
    pub(crate) fn bytes_with(&self, suffix: &str) -> Vec<u8> {
        let length: usize = self.chars.iter().map(|c| c.len_utf8()).sum::<usize>() + suffix.len();
        let mut bytes = Vec::with_capacity(length);
        let mut encoded = [0u8; 4];
        for ch in &self.chars {
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
        wipe_bytes(&mut encoded);
        bytes.extend_from_slice(suffix.as_bytes());
        bytes
    }

    /// Returns a secret of its own with the same text.
    pub(crate) fn duplicate(&self) -> Self {
        let mut copy = Self::new();
        copy.make_room(self.len());
        copy.chars.extend_from_slice(&self.chars);
        copy
    }

    /// Makes room for EXTRA more characters. When the storage has to grow,
    /// the characters move to storage of their own, and the old storage is
    /// overwritten before it is given back.
    fn make_room(&mut self, extra: usize) {
        let wanted = self.len() + extra;
        if wanted <= self.chars.capacity() {
            return;
        }
        let mut grown = Vec::with_capacity(wanted.next_power_of_two().max(INITIAL_ROOM));
        grown.extend_from_slice(&self.chars);
        let mut old = std::mem::replace(&mut self.chars, grown);
        wipe_all(&mut old);
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.chars == other.chars
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe_all(&mut self.chars);
    }
}

/// Never the text: a secret printed in a log or an error says how long it
/// is and nothing else.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret({} characters)", self.len())
    }
}

/// Overwrites every character CHARS has room for, those it holds and those
/// past its end, and empties it.
fn wipe_all(chars: &mut Vec<char>) {
    for ch in chars.iter_mut() {
        // SAFETY: a valid, aligned reference to a `char`, written with a
        // valid `char`.
        unsafe { std::ptr::write_volatile(ch, '\0') };
    }
    chars.clear();
    wipe_spare(chars);
}

/// Overwrites the room past the end of CHARS, where what was deleted from it
/// may still be.
fn wipe_spare(chars: &mut Vec<char>) {
    for slot in chars.spare_capacity_mut() {
        // SAFETY: the slot is allocated, aligned storage for a `char`, and a
        // valid `char` is written to it; the length does not change, so it
        // is never read as initialised.
        unsafe { std::ptr::write_volatile(slot.as_mut_ptr(), '\0') };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// Overwrites BYTES with zeros.
pub(crate) fn wipe_bytes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: a valid, aligned reference to a byte.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// Overwrites TEXT with zeros, and empties it: for a string made from a
/// secret, once it has been used.
pub(crate) fn wipe_string(text: &mut String) {
    // SAFETY: zeros are valid UTF-8, so the string stays a string.
    wipe_bytes(unsafe { text.as_bytes_mut() });
    text.clear();
}

// ---------------------------------------------------------------------------
// The prompt
// ---------------------------------------------------------------------------

/// What is done with a password once it has been typed.
pub(crate) enum Answer<B: BufferTrait> {
    /// FUNCTION is called with it, as a string, when it is not nil; with a
    /// key, it goes into the cache under that key first.
    Call {
        function: ELispExp<B>,
        cache_key: Option<String>,
    },
    /// It is written to the input of the process PROCESS, and NEWLINE after
    /// it, as it is in the secret: no string of Lisp's is made of it. Then
    /// ON_SENT, when it is not nil, is called with whether it was written.
    Send {
        process: String,
        newline: String,
        on_sent: ELispExp<B>,
    },
}

/// What a password is asked for: everything a prompt needs to be opened, and
/// opened again for its confirmation, kept from one to the next.
pub(crate) struct Request<B: BufferTrait> {
    /// The question.
    pub(crate) prompt: String,
    /// Whether the password is asked for twice, and must be the same.
    pub(crate) confirm: bool,
    /// What an empty answer stands for.
    pub(crate) default: Option<String>,
    /// Called, with no arguments, when the prompt is closed without an
    /// answer, when it is not nil.
    pub(crate) on_cancel: ELispExp<B>,
    /// What is done with the answer.
    pub(crate) answer: Answer<B>,
}

/// The password prompt that is open: what was asked, what has been typed,
/// and whether it is shown. Kept in the data of the prompt's buffer, under
/// [`PROMPT_DATA`], and dropped -- the secrets overwritten -- with it.
pub(crate) struct PasswordPrompt<B: BufferTrait> {
    pub(crate) request: Arc<Request<B>>,
    /// What has been typed.
    pub(crate) typed: Secret,
    /// The first answer, while the second asks for it again.
    pub(crate) first: Option<Secret>,
    /// Whether the buffer shows the password itself rather than hiding it.
    pub(crate) shown: bool,
}

impl<B: BufferTrait> PasswordPrompt<B> {
    /// Returns the prompt for REQUEST, nothing typed, the first answer FIRST
    /// when this one confirms it.
    pub(crate) fn new(request: Arc<Request<B>>, first: Option<Secret>) -> Self {
        Self {
            request,
            typed: Secret::new(),
            first,
            shown: false,
        }
    }

    /// Returns whether the buffer has a character for each one typed, so
    /// that its point says where in the password the cursor is: when the
    /// password is shown, or hidden behind HIDE.
    fn mirrored(&self, hide: Option<char>) -> bool {
        self.shown || hide.is_some()
    }

    /// Returns where in the password the cursor is, the buffer's point
    /// being POINT: point itself when the buffer has a character for each one
    /// typed, and the end otherwise, since nothing shows where else it is.
    pub(crate) fn cursor(&self, point: usize, hide: Option<char>) -> usize {
        if self.mirrored(hide) {
            point.min(self.typed.len())
        } else {
            self.typed.len()
        }
    }
}

/// Returns whether the current buffer is an open password prompt.
pub(crate) fn is_password_prompt<B: BufferTrait>(ctx: &EditorState<B>) -> bool {
    ctx.with_current_buffer(|buf| buf.name == PROMPT_BUFFER && buf.data.contains_key(PROMPT_DATA))
}

/// Returns what is drawn for each character of a hidden password: the first
/// character of `read-hide-char`, `*` when it is unbound or no string, and
/// nothing when it is nil or empty. A control character, which would not be
/// seen, is drawn as `*`.
pub(crate) fn hide_char<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<char> {
    match env.get_variable(READ_HIDE_CHAR) {
        Some(ELispExp::String(text)) => text.chars().next().map(|ch| {
            if ch.is_control() {
                DEFAULT_HIDE_CHAR
            } else {
                ch
            }
        }),
        Some(value) if value.is_nil() => None,
        _ => Some(DEFAULT_HIDE_CHAR),
    }
}

/// Puts in BUF what PROMPT shows -- a HIDE for each character typed,
/// nothing, or the password itself while it is shown -- with point at
/// CURSOR, or at the start of nothing.
///
/// Put there as a file's text is read in, not through the doors: no undo
/// record is made of it, nothing anchored to the old text is kept, and the
/// buffer stays read-only for every command but these.
pub(crate) fn redraw<B: BufferTrait>(
    buf: &mut Buffer<B>,
    prompt: &PasswordPrompt<B>,
    cursor: usize,
    hide: Option<char>,
) {
    let mut shown = if prompt.shown {
        prompt.typed.reveal()
    } else {
        match hide {
            Some(ch) => std::iter::repeat_n(ch, prompt.typed.len()).collect(),
            None => String::new(),
        }
    };
    buf.adopt_text(&shown);
    if prompt.shown {
        wipe_string(&mut shown);
    }
    let at = if prompt.mirrored(hide) { cursor } else { 0 };
    let (line, column) = buf.text.cursor_1d_to_2d(at.min(buf.text.len()));
    buf.text.cursor_move(line, column);
}

/// Makes BUF, the prompt's buffer just opened in `read-passwd-mode`, the
/// prompt PROMPT: read-only, its keymap claiming `C-u` from the prefix
/// argument's reader, nothing typed.
pub(crate) fn attach<B: BufferTrait>(
    buf: &mut Buffer<B>,
    prompt: PasswordPrompt<B>,
    hide: Option<char>,
) {
    // The mode's map as the buffer's local one too: a local map comes before
    // the prefix argument's reader, so that `C-u` erases what was typed, as
    // in Emacs, instead of starting an argument nothing here would take.
    buf.put_data(LOCAL_KEYMAP, ELispExp::<B>::symbol(PASSWORD_MODE.into()));
    buf.set_read_only(true);
    redraw(buf, &prompt, 0, hide);
    buf.put_data(PROMPT_DATA, prompt);
}

/// Installs `read-passwd-mode`, the keymap and hook a password prompt runs
/// under, and the first values of the settings this module reads.
pub fn install_password<B: BufferTrait>(
    editor_state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) {
    env.set_variable(
        READ_HIDE_CHAR.into(),
        ELispExp::string(DEFAULT_HIDE_CHAR.to_string()),
    );
    env.set_variable(PASSWORD_CACHE.into(), ELispExp::t());
    env.set_variable(
        PASSWORD_CACHE_EXPIRY.into(),
        ELispExp::number(DEFAULT_CACHE_EXPIRY),
    );

    let mut mode = MajorMode::new(PASSWORD_MODE);
    let ctrl = |ch: char| KeyEvent {
        code: KeyCode::Char(ch),
        modifiers: KeyModifiers {
            ctrl: true,
            ..Default::default()
        },
    };
    for (key, command) in [
        (KeyEvent::new(KeyCode::Enter), "read-passwd-confirm"),
        (KeyEvent::new(KeyCode::Esc), "minibuffer-cancel"),
        (ctrl('g'), "minibuffer-cancel"),
        // Nothing to complete: Tab shows the password, as in Emacs 30.
        (KeyEvent::new(KeyCode::Tab), "read-passwd-toggle-visibility"),
        (
            KeyEvent::new(KeyCode::Backspace),
            "read-passwd-delete-backward-char",
        ),
        (KeyEvent::new(KeyCode::Delete), "read-passwd-delete-char"),
        (ctrl('d'), "read-passwd-delete-char"),
        (ctrl('k'), "read-passwd-kill-line"),
        (ctrl('u'), "read-passwd-erase"),
        (ctrl('y'), "read-passwd-yank"),
    ] {
        mode.keymaps
            .insert_key(key, ELispExp::symbol(command.into()));
    }
    // Every other character goes into the password, not into the buffer.
    mode.keymaps
        .set_character_handler(ELispExp::symbol("read-passwd-self-insert".into()));
    // As a prompt's, however it closed: the buffer before it current again,
    // and the prompt's variables cleared.
    mode.hooks.insert(
        "after-close-hook".into(),
        vec![ELispExp::symbol("minibuffer-cleanup".into())],
    );
    editor_state.set_mode(PASSWORD_MODE, mode);
}

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

/// A password kept by the cache, and until when.
struct Cached {
    secret: Secret,
    /// When it goes; `None` for never.
    until: Option<Instant>,
}

/// The passwords kept for a while, by key: `password-cache-add` puts one in,
/// and it goes `password-cache-expiry` seconds later, or when it is removed
/// or the cache reset. An expired one is never handed out, and is overwritten
/// the next time the cache is looked at, or a key is pressed.
#[derive(Default)]
pub struct PasswordCache {
    entries: HashMap<String, Cached>,
}

impl PasswordCache {
    /// Keeps SECRET under KEY, in place of what was there, for LIFETIME from
    /// NOW, or for ever when it is `None`.
    pub(crate) fn add(
        &mut self,
        key: &str,
        secret: Secret,
        lifetime: Option<Duration>,
        now: Instant,
    ) {
        self.entries.insert(
            key.to_string(),
            Cached {
                secret,
                until: lifetime.map(|lifetime| now + lifetime),
            },
        );
    }

    /// Returns the password kept under KEY, as a string, unless it has
    /// expired by NOW.
    pub(crate) fn get(&mut self, key: &str, now: Instant) -> Option<String> {
        self.purge(now);
        self.entries.get(key).map(|cached| cached.secret.reveal())
    }

    /// Returns whether a password is kept under KEY, and has not expired by
    /// NOW.
    pub(crate) fn contains(&mut self, key: &str, now: Instant) -> bool {
        self.purge(now);
        self.entries.contains_key(key)
    }

    /// Forgets the password under KEY, and returns whether there was one.
    pub(crate) fn remove(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// Forgets them all, and returns how many there were.
    pub(crate) fn clear(&mut self) -> usize {
        let count = self.entries.len();
        self.entries.clear();
        count
    }

    /// Forgets those expired by NOW.
    pub(crate) fn purge(&mut self, now: Instant) {
        self.entries
            .retain(|_, cached| cached.until.is_none_or(|until| now < until));
    }

    /// Returns whether it keeps nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Returns whether passwords are kept: `password-cache`, true when unbound.
pub(crate) fn cache_enabled<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.get_variable(PASSWORD_CACHE)
        .is_none_or(|value| value.is_truthy())
}

/// Returns how long a password stays in the cache: `password-cache-expiry`
/// seconds when it is a number of at least zero, for ever when it is nil, and
/// 16 seconds otherwise.
pub(crate) fn cache_lifetime<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
    match env.get_variable(PASSWORD_CACHE_EXPIRY) {
        Some(ELispExp::Number(seconds)) if seconds.is_finite() && seconds >= 0.0 => {
            Some(Duration::from_secs_f64(seconds))
        }
        Some(value) if value.is_nil() => None,
        _ => Some(Duration::from_secs_f64(DEFAULT_CACHE_EXPIRY)),
    }
}
