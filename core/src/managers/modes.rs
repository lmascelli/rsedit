//! What a mode is, what keys do, and where completions come from.
//!
//! # Why this is a compartment and not eight fields
//!
//! The registry, the global keymap, the global completion list and the
//! file-name patterns are four halves of two things: a mode's keymap and the
//! global one are consulted in that order on every keystroke, and a mode's
//! completion sources and the global ones are merged on every completion. A
//! caller that had to reach both had to know there were two places and take
//! them in the right order.
//!
//! The other four were two pairs -- a payload and an atomic gate in front of
//! it. See below for why those pairs stop existing.
//!
//! # What the gates were for, and why they are gone
//!
//! `transient_keymap` and `repeat_keys` each had an `AtomicBool` beside them,
//! so that a keystroke finding no transient map and no repeat key paid a load
//! rather than a lock. Keeping a flag and a payload in step across two
//! locations needed a protocol, and the protocol was written down twice: the
//! gate was opened *after* the map was stored and closed *before* it was
//! taken away, so that a reader through the gate always found something there.
//! Key resolution still carried a defensive branch for "the gate was open but
//! the map had gone", commented as unreachable.
//!
//! Under one lock that state cannot be represented, so the protocol, the
//! comments and the branch are all gone. It also costs less rather than more:
//! resolving one keystroke used to take the gate, the transient map, the
//! registry and the global keymap -- four acquisitions where there is now one.
//!
//! # What is deliberately not here
//!
//! Buffers. A buffer names its mode and this names none of the buffers: where
//! an operation needs to know what mode is current, the *name* is passed in.
//! Nothing in this file can reach a buffer's text.
//!
//! Lisp. The lists here hold `ELispExp` values, but nothing here calls one.
//! Every accessor copies the expressions out and gives the lock back, because
//! a completion source or a hook is arbitrary user code that can re-enter the
//! editor through any primitive -- and a lock held across that is a lock
//! offered to arbitrary code.
use crate::{
    ELispExp,
    buffer::BufferTrait,
    input::{KeyEvent, Keymap, TransientKeymap},
    modes::{MajorMode, SyntaxTable},
};
use std::collections::HashMap;

pub struct Modes<B: BufferTrait> {
    registry: HashMap<String, MajorMode<B>>,
    /// The keymap in force in every buffer, whatever its mode -- the global
    /// half of what `MajorMode::keymaps` holds per mode.
    global_keymap: Keymap<B>,
    /// Where completions come from in any buffer, whatever its mode -- the
    /// global half of `MajorMode::completion_functions`, and the same
    /// relationship `global_keymap` has to `MajorMode::keymaps`.
    global_completions: Vec<ELispExp<B>>,
    /// Which file names get which major mode, in the order they were declared.
    ///
    /// An ordered list rather than a map: patterns overlap -- `\.rs$` and
    /// `^Cargo\.` both match `Cargo.rs` -- so which one wins has to be a
    /// decision somebody made rather than whichever the hash happened to
    /// answer with.
    auto_modes: Vec<(regex::Regex, String)>,
    /// A keymap consulted before every other, for as long as it is installed.
    /// See [`TransientKeymap`].
    ///
    /// A plain `Option` rather than an option behind a gate: the gate existed
    /// to keep it in step with a flag that is no longer separate from it.
    transient: Option<TransientKeymap<B>>,
    /// Which commands offer to repeat, and with which key.
    repeat_keys: HashMap<String, KeyEvent>,
    /// Hooks that run in every major mode, keyed by hook name.
    ///
    /// The global half of `MajorMode::hooks`, and here for the same reason
    /// `global_keymap` and `global_completions` are: the two halves are merged
    /// on every read, so keeping them apart meant every reader knowing there
    /// were two places and taking them in the right order.
    global_hooks: HashMap<String, Vec<ELispExp<B>>>,
}

impl<B: BufferTrait> Modes<B> {
    pub fn new(global_keymap: Keymap<B>) -> Self {
        Self {
            registry: HashMap::new(),
            global_keymap,
            global_completions: Vec::new(),
            auto_modes: Vec::new(),
            transient: None,
            repeat_keys: HashMap::new(),
            global_hooks: HashMap::new(),
        }
    }

    // ------------------------------------------------------------------
    // The registry
    // ------------------------------------------------------------------

    pub fn get(&self, name: &str) -> Option<&MajorMode<B>> {
        self.registry.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.registry.contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.registry.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    pub fn insert(&mut self, name: &str, mode: MajorMode<B>) {
        self.registry.insert(name.to_string(), mode);
    }

    /// Change one mode, reporting whether it was there.
    pub fn edit(&mut self, name: &str, edit: impl FnOnce(&mut MajorMode<B>)) -> bool {
        match self.registry.get_mut(name) {
            Some(mode) => {
                edit(mode);
                true
            }
            None => false,
        }
    }

    /// The syntax table for MODE, or the default one when it has none.
    ///
    /// Cloned out, because every caller is about to run a scan with it and a
    /// scan reads a whole buffer -- far too long to hold this against
    /// everything else that wants a mode.
    pub fn syntax_table(&self, mode: &str) -> SyntaxTable {
        self.registry
            .get(mode)
            .and_then(|mode| mode.syntax_table.clone())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------------
    // Keymaps
    // ------------------------------------------------------------------

    pub fn global_keymap(&self) -> &Keymap<B> {
        &self.global_keymap
    }

    pub fn global_keymap_mut(&mut self) -> &mut Keymap<B> {
        &mut self.global_keymap
    }

    /// MODE's own keymap, when it has one.
    pub fn mode_keymap(&self, mode: &str) -> Option<&Keymap<B>> {
        self.registry.get(mode).map(|mode| &mode.keymaps)
    }

    // ------------------------------------------------------------------
    // The transient keymap
    // ------------------------------------------------------------------

    /// Install a map consulted before every other until it goes away.
    ///
    /// Replaces any map already installed rather than stacking: two maps
    /// competing for the same keystroke could not both win, and the newer one
    /// is always the more recent thing the user asked for.
    pub fn set_transient(&mut self, map: TransientKeymap<B>) {
        self.transient = Some(map);
    }

    /// Take the map down. Idempotent, so a command that ends one can call it
    /// without first asking whether one is up.
    pub fn clear_transient(&mut self) {
        self.transient = None;
    }

    pub fn transient_active(&self) -> bool {
        self.transient.is_some()
    }

    /// What the installed map wants shown, or empty when none is installed.
    pub fn transient_message(&self) -> String {
        self.transient
            .as_ref()
            .map(|map| map.message.clone())
            .unwrap_or_default()
    }

    pub fn transient(&self) -> Option<&TransientKeymap<B>> {
        self.transient.as_ref()
    }

    // ------------------------------------------------------------------
    // Completion sources
    // ------------------------------------------------------------------

    /// Everything that could complete in MODE: the mode's own sources first,
    /// then the global ones.
    ///
    /// Copied out, because every one of these is about to be *called* and a
    /// source is arbitrary Lisp.
    pub fn completion_sources(&self, mode: &str) -> Vec<ELispExp<B>> {
        let mut sources: Vec<ELispExp<B>> = self
            .registry
            .get(mode)
            .map(|mode| mode.completion_functions.clone())
            .unwrap_or_default();
        sources.extend(self.global_completions.iter().cloned());
        sources
    }

    /// One list on its own, unmerged, or `None` if MODE is unknown.
    pub fn completion_list(&self, mode: Option<&str>) -> Option<Vec<ELispExp<B>>> {
        match mode {
            None => Some(self.global_completions.clone()),
            Some(name) => self
                .registry
                .get(name)
                .map(|mode| mode.completion_functions.clone()),
        }
    }

    /// Append a source to MODE's list, or to the global one when MODE is
    /// `None`. False if MODE names a mode that does not exist.
    pub fn add_completion(&mut self, mode: Option<&str>, function: ELispExp<B>) -> bool {
        match mode {
            None => {
                self.global_completions.push(function);
                true
            }
            Some(name) => self.edit(name, |mode| mode.completion_functions.push(function)),
        }
    }

    /// Replace a whole list. This is how a source is removed or the order
    /// changed, which a bare append could not express.
    pub fn set_completions(&mut self, mode: Option<&str>, functions: Vec<ELispExp<B>>) -> bool {
        match mode {
            None => {
                self.global_completions = functions;
                true
            }
            Some(name) => self.edit(name, |mode| mode.completion_functions = functions),
        }
    }

    // ------------------------------------------------------------------
    // File names, and repeat keys
    // ------------------------------------------------------------------

    /// Say that a file whose name matches PATTERN opens in MODE.
    pub fn add_auto_mode(&mut self, pattern: regex::Regex, mode: &str) {
        self.auto_modes.push((pattern, mode.to_string()));
    }

    /// The mode a file called PATH should open in, if any pattern claims it.
    ///
    /// Matched against the whole path, so a pattern can key on a directory as
    /// well as an extension. First declared wins, which is what makes the
    /// order a decision rather than an accident.
    pub fn auto_mode_for(&self, path: &str) -> Option<String> {
        self.auto_modes
            .iter()
            .find(|(pattern, _)| pattern.is_match(path))
            .map(|(_, mode)| mode.clone())
    }

    /// Say that COMMAND may be repeated by pressing KEY on its own afterwards.
    ///
    /// Declared rather than inferred. The tempting rule -- "after a sequence
    /// ending in K, a bare K repeats" -- would make `C-x C-f` followed by `f`
    /// re-open `find-file`, which is not a convenience.
    pub fn set_repeat_key(&mut self, command: &str, key: KeyEvent) {
        self.repeat_keys.insert(command.to_string(), key);
    }

    /// The key that repeats COMMAND, if it has one.
    pub fn repeat_key(&self, command: &str) -> Option<KeyEvent> {
        self.repeat_keys.get(command).cloned()
    }

    // ------------------------------------------------------------------
    // Hooks
    // ------------------------------------------------------------------

    /// Register FUNCTION to run under HOOK_NAME in every major mode.
    pub fn add_global_hook(&mut self, hook_name: &str, function: ELispExp<B>) {
        self.global_hooks
            .entry(hook_name.to_string())
            .or_default()
            .push(function);
    }

    /// Everything registered under HOOK_NAME for MODE: the mode's own first,
    /// then the ones registered for every mode.
    ///
    /// A mode-specific hook is the more specific statement about this buffer,
    /// so it gets to act before anything general reacts to the result.
    ///
    /// Copied out, and the caller must let this lock go before running any of
    /// them. A hook can call `add-hook`, `define-key`, `make-mode` or
    /// `add-syntax-rule`, every one of which wants this lock on the write
    /// side -- which is not a race but a hang, every time, on one thread,
    /// from ordinary user Lisp.
    pub fn hooks_for(&self, mode: &str, hook_name: &str) -> Vec<ELispExp<B>> {
        let mut hooks: Vec<ELispExp<B>> = self
            .registry
            .get(mode)
            .and_then(|mode| mode.hooks.get(hook_name))
            .cloned()
            .unwrap_or_default();
        hooks.extend(
            self.global_hooks
                .get(hook_name)
                .cloned()
                .unwrap_or_default(),
        );
        hooks
    }
}
