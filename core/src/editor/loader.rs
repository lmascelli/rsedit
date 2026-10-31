//! The loader's half of the facade: finding a module in `lisp-path`,
//! evaluating a file, `require` and `provide`, calling what waited for a
//! feature, and finding the plugins.
//!
//! Design: `doc/architettura-editor.typ, <moduli-lisp>`.

use super::boot::top_level;
use super::*;
use crate::managers::Plugin;
use std::path::Path;

/// The directories the editor's modules are kept in, by kind, in the order
/// `lisp-path` lists them: `data/lisp/base` and the others beside the
/// executable, `core/lisp/base` and the others in the source.
pub(crate) const MODULE_KINDS: [&str; 3] = ["base", "features", "languages"];

/// The file in a plugin's directory that says what the plugin is.
pub(crate) const PLUGIN_DESCRIPTOR: &str = "plugin.lisp";

/// Returns whether NAME names a module rather than a file: no `/`, `\` or `.`
/// in it, so that it is looked for in `lisp-path` as NAME.lisp.
fn is_bare(name: &str) -> bool {
    !name.contains(['/', '\\', '.'])
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns the directories of `lisp-path`, read from ENV: none when it is
    /// unbound, and `None` when it is not a list. An element that is not a
    /// string is logged and skipped.
    fn lisp_path(&self, env: &Arc<Env<Self>>) -> Option<Vec<String>> {
        let mut dirs = vec![];
        if let Some(list) = env.get_variable(LISP_PATH) {
            if !matches!(list, ELispExp::Cons(_)) {
                self.log_diagnostic("Variable lisp-path is not a list of paths");
                return None;
            }
            for item in list.iter() {
                if let ELispExp::String(path) = &item {
                    dirs.push(path.to_string());
                } else {
                    self.log_diagnostic(&format!("Element in lisp-path is not a path {item:?}"));
                }
            }
        }
        Some(dirs)
    }

    /// Returns NAME.lisp in the first of DIRS that has it, or `None`.
    ///
    /// One in a later directory as well is shadowed, and the log says so:
    /// a file of the person's own in place of one the editor ships is what
    /// they meant, but a stale copy is not, and only the log can tell them.
    fn find_module(&self, name: &str, dirs: &[String]) -> Option<PathBuf> {
        let mut found = dirs
            .iter()
            .map(|dir| Path::new(dir).join(format!("{name}.lisp")))
            .filter(|path| path.is_file());
        let first = found.next()?;
        if let Some(shadowed) = found.next() {
            self.log_diagnostic(&format!(
                "[INFO] {} shadows {}",
                first.display(),
                shadowed.display()
            ));
        }
        Some(first)
    }

    /// Evaluates the Lisp file FILE, and returns a list holding the value of
    /// its last form, or the first error its evaluation raised.
    ///
    /// FILE is a path, absolute or relative; when there is no such file and
    /// FILE is a bare name, with no `/`, `\` or `.`, it is `FILE.lisp` in the
    /// first directory of `lisp-path`, read from ENV, that has one. A file not
    /// found, or not read, is logged, and the answer is nil. The file is
    /// evaluated at the root of ENV's chain, as `top_level` finds it, wherever
    /// the call comes from: a module loaded inside a function, or with `M-:`,
    /// defines what `init.lisp` would. It is evaluated whether or not it has
    /// been before: `require` is what loads once.
    pub fn eval_file(
        &self,
        file: &str,
        env: Arc<Env<Self>>,
    ) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
        // First, FILE as a path, read in the working directory as any name
        // the user writes.
        let as_path = self.expand_path(file);
        let (path, content) = match std::fs::read_to_string(&as_path) {
            Ok(content) => (PathBuf::from(as_path), content),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // A name with a separator or a dot was meant as a path; a bare
                // one is looked for in `lisp-path`.
                if !is_bare(file) {
                    self.log_diagnostic(&format!(
                        "[ERROR] eval_file {file} is not a valid script name"
                    ));
                    return Ok(ELispExp::nil());
                }
                let Some(dirs) = self.lisp_path(&env) else {
                    return Ok(ELispExp::nil());
                };
                match self
                    .find_module(file, &dirs)
                    .and_then(|path| Some((path.clone(), std::fs::read_to_string(path).ok()?)))
                {
                    Some(found) => found,
                    None => {
                        self.log_diagnostic(&format!(
                            "[ERROR] eval_file {file}.lisp was not found in lisp_path"
                        ));
                        return Ok(ELispExp::nil());
                    }
                }
            }
            Err(err) => {
                self.log_diagnostic(&format!("[ERROR] Failed eval file {file} {:?}", err));
                return Ok(ELispExp::nil());
            }
        };
        // TODO(AVV-2): read as one form, so that a closing parenthesis too
        // many or too few evaluates nothing, and nothing says so.
        let Ok(ast) = Parser::new(&format!("(progn {content})")).next() else {
            return Ok(ELispExp::nil());
        };
        self.eval_source(&path, &ast, &env)
    }

    /// Evaluates AST, the forms of the file PATH, at the root of ENV's chain,
    /// and returns a list holding the value of the last.
    ///
    /// PATH is what a `provide` in it is recorded as coming from.
    fn eval_source(
        &self,
        path: &Path,
        ast: &ELispExp<B>,
        env: &Arc<Env<Self>>,
    ) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
        let _command = self.begin_command();
        self.loader_mut(|loader| loader.enter_file(path.to_path_buf()));
        let value = eval(ast, top_level(env), self);
        self.loader_mut(|loader| loader.leave_file());
        Ok(ELispExp::proper_list(vec![value?]))
    }

    /// Loads the feature NAME unless it is loaded already, and returns NAME as
    /// a symbol; or nil, logged, when there is no file to load it from.
    ///
    /// The file is FILE when given, a path or a bare name, and NAME otherwise;
    /// a bare name is NAME.lisp in the first directory of `lisp-path` that
    /// has one, as for [`Self::eval_file`]. It is evaluated as `eval_file`
    /// evaluates, and must `provide` NAME.
    ///
    /// # Errors
    ///
    /// When NAME is being required already, further out: a cycle. When the
    /// file cannot be read or does not parse, raises an error, which is
    /// returned, or did not provide NAME. A failure is recorded against NAME, and the features first
    /// provided since the `require` began are in force no longer.
    pub(crate) fn require(
        &self,
        name: &str,
        file: Option<&str>,
        env: &Arc<Env<Self>>,
    ) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
        if self.loader(|loader| loader.is_loaded(name)) {
            return Ok(ELispExp::symbol(name.into()));
        }
        let wanted = file.unwrap_or(name);
        let path = if is_bare(wanted) {
            self.lisp_path(env)
                .and_then(|dirs| self.find_module(wanted, &dirs))
        } else {
            Some(PathBuf::from(wanted)).filter(|path| path.is_file())
        };
        let Some(path) = path else {
            self.log_diagnostic(&format!(
                "[ERROR] require {name}: no {wanted}{} in lisp-path",
                if is_bare(wanted) { ".lisp" } else { "" }
            ));
            return Ok(ELispExp::nil());
        };
        let mark = self
            .loader_mut(|loader| loader.begin_require(name, path.clone()))
            .map_err(|chain| {
                EvalError::RuntimeMessage(format!("Recursive require: {}", chain.join(" -> ")))
            })?;
        // Unlike `eval_file`, a file that does not parse is an error: it
        // provides nothing, and saying only that would hide why.
        let outcome = std::fs::read_to_string(&path)
            .map_err(|why| format!("Cannot read {}: {why}", path.display()))
            .and_then(|content| {
                Parser::new(&format!("(progn {content})"))
                    .next()
                    .map_err(|why| format!("{} does not parse: {why:?}", path.display()))
            })
            .map_err(EvalError::RuntimeMessage)
            .and_then(|ast| self.eval_source(&path, &ast, env).map(|_| ()))
            .and_then(|()| {
                if self.loader(|loader| loader.is_loaded(name)) {
                    Ok(())
                } else {
                    Err(EvalError::RuntimeMessage(format!(
                        "Loading {} did not provide {name}",
                        path.display()
                    )))
                }
            });
        let failure = outcome.as_ref().err().map(|why| format!("{why:?}"));
        if let Some(why) = &failure {
            self.log_diagnostic(&format!("[ERROR] require {name}: {why}"));
        }
        self.loader_mut(|loader| loader.end_require(name, mark, failure));
        outcome.map(|()| ELispExp::symbol(name.into()))
    }

    /// Records the feature NAME as provided, from the file being evaluated,
    /// and calls what was waiting for it, in the order it was added, with no
    /// arguments. One that fails is logged, and the others are still called.
    pub(crate) fn provide(&self, name: &str, env: &Arc<Env<Self>>) {
        let waiting = self.loader_mut(|loader| loader.provide(name));
        for function in waiting {
            if let Err(why) = call_callable(&function, &[], top_level(env), self) {
                self.log_diagnostic(&format!("[ERROR] After loading {name}: {why:?}"));
            }
        }
    }

    /// Calls FUNCTION, with no arguments, once the feature NAME is provided,
    /// and returns nil; or, when it is already, calls it now and returns its
    /// value.
    pub(crate) fn eval_after_load(
        &self,
        name: &str,
        function: ELispExp<B>,
        env: &Arc<Env<Self>>,
    ) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
        match self.loader_mut(|loader| loader.after_load(name, function)) {
            Some(function) => call_callable(&function, &[], top_level(env), self),
            None => Ok(ELispExp::nil()),
        }
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns the plugins in ROOT, sorted by name: each a directory of it
    /// with a [`PLUGIN_DESCRIPTOR`] that reads as one. A directory passed
    /// over is logged, with why.
    ///
    /// The descriptor is read as data, never evaluated: what it says is known
    /// without running anything of the plugin's.
    pub(crate) fn find_plugins(&self, root: &Path) -> Vec<Plugin> {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Vec::new();
        };
        let mut plugins = Vec::new();
        for entry in entries.flatten() {
            let directory = entry.path();
            if !directory.is_dir() {
                continue;
            }
            let descriptor = directory.join(PLUGIN_DESCRIPTOR);
            let read = std::fs::read_to_string(&descriptor)
                .map_err(|why| why.to_string())
                .and_then(|text| read_descriptor::<B>(&text, &directory));
            match read {
                Ok(plugin) => plugins.push(plugin),
                Err(why) => self.log_diagnostic(&format!(
                    "[ERROR] Plugin {} passed over: {why}",
                    directory.display()
                )),
            }
        }
        plugins.sort_by(|a, b| a.name.cmp(&b.name));
        plugins
    }
}

/// Returns the plugin TEXT describes, a descriptor found in DIRECTORY:
/// `(define-plugin NAME :version "0.1" :requires (dired) :description "…")`,
/// NAME the directory's own name, and each keyword optional.
fn read_descriptor<B: BufferTrait>(text: &str, directory: &Path) -> Result<Plugin, String> {
    let form: ELispExp<B> = Parser::new(text)
        .next()
        .map_err(|why| format!("{PLUGIN_DESCRIPTOR} does not parse: {why:?}"))?;
    // A list as read is a `Form`, not a chain of conses: `args::list` reads
    // both.
    let items = crate::primitives::args::list(&form);
    let symbol = |exp: &ELispExp<B>| match exp {
        ELispExp::Symbol(name) => Some(name.to_string()),
        _ => None,
    };
    let string = |exp: &ELispExp<B>| match exp {
        ELispExp::String(text) => Some(text.to_string()),
        _ => None,
    };
    if items.first().and_then(symbol).as_deref() != Some("define-plugin") {
        return Err(format!(
            "{PLUGIN_DESCRIPTOR} is not a (define-plugin NAME ...)"
        ));
    }
    let name = items
        .get(1)
        .and_then(symbol)
        .ok_or_else(|| "define-plugin has no NAME".to_string())?;
    let own = directory
        .file_name()
        .map(|own| own.to_string_lossy().into_owned());
    if own.as_deref() != Some(name.as_str()) {
        return Err(format!(
            "it calls itself {name}, in a directory named otherwise"
        ));
    }
    let mut plugin = Plugin {
        name,
        version: None,
        requires: Vec::new(),
        description: None,
        directory: directory.to_path_buf(),
    };
    let mut rest = items[2..].iter();
    while let Some(key) = rest.next() {
        let value = rest.next().ok_or_else(|| format!("{key:?} has no value"))?;
        match symbol(key).as_deref() {
            Some(":version") => plugin.version = string(value),
            Some(":description") => plugin.description = string(value),
            Some(":requires") => {
                plugin.requires = crate::primitives::args::list(value)
                    .iter()
                    .filter_map(symbol)
                    .collect()
            }
            // Unknown keys are left for what a later editor may read.
            _ => {}
        }
    }
    Ok(plugin)
}
