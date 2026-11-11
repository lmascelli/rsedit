//! What a mode is, what keys do, and where completions come from: the
//! registry of major modes, the global halves of what a mode holds -- keymap,
//! completion sources, hooks -- the transient keymap, the repeat keys and the
//! file-name patterns. Every accessor copies Lisp out.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use crate::{
    ELispExp,
    buffer::BufferTrait,
    input::{KeyEvent, Keymap, OnUnbound, TransientKeymap, describe_keys},
    modes::{MajorMode, SyntaxTable},
};
use std::collections::HashMap;

/// Which map a binding came from, in the order the maps are consulted: what
/// somebody looking a key up wants to know beside what it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingSource {
    /// A map installed for one question and taken down after it.
    Transient,
    /// A buffer-local map layered above its minor modes.
    Local,
    /// A minor mode on in the buffer.
    Minor,
    /// The current buffer's mode.
    Mode,
    /// In force everywhere.
    Global,
}

impl BindingSource {
    /// Returns the name Lisp sees.
    pub fn name(self) -> &'static str {
        match self {
            BindingSource::Transient => "transient",
            BindingSource::Local => "local",
            BindingSource::Minor => "minor",
            BindingSource::Mode => "mode",
            BindingSource::Global => "global",
        }
    }
}

/// One binding in effect: what to press, what it runs, and where it is from.
#[derive(Clone, Debug)]
pub struct Binding<B: BufferTrait> {
    pub keys: Vec<KeyEvent>,
    pub target: ELispExp<B>,
    pub source: BindingSource,
}

impl<B: BufferTrait> Binding<B> {
    /// Creates the binding of KEYS to TARGET, found in SOURCE.
    fn new(keys: &[KeyEvent], target: ELispExp<B>, source: BindingSource) -> Self {
        Self {
            keys: keys.to_vec(),
            target,
            source,
        }
    }

    /// Returns the keys, spelt the way a binding is written.
    pub fn described(&self) -> String {
        describe_keys(&self.keys)
    }
}

/// The mode registry, the global keymap, completion sources and hooks, the
/// file-name patterns, the transient keymap and the repeat keys.
pub struct Modes<B: BufferTrait> {
    registry: HashMap<String, MajorMode<B>>,
    /// The keymap in force in every buffer, whatever its mode -- the global
    /// half of what `MajorMode::keymaps` holds per mode.
    global_keymap: Keymap<B>,
    /// Where completions come from in any buffer, whatever its mode -- the
    /// global half of `MajorMode::completion_functions`, and the same
    /// relationship `global_keymap` has to `MajorMode::keymaps`.
    global_completions: Vec<ELispExp<B>>,
    /// Which file names get which major mode, in the order they were
    /// declared: patterns overlap, and the first declared wins.
    auto_modes: Vec<(regex::Regex, String)>,
    /// A keymap consulted before every other, for as long as it is
    /// installed: see [`TransientKeymap`].
    transient: Option<TransientKeymap<B>>,
    /// Which commands offer to repeat, and with which key.
    repeat_keys: HashMap<String, KeyEvent>,
    /// Hooks that run in every major mode, by hook name: the global half of
    /// `MajorMode::hooks`.
    global_hooks: HashMap<String, Vec<ELispExp<B>>>,
    /// Which minor modes a buffer gets with its major mode: the major mode,
    /// or `None` for every one, and the minor mode, in the order they were
    /// asked for.
    minor_rules: Vec<(Option<String>, String)>,
}

impl<B: BufferTrait> Modes<B> {
    /// Creates the compartment, with GLOBAL_KEYMAP in force everywhere.
    pub fn new(global_keymap: Keymap<B>) -> Self {
        Self {
            registry: HashMap::new(),
            global_keymap,
            global_completions: Vec::new(),
            auto_modes: Vec::new(),
            transient: None,
            repeat_keys: HashMap::new(),
            global_hooks: HashMap::new(),
            minor_rules: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // The registry
    // ------------------------------------------------------------------

    /// Returns the names of the modes there are.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.registry.keys()
    }

    /// Returns the mode called NAME.
    pub fn get(&self, name: &str) -> Option<&MajorMode<B>> {
        self.registry.get(name)
    }

    /// Adds MODE as NAME, in place of any mode of that name, whole. What
    /// `make-mode` does to a mode that exists is not this: see
    /// `EditorState::make_mode`, which keeps what others gave it.
    pub fn insert(&mut self, name: &str, mode: MajorMode<B>) {
        self.registry.insert(name.to_string(), mode);
    }

    /// Changes the mode called NAME with EDIT, and returns whether it was
    /// there.
    pub fn edit(&mut self, name: &str, edit: impl FnOnce(&mut MajorMode<B>)) -> bool {
        match self.registry.get_mut(name) {
            Some(mode) => {
                edit(mode);
                true
            }
            None => false,
        }
    }

    /// Returns MODE's syntax table, or the default one, cloned out: a scan
    /// with it reads a whole buffer, far too long to hold this lock.
    pub fn syntax_table(&self, mode: &str) -> SyntaxTable {
        self.registry
            .get(mode)
            .and_then(|mode| mode.syntax_table.clone())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------------
    // Keymaps
    // ------------------------------------------------------------------

    /// Returns the global keymap itself, for tests only: the editor asks
    /// [`Modes::binding_for`] and its relatives, which walk the maps as key
    /// resolution does.
    #[cfg(test)]
    pub fn global_keymap(&self) -> &Keymap<B> {
        &self.global_keymap
    }

    /// Returns the global keymap, to bind keys in.
    pub fn global_keymap_mut(&mut self) -> &mut Keymap<B> {
        &mut self.global_keymap
    }

    /// Returns which kind of map MODE's is, at INDEX among MODES, the maps of
    /// a buffer: a minor mode's, the major mode's, which comes last, or the
    /// local one.
    fn source_of(&self, mode: &str, index: usize, modes: &[&str]) -> BindingSource {
        if self.registry.get(mode).is_some_and(MajorMode::is_minor) {
            BindingSource::Minor
        } else if index + 1 < modes.len() {
            BindingSource::Local
        } else {
            BindingSource::Mode
        }
    }

    /// Returns whether NAME is a minor mode.
    pub fn is_minor(&self, name: &str) -> bool {
        self.registry.get(name).is_some_and(MajorMode::is_minor)
    }

    /// Returns what the status line shows for the minor mode NAME, or `None`
    /// when NAME is no minor mode.
    pub fn lighter_of(&self, name: &str) -> Option<String> {
        self.registry
            .get(name)
            .and_then(|mode| mode.lighter.clone())
    }

    /// Records that buffers in MAJOR, or in every major mode when it is
    /// `None`, get MINOR on; asking twice records once.
    pub fn add_minor_rule(&mut self, major: Option<&str>, minor: &str) {
        let rule = (major.map(str::to_string), minor.to_string());
        if !self.minor_rules.contains(&rule) {
            self.minor_rules.push(rule);
        }
    }

    /// Forgets that buffers in MAJOR, or in every major mode when it is
    /// `None`, get MINOR on: the rule [`Self::add_minor_rule`] recorded for
    /// the same two. Returns whether there was one.
    pub fn remove_minor_rule(&mut self, major: Option<&str>, minor: &str) -> bool {
        let before = self.minor_rules.len();
        self.minor_rules.retain(|(rule_major, rule_minor)| {
            rule_major.as_deref() != major || rule_minor != minor
        });
        self.minor_rules.len() != before
    }

    /// Returns the minor modes a buffer in MAJOR gets, in the order they were
    /// asked for, each once.
    pub fn minors_for(&self, major: &str) -> Vec<String> {
        let mut minors: Vec<String> = Vec::new();
        for (rule_major, minor) in &self.minor_rules {
            let applies = rule_major.as_deref().is_none_or(|rule| rule == major);
            if applies && !minors.contains(minor) {
                minors.push(minor.clone());
            }
        }
        minors
    }

    /// Returns MODE's own keymap, when it has one.
    fn mode_keymap(&self, mode: &str) -> Option<&Keymap<B>> {
        self.registry.get(mode).map(|mode| &mode.keymaps)
    }

    // ------------------------------------------------------------------
    // Reading the keymaps back
    // ------------------------------------------------------------------
    //
    // About the bindings in effect rather than any one keymap: the transient
    // map, then the buffer's modes, then the global map. Written once, here,
    // and asked by key resolution and by help alike, so that the two agree.

    /// Returns what KEYS would run now, and from which map, in a buffer whose
    /// keymaps are MODES' -- the local ones first, the major mode's last.
    ///
    /// `None` for a sequence bound to nothing, a prefix of a binding included:
    /// [`Modes::is_prefix`] tells the two apart. Under a transient map that
    /// refuses what it does not bind, only its own bindings run, as in key
    /// resolution.
    pub fn binding_for(&self, modes: &[&str], keys: &[KeyEvent]) -> Option<Binding<B>> {
        if let Some(map) = self.transient() {
            if let Some(target) = map.keymap.get(keys) {
                return Some(Binding::new(keys, target.clone(), BindingSource::Transient));
            }
            if map.on_unbound == OnUnbound::Refuse {
                return None;
            }
        }
        self.binding_below_transient(modes, keys)
    }

    /// Returns what KEYS runs in the maps below the transient one: MODES'
    /// keymaps, then the global one. Key resolution asks this after handling
    /// the transient map itself, where a miss may mean refusing the key.
    pub fn binding_below_transient(&self, modes: &[&str], keys: &[KeyEvent]) -> Option<Binding<B>> {
        for (index, mode) in modes.iter().enumerate() {
            if let Some(keymap) = self.mode_keymap(mode)
                && let Some(target) = keymap.binding(keys)
            {
                return Some(Binding::new(
                    keys,
                    target,
                    self.source_of(mode, index, modes),
                ));
            }
        }
        self.global_keymap
            .binding(keys)
            .map(|target| Binding::new(keys, target, BindingSource::Global))
    }

    /// Returns what a character typed alone runs when no map in effect binds
    /// it, and from which map: the first character handler, in the order key
    /// resolution reads the maps. `None` when none has one, or under a
    /// transient map that refuses what it does not bind.
    pub fn character_binding(&self, modes: &[&str]) -> Option<(ELispExp<B>, BindingSource)> {
        if self
            .transient()
            .is_some_and(|map| map.on_unbound == OnUnbound::Refuse)
        {
            return None;
        }
        for (index, mode) in modes.iter().enumerate() {
            if let Some(handler) = self.mode_keymap(mode).and_then(Keymap::character_handler) {
                return Some((handler.clone(), self.source_of(mode, index, modes)));
            }
        }
        self.global_keymap
            .character_handler()
            .map(|handler| (handler.clone(), BindingSource::Global))
    }

    /// Returns whether KEYS begins a longer binding in a map in effect, which
    /// keeps a half-typed sequence alive. Under a transient map that refuses,
    /// only its own sequences count.
    pub fn is_prefix(&self, modes: &[&str], keys: &[KeyEvent]) -> bool {
        match self.transient() {
            Some(map) if map.on_unbound == OnUnbound::Refuse => map.keymap.is_prefix(keys),
            Some(map) => map.keymap.is_prefix(keys) || self.is_prefix_below_transient(modes, keys),
            None => self.is_prefix_below_transient(modes, keys),
        }
    }

    /// Returns the same, asked of MODES' keymaps and the global one alone.
    pub fn is_prefix_below_transient(&self, modes: &[&str], keys: &[KeyEvent]) -> bool {
        modes.iter().any(|mode| {
            self.mode_keymap(mode)
                .is_some_and(|keymap| keymap.is_prefix(keys))
        }) || self.global_keymap.is_prefix(keys)
    }

    /// Returns every binding in effect, each sequence once under the map that
    /// wins it, sorted by keys. A binding a higher map hides cannot run, and
    /// is not listed.
    pub fn bindings(&self, modes: &[&str]) -> Vec<Binding<B>> {
        // Lowest priority first, so a later insert overwrites a lower map's
        // claim on the same sequence -- the same order resolution reads them
        // in, run backwards. Under a transient map that refuses what it does
        // not bind, the walk at the end leaves the lower maps' bindings out.
        let mut found: HashMap<Vec<KeyEvent>, Binding<B>> = HashMap::new();
        for (keys, target) in self.global_keymap.iter() {
            found.insert(
                keys.clone(),
                Binding::new(keys, target.clone(), BindingSource::Global),
            );
        }
        for (index, mode) in modes.iter().enumerate().rev() {
            if let Some(keymap) = self.mode_keymap(mode) {
                let source = if index + 1 < modes.len() {
                    BindingSource::Local
                } else {
                    BindingSource::Mode
                };
                for (keys, target) in keymap.iter() {
                    found.insert(keys.clone(), Binding::new(keys, target.clone(), source));
                }
            }
        }
        if let Some(map) = self.transient() {
            for (keys, target) in map.keymap.iter() {
                found.insert(
                    keys.clone(),
                    Binding::new(keys, target.clone(), BindingSource::Transient),
                );
            }
        }
        // A character fallback in a higher map can override an explicit
        // binding below it. Attribute each listed key through the same walk
        // used for dispatch, including those generated bindings.
        let mut bindings: Vec<Binding<B>> = found
            .into_values()
            .filter_map(|binding| self.binding_for(modes, &binding.keys))
            .collect();

        bindings.sort_by(|a, b| describe_keys(&a.keys).cmp(&describe_keys(&b.keys)));
        bindings
    }

    // ------------------------------------------------------------------
    // The transient keymap
    // ------------------------------------------------------------------

    /// Installs MAP, consulted before every other until it goes, in place of
    /// any map already installed: two maps cannot both win a key, and the
    /// newer is what was asked for last.
    pub fn set_transient(&mut self, map: TransientKeymap<B>) {
        self.transient = Some(map);
    }

    /// Takes the map down, if one is installed.
    pub fn clear_transient(&mut self) {
        self.transient = None;
    }

    /// Returns whether a map is installed, for tests only: nothing in the
    /// editor asks.
    #[cfg(test)]
    pub fn transient_active(&self) -> bool {
        self.transient.is_some()
    }

    /// Returns what the installed map is the question of, if anything.
    pub fn transient_owner(&self) -> Option<&'static str> {
        self.transient.as_ref().and_then(|map| map.owner)
    }

    /// Returns what the installed map wants shown, or nothing.
    pub fn transient_message(&self) -> String {
        self.transient
            .as_ref()
            .map(|map| map.message.clone())
            .unwrap_or_default()
    }

    /// Returns the installed map, if any.
    pub fn transient(&self) -> Option<&TransientKeymap<B>> {
        self.transient.as_ref()
    }

    // ------------------------------------------------------------------
    // Completion sources
    // ------------------------------------------------------------------

    /// Returns MODE's completion sources, then the global ones, copied out:
    /// each is about to be called, and is arbitrary Lisp.
    pub fn completion_sources(&self, mode: &str) -> Vec<ELispExp<B>> {
        let mut sources: Vec<ELispExp<B>> = self
            .registry
            .get(mode)
            .map(|mode| mode.completion_functions.clone())
            .unwrap_or_default();
        sources.extend(self.global_completions.iter().cloned());
        sources
    }

    /// Returns one list on its own: MODE's, or the global one for `None`;
    /// `None` if MODE is unknown.
    pub fn completion_list(&self, mode: Option<&str>) -> Option<Vec<ELispExp<B>>> {
        match mode {
            None => Some(self.global_completions.clone()),
            Some(name) => self
                .registry
                .get(name)
                .map(|mode| mode.completion_functions.clone()),
        }
    }

    /// Appends FUNCTION to MODE's sources, or to the global ones for `None`,
    /// and returns false if MODE is unknown.
    pub fn add_completion(&mut self, mode: Option<&str>, function: ELispExp<B>) -> bool {
        match mode {
            None => {
                self.global_completions.push(function);
                true
            }
            Some(name) => self.edit(name, |mode| mode.completion_functions.push(function)),
        }
    }

    /// Replaces a whole list, which is how a source is removed or the order
    /// changed; false if MODE is unknown.
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

    /// Says that a file whose path matches PATTERN opens in MODE.
    pub fn add_auto_mode(&mut self, pattern: regex::Regex, mode: &str) {
        self.auto_modes.push((pattern, mode.to_string()));
    }

    /// Returns the mode a file at PATH opens in: the first declared pattern
    /// matching the whole path, so that a pattern can name a directory.
    pub fn auto_mode_for(&self, path: &str) -> Option<String> {
        self.auto_modes
            .iter()
            .find(|(pattern, _)| pattern.is_match(path))
            .map(|(_, mode)| mode.clone())
    }

    /// Says that COMMAND may be repeated by pressing KEY on its own
    /// afterwards. Declared, not inferred from the binding: after `C-x C-f`,
    /// a bare `f` must not open another file.
    pub fn set_repeat_key(&mut self, command: &str, key: KeyEvent) {
        self.repeat_keys.insert(command.to_string(), key);
    }

    /// Returns the key that repeats COMMAND, if it has one.
    pub fn repeat_key(&self, command: &str) -> Option<KeyEvent> {
        self.repeat_keys.get(command).cloned()
    }

    // ------------------------------------------------------------------
    // Hooks
    // ------------------------------------------------------------------

    /// Registers FUNCTION under HOOK_NAME in every major mode, once.
    pub fn add_global_hook(&mut self, hook_name: &str, function: ELispExp<B>) {
        let hooks = self.global_hooks.entry(hook_name.to_string()).or_default();
        if !hooks.contains(&function) {
            hooks.push(function);
        }
    }

    /// Removes FUNCTION from HOOK in MODE, or from the global hooks for
    /// `None`, and returns false if MODE is unknown.
    pub fn remove_hook(&mut self, mode: Option<&str>, hook: &str, function: &ELispExp<B>) -> bool {
        if let Some(mode) = mode {
            return self.edit(mode, |mode| {
                if let Some(hooks) = mode.hooks.get_mut(hook) {
                    hooks.retain(|entry| entry != function);
                }
            });
        }
        if let Some(hooks) = self.global_hooks.get_mut(hook) {
            hooks.retain(|entry| entry != function);
        }
        true
    }

    /// Returns what is registered under HOOK_NAME for MODE and the minor
    /// modes MINORS: the major mode's own first, then each minor mode's in
    /// the order they were turned on, then the global ones.
    ///
    /// Copied out, and the caller gives this lock back before running them: a
    /// hook may call `add-hook` or `define-key`, which take it to write.
    pub fn hooks_for(&self, mode: &str, minors: &[String], hook_name: &str) -> Vec<ELispExp<B>> {
        let mut hooks: Vec<ELispExp<B>> = Vec::new();
        for name in std::iter::once(mode).chain(minors.iter().map(String::as_str)) {
            hooks.extend(
                self.registry
                    .get(name)
                    .and_then(|mode| mode.hooks.get(hook_name))
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        hooks.extend(
            self.global_hooks
                .get(hook_name)
                .cloned()
                .unwrap_or_default(),
        );
        hooks
    }
}
