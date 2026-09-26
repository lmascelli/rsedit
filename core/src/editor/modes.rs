//! The major-mode half of the facade: defining modes, and asking what one says
//! about completion, file names and syntax.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    // -----------------------------------------------------------------------
    // Completion sources
    // -----------------------------------------------------------------------
    //
    // A list per mode and one global list, reached through the five methods
    // below so that no caller has to know there are two places. `nil` means
    // the global list everywhere, exactly as it does in `define-key`.
    /// Every completion source to try in MODE, most specific first.
    ///
    /// The mode's own sources come before the global ones because a mode knows
    /// something the editor does not: in a Lisp buffer the interpreter's
    /// function names are the good answer and the words lying around in other
    /// buffers are the fallback, and only `risp-mode` is in a position to say
    /// so.
    ///
    /// Both lists are copied out and both locks released before the caller
    /// gets them. Every one of these is about to be *called*, and a source is
    /// arbitrary Lisp that may load a module, define a mode, or open a buffer
    /// -- the same rule `run_hook` is written to, for the same reason.
    pub(crate) fn completion_sources(&self, mode: &str) -> Vec<ELispExp<B>> {
        self.modes(|modes| modes.completion_sources(mode))
    }

    /// Append a source to MODE's list, or to the global one when MODE is
    /// `None`. False if MODE names a mode that does not exist.
    pub(crate) fn add_completion_function(
        &self,
        mode: Option<&str>,
        function: ELispExp<B>,
    ) -> bool {
        self.modes_mut(|modes| modes.add_completion(mode, function))
    }

    /// Replace a whole list. This is how a source is removed or the order
    /// changed -- `(set-completion-functions nil (list ...))` -- which a
    /// bare `add` could not express.
    pub(crate) fn set_completion_functions(
        &self,
        mode: Option<&str>,
        functions: Vec<ELispExp<B>>,
    ) -> bool {
        self.modes_mut(|modes| modes.set_completions(mode, functions))
    }

    /// One list on its own, unmerged, or `None` if MODE is unknown.
    pub(crate) fn completion_function_list(&self, mode: Option<&str>) -> Option<Vec<ELispExp<B>>> {
        self.modes(|modes| modes.completion_list(mode))
    }

    /// Say that a file whose name matches PATTERN opens in MODE.
    ///
    /// Without this a language module can be loaded and never selected: nothing
    /// else maps a file to a mode, so every grammar would have to be reached by
    /// hand.
    pub(crate) fn add_auto_mode(&self, pattern: regex::Regex, mode: &str) {
        self.modes_mut(|modes| modes.add_auto_mode(pattern, mode));
    }

    /// The mode a file called PATH should open in, if any pattern claims it.
    ///
    /// Matched against the whole path, so a pattern can key on a directory as
    /// well as an extension.
    pub(crate) fn auto_mode_for(&self, path: &str) -> Option<String> {
        self.modes(|modes| modes.auto_mode_for(path))
    }

    pub fn set_mode(&self, mode_name: &str, mode: MajorMode<B>) {
        self.modes_mut(|modes| modes.insert(mode_name, mode));
    }

    //--------------------------------------------------------------------------
    //                         GETTERS AND SETTERS
    //--------------------------------------------------------------------------

    /// The syntax table for MODE, or the default one when it has none.
    pub(crate) fn syntax_table(&self, mode: &str) -> SyntaxTable {
        self.modes(|modes| modes.syntax_table(mode))
    }

    /// The syntax table in force in the current buffer.
    pub(crate) fn current_syntax_table(&self) -> SyntaxTable {
        // The buffer's lock is let go before the registry's is taken: two
        // compartments, one at a time.
        let mode = self.with_current_buffer(|buf| buf.current_mode.clone());
        self.syntax_table(&mode)
    }

    /// Register FUNCTION to run under HOOK_NAME in every major mode.
    pub(crate) fn add_global_hook(&self, hook_name: &str, function: ELispExp<B>) {
        self.modes_mut(|modes| modes.add_global_hook(hook_name, function));
    }
}
