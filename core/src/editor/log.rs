//! Diagnostics: the log and its file, the backtrace of the last error, and how
//! an uncaught error reaches the user.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Starts copying the log to a new file at PATH, first writing out what has
    /// been logged so far. What is logged afterwards reaches the file through
    /// the WorkManager's watchdog, a turn at a time, and [`Self::shutdown`].
    ///
    /// The file is under the log's lock, so every clone of the editor logs to
    /// it, whichever enabled it.
    pub fn enable_log_file<P: AsRef<std::path::Path>>(&self, path: P) -> std::io::Result<()> {
        // TODO(uncertain) maybe this is an unwanted change, i don't know if
        // it's better to be able to enable the writing of the logs only at
        // specific times and maybe disable it to get only some logs.
        // TODO(AVV-6): `File::create` empties the file, even while another
        // editor is writing to it.
        let file = File::create(path)?;
        self.log_mut(|log| log.enable_file(file))
    }

    /// Returns every line logged so far, oldest first.
    pub fn get_logs(&self) -> Vec<String> {
        self.log(|log| log.lines())
    }

    /// Returns the backtrace of the last uncaught error as a suffix,
    /// `" | backtrace: a -> b -> c"`, innermost call first, or an empty string
    /// when there is none; and clears it either way, so that the next error
    /// starts from a clean stack.
    pub fn take_backtrace_suffix(&self) -> String {
        let frames = self.backtrace();
        self.clear_backtrace();
        if frames.is_empty() {
            String::new()
        } else {
            format!(" | backtrace: {}", frames.join(" -> "))
        }
    }

    /// Reports an uncaught evaluation error to the user, and counts it as a
    /// failed command.
    ///
    /// When Lisp defines `report-error`, as `debug.lisp` does, it is called
    /// with MESSAGE and the backtrace, innermost call first, and decides how to
    /// show them: the one shipped echoes the message, and opens a
    /// `*Backtrace*` window when `debug-on-error` is set. Before it exists,
    /// early in startup, the error is logged, with the suffix
    /// [`Self::take_backtrace_suffix`] gives.
    pub fn report_error(&self, message: &str, env: &Arc<Env<Self>>) {
        // Counted first, so that a `report-error` that fails itself cannot keep
        // a replaying macro from noticing the failure.
        self.note_command_error();
        let frames = self.backtrace();
        self.clear_backtrace();

        if env.get_function("report-error").is_some() {
            let call_ast = ELispExp::form(vec![
                ELispExp::symbol("report-error".into()),
                ELispExp::string(message.to_string()),
                ELispExp::form(vec![
                    ELispExp::symbol("quote".into()),
                    ELispExp::proper_list(frames.into_iter().map(ELispExp::string).collect()),
                ]),
            ]);
            let _command = self.begin_command();
            if let Err(e) = eval(&call_ast, env.clone(), self) {
                self.log_diagnostic(&format!("[ERROR] report-error hook itself failed: {:?}", e));
            }
        } else {
            let suffix = if frames.is_empty() {
                String::new()
            } else {
                format!(" | backtrace: {}", frames.join(" -> "))
            };
            self.log_diagnostic(&format!("Eval Error: {message}{suffix}"));
        }
    }
}
