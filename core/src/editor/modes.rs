//! The modes' half of the facade: changing a buffer's major mode, changing a
//! grammar or a syntax table and saying so to the jobs, running hooks, and the
//! completion sources, file-name patterns and keymaps the modes hold.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use super::*;

/// The hook a minor mode runs, with the buffer current, each time it is
/// turned on or off there.
pub(crate) const MINOR_MODE_HOOK: &str = "minor-mode-hook";

impl<B: BufferTrait> EditorState<B> {
    /// Makes MODE the major mode of the buffer NAME, keeping what the buffer
    /// holds, and runs the mode's hooks when that is a change -- once the
    /// buffer's lock has been given back. The minor modes on go, with no hook
    /// run, all but `read-only-mode`, and those `minor-mode-enable` asks for in
    /// MODE come on.
    pub(crate) fn change_major_mode(&self, name: &str, mode: &str, env: &Arc<Env<EditorState<B>>>) {
        let changed = self.with_buffer_mut(name, |buf| {
            if buf.current_mode == mode {
                return false;
            }
            buf.current_mode = mode.to_string();
            // A buffer that refuses changes goes on refusing them.
            buf.minor_modes
                .retain(|on| on.name == crate::buffer::READ_ONLY_MODE);
            buf.syntax.invalidate_from(buf.version, 0);
            buf.scan.invalidate_from(buf.version, 0);
            // The text is the same, but what it means is not: everything
            // worked out about it is to be worked out again.
            buf.changed.emit();
            true
        });
        if changed == Some(true) {
            self.run_buffer_hook(name, "major-mode-hook", env);
            self.apply_minor_rules(name, env);
            self.run_buffer_hook(name, "after-change-major-mode-hook", env);
        }
    }

    // ------------------------------------------------------------------
    // Minor modes
    // ------------------------------------------------------------------

    /// Turns the minor mode MINOR on in the buffer NAME, last of those on, and
    /// runs its `minor-mode-hook` with NAME current. Returns false, doing
    /// nothing, when MINOR is on there already, or NAME or MINOR is not
    /// there.
    pub(crate) fn minor_mode_on(
        &self,
        name: &str,
        minor: &str,
        env: &Arc<Env<EditorState<B>>>,
    ) -> bool {
        let Some(lighter) = self.modes(|modes| modes.lighter_of(minor)) else {
            return false;
        };
        let turned = self.with_buffer_mut(name, |buf| {
            if buf.minor_modes.iter().any(|on| on.name == minor) {
                return false;
            }
            buf.minor_modes.push(crate::buffer::ActiveMinor {
                name: minor.to_string(),
                lighter,
            });
            true
        });
        if turned == Some(true) {
            self.run_minor_mode_hook(name, minor, env);
            return true;
        }
        false
    }

    /// Turns the minor mode MINOR off in the buffer NAME, and runs its
    /// `minor-mode-hook` with NAME current. Returns false, doing nothing,
    /// when it was not on there.
    pub(crate) fn minor_mode_off(
        &self,
        name: &str,
        minor: &str,
        env: &Arc<Env<EditorState<B>>>,
    ) -> bool {
        let turned = self.with_buffer_mut(name, |buf| {
            let before = buf.minor_modes.len();
            buf.minor_modes.retain(|on| on.name != minor);
            buf.minor_modes.len() != before
        });
        if turned == Some(true) {
            self.run_minor_mode_hook(name, minor, env);
            return true;
        }
        false
    }

    /// Returns the minor modes on in the buffer NAME, in the order they were
    /// turned on; none for a buffer that is not there.
    pub(crate) fn minor_modes_of(&self, name: &str) -> Vec<String> {
        self.with_buffer(name, |buf| {
            buf.minor_modes.iter().map(|on| on.name.clone()).collect()
        })
        .unwrap_or_default()
    }

    /// Turns on in the buffer NAME every minor mode `minor-mode-enable` asks
    /// for in its major mode.
    pub(crate) fn apply_minor_rules(&self, name: &str, env: &Arc<Env<EditorState<B>>>) {
        let Some(major) = self.with_buffer(name, |buf| buf.current_mode.clone()) else {
            return;
        };
        for minor in self.modes(|modes| modes.minors_for(&major)) {
            self.minor_mode_on(name, &minor, env);
        }
    }

    /// Runs MINOR's own `minor-mode-hook`, and the one of every mode, with the
    /// buffer NAME current.
    fn run_minor_mode_hook(&self, name: &str, minor: &str, env: &Arc<Env<EditorState<B>>>) {
        let Some(previous) = self.set_current_buffer(name) else {
            return;
        };
        self.run_hook_of(minor, &[], MINOR_MODE_HOOK, env);
        self.set_current_buffer(&previous);
    }

    // ------------------------------------------------------------------
    // Completion sources: one list per mode, and one for every mode
    // ------------------------------------------------------------------

    /// Returns every completion source to try in MODE, the mode's own before
    /// the global ones: in a Lisp buffer the interpreter's names are the good
    /// answer and the words of other buffers the fallback, and only the mode
    /// can say so.
    ///
    /// Copied out, since each is about to be called, and a source is Lisp.
    pub(crate) fn completion_sources(&self, mode: &str) -> Vec<ELispExp<B>> {
        self.modes(|modes| modes.completion_sources(mode))
    }

    /// Adds FUNCTION to MODE's sources, or to the global ones when MODE is
    /// `None`. False when MODE names no mode.
    pub(crate) fn add_completion_function(
        &self,
        mode: Option<&str>,
        function: ELispExp<B>,
    ) -> bool {
        self.modes_mut(|modes| modes.add_completion(mode, function))
    }

    /// Replaces MODE's sources, or the global ones, with FUNCTIONS: how a
    /// source is taken out, or the order changed --
    /// `(set-completion-functions nil (list ...))`.
    pub(crate) fn set_completion_functions(
        &self,
        mode: Option<&str>,
        functions: Vec<ELispExp<B>>,
    ) -> bool {
        self.modes_mut(|modes| modes.set_completions(mode, functions))
    }

    /// Returns MODE's own sources, or the global ones, unmerged: `None` when
    /// MODE names no mode.
    pub(crate) fn completion_function_list(&self, mode: Option<&str>) -> Option<Vec<ELispExp<B>>> {
        self.modes(|modes| modes.completion_list(mode))
    }

    // ------------------------------------------------------------------
    // The mode a file opens in
    // ------------------------------------------------------------------

    /// Opens in MODE the files whose name matches PATTERN: nothing else leads
    /// from a file to its language's mode.
    pub(crate) fn add_auto_mode(&self, pattern: regex::Regex, mode: &str) {
        self.modes_mut(|modes| modes.add_auto_mode(pattern, mode));
    }

    /// Returns the mode a file at PATH opens in, if a pattern claims it.
    /// Matched against the whole path, so a pattern can name a directory as
    /// well as an extension.
    pub(crate) fn auto_mode_for(&self, path: &str) -> Option<String> {
        self.modes(|modes| modes.auto_mode_for(path))
    }

    // ------------------------------------------------------------------
    // Grammars and syntax tables
    // ------------------------------------------------------------------

    /// Defines MODE as MODE_NAME, replacing what was defined under that name.
    ///
    /// The replacement brings its own grammar -- an empty one, usually, since
    /// `make-mode` is the first line of a mode's file -- so for what is already
    /// coloured, this is a change of grammar.
    pub fn set_mode(&self, mode_name: &str, mode: MajorMode<B>) {
        self.modes_mut(|modes| {
            modes.insert(mode_name, mode);
            self.grammar_changed();
        });
    }

    /// Makes the mode NAME, empty; or, when there is one, keeps it with its
    /// keys, hooks and completion sources and starts its grammar and syntax
    /// table again. Returns whether it was there.
    ///
    /// Kept, because what other modules gave it -- electric-pair's hook, a
    /// key bound in `init.lisp` -- would otherwise go each time the mode's
    /// file is loaded again. The grammar and the table start again because
    /// the file builds them by adding to them, and loaded twice would add
    /// each rule twice; and, as for [`Self::set_mode`], what was coloured
    /// under the old grammar hears about it.
    pub(crate) fn make_mode(&self, name: &str) -> bool {
        self.modes_mut(|modes| {
            let existed = modes.edit(name, |mode| {
                mode.grammar = crate::modes::Grammar::default();
                mode.syntax_table = None;
            });
            if !existed {
                modes.insert(name, MajorMode::new(name));
            }
            self.grammar_changed();
            existed
        })
    }

    /// Makes NAME a minor mode, shown as LIGHTER, and returns whether it was
    /// there already; one that was keeps its keys and hooks.
    pub(crate) fn make_minor_mode(&self, name: &str, lighter: &str) -> bool {
        self.modes_mut(|modes| {
            let existed = modes.edit(name, |mode| mode.lighter = Some(lighter.to_string()));
            if !existed {
                let mut mode = MajorMode::new(name);
                mode.lighter = Some(lighter.to_string());
                modes.insert(name, mode);
            }
            existed
        })
    }

    /// Changes MODE's grammar with EDIT. False when there is no such mode.
    ///
    /// Every change of a grammar goes through here or [`Self::set_mode`],
    /// which move [`Self::grammar_epoch`] while the registry is still locked,
    /// so that nothing reads a grammar with the wrong edition, and emit
    /// `syntax-changed`.
    pub(crate) fn edit_grammar(&self, mode: &str, edit: impl FnOnce(&mut Grammar)) -> bool {
        self.modes_mut(|modes| {
            let found = modes.edit(mode, |found| edit(&mut found.grammar));
            if found {
                self.grammar_changed();
            }
            found
        })
    }

    /// Changes MODE's syntax table with EDIT, making one from the default when
    /// the mode has none. False when there is no such mode.
    ///
    /// Every change of a syntax table goes through here, and emits
    /// `syntax-changed`: what was scanned under the old table is out of date.
    /// The grammar epoch stays where it is, since no colouring reads the table.
    pub(crate) fn edit_syntax_table(
        &self,
        mode: &str,
        edit: impl FnOnce(&mut SyntaxTable),
    ) -> bool {
        self.modes_mut(|modes| {
            let found = modes.edit(mode, |found| {
                edit(found.syntax_table.get_or_insert_with(SyntaxTable::default))
            });
            if found {
                self.syntax_changed.emit();
            }
            found
        })
    }

    /// Returns which edition of the grammars is in force: how many times one
    /// has changed.
    pub(crate) fn grammar_epoch(&self) -> u64 {
        self.grammar_epoch.load(Ordering::Relaxed)
    }

    /// Moves the grammar epoch on, and emits `syntax-changed`. Called inside
    /// the modes' door, with the registry's write lock held, by the methods
    /// above: it takes no lock.
    fn grammar_changed(&self) {
        self.grammar_epoch.fetch_add(1, Ordering::Relaxed);
        self.syntax_changed.emit();
    }

    /// Returns MODE's syntax table, or the default one when it has none.
    pub(crate) fn syntax_table(&self, mode: &str) -> SyntaxTable {
        self.modes(|modes| modes.syntax_table(mode))
    }

    /// Returns the syntax table in force in the current buffer.
    pub(crate) fn current_syntax_table(&self) -> SyntaxTable {
        // The buffer's lock is given back before the registry's is taken.
        let mode = self.with_current_buffer(|buf| buf.current_mode.clone());
        self.syntax_table(&mode)
    }

    // ------------------------------------------------------------------
    // Hooks
    // ------------------------------------------------------------------

    /// Adds FUNCTION to the hook HOOK_NAME of every major mode.
    pub(crate) fn add_global_hook(&self, hook_name: &str, function: ELispExp<B>) {
        self.modes_mut(|modes| modes.add_global_hook(hook_name, function));
    }

    /// Runs the functions of the hook HOOK_NAME, the mode MODE_NAME's, then
    /// those of the minor modes on in the current buffer, then those of every
    /// mode, in the order they were added, each with no arguments, in a scope
    /// of [`Self::begin_command`]. One that fails is logged, and the others
    /// still run.
    pub(crate) fn run_hook(
        &self,
        mode_name: &str,
        hook_name: &str,
        env: &Arc<Env<EditorState<B>>>,
    ) {
        let minors = self.with_current_buffer(|buf| {
            buf.minor_modes
                .iter()
                .map(|on| on.name.clone())
                .collect::<Vec<_>>()
        });
        self.run_hook_of(mode_name, &minors, hook_name, env);
    }

    /// Runs the hook HOOK_NAME as [`Self::run_hook`] does, with MINORS for the
    /// minor modes: for a buffer no longer current, or no longer there.
    pub(crate) fn run_hook_of(
        &self,
        mode_name: &str,
        minors: &[String],
        hook_name: &str,
        env: &Arc<Env<EditorState<B>>>,
    ) {
        // Copied out, all of them in one acquisition, and the registry let go
        // before any of them runs: a hook may add a hook or define a key, which
        // needs the write lock this thread would otherwise be holding.
        let hooks = self.modes(|modes| modes.hooks_for(mode_name, minors, hook_name));

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

    // ------------------------------------------------------------------
    // The transient keymap, and the repeat keys
    // ------------------------------------------------------------------

    /// Installs MAP, consulted before every other keymap until it goes. See
    /// [`TransientKeymap`].
    pub(crate) fn set_transient_keymap(&self, map: TransientKeymap<B>) {
        self.modes_mut(|modes| modes.set_transient(map));
    }

    /// Takes the transient keymap down, if one is up: a command that ends one
    /// need not ask first.
    pub(crate) fn clear_transient_keymap(&self) {
        self.modes_mut(|modes| modes.clear_transient());
    }

    /// Returns what the transient keymap up is the question of, if anything.
    pub(crate) fn transient_owner(&self) -> Option<&'static str> {
        self.modes(|modes| modes.transient_owner())
    }

    /// Returns what the transient keymap wants shown, or an empty string when
    /// none is up.
    pub(crate) fn transient_message(&self) -> String {
        self.modes(|modes| modes.transient_message())
    }

    /// Returns whether a transient keymap is up. For the tests: the editor
    /// never asks, since a map that is up resolves keys and one that is not
    /// does not.
    #[cfg(test)]
    pub(crate) fn transient_keymap_active(&self) -> bool {
        self.modes(|modes| modes.transient_active())
    }

    /// Lets COMMAND be repeated by pressing KEY alone after it.
    pub(crate) fn set_repeat_key(&self, command: &str, key: KeyEvent) {
        self.modes_mut(|modes| modes.set_repeat_key(command, key));
    }

    /// Returns the key that repeats COMMAND, if it has one.
    pub(super) fn repeat_key(&self, command: &str) -> Option<KeyEvent> {
        self.modes(|modes| modes.repeat_key(command))
    }
}
