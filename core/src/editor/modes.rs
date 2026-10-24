//! The modes' half of the facade: changing a buffer's major mode, changing a
//! grammar or a syntax table and saying so to the jobs, running hooks, and the
//! completion sources, file-name patterns and keymaps the modes hold.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Makes MODE the major mode of the buffer NAME, keeping what the buffer
    /// holds, and runs the mode's hooks when that is a change -- once the
    /// buffer's lock has been given back.
    pub(crate) fn change_major_mode(&self, name: &str, mode: &str, env: &Arc<Env<EditorState<B>>>) {
        let changed = self.with_buffer_mut(name, |buf| {
            if buf.current_mode == mode {
                return false;
            }
            buf.current_mode = mode.to_string();
            buf.syntax.invalidate_from(buf.version, 0);
            buf.scan.invalidate_from(buf.version, 0);
            // The text is the same, but what it means is not: everything
            // worked out about it is to be worked out again.
            buf.changed.emit();
            true
        });
        if changed == Some(true) {
            self.run_buffer_hook(name, "major-mode-hook", env);
            self.run_buffer_hook(name, "after-change-major-mode-hook", env);
        }
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

    /// Runs the functions of the hook HOOK_NAME, the mode MODE_NAME's and then
    /// those of every mode, in the order they were added, each with no
    /// arguments, in a scope of [`Self::begin_command`]. One that fails is
    /// logged, and the others still run.
    pub(crate) fn run_hook(
        &self,
        mode_name: &str,
        hook_name: &str,
        env: &Arc<Env<EditorState<B>>>,
    ) {
        // Copied out, both halves in one acquisition, and the registry let go
        // before any of them runs: a hook may add a hook or define a key, which
        // needs the write lock this thread would otherwise be holding.
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
