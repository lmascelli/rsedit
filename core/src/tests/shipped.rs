//! What the editor ships, read from where it is defined: the modules the
//! default init.lisp loads, the manual pages in `man/`, and the names the
//! editor adds to its Lisp.
//!
//! Tests take these from here rather than keeping lists of their own, which
//! drift: two such lists missed four modules between them, and a third left
//! out a manual page.
use crate::buffer::gap_buffer::GapBuffer;
use crate::editor::EditorState;
use crate::editor::boot::DEFAULT_INIT_LISP;
use risp::{Env, LispExp, Parser, eval};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type Ctx = EditorState<GapBuffer>;

/// Returns the modules the default init.lisp loads, in its order.
pub(crate) fn default_modules() -> Vec<String> {
    DEFAULT_INIT_LISP
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("(eval-file \""))
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect()
}

/// Returns the source of the shipped module NAME.
///
/// # Panics
///
/// When `core/lisp/NAME.lisp` cannot be read.
pub(crate) fn module_source(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("lisp")
        .join(format!("{name}.lisp"));
    std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("{}: {why}", path.display()))
}

/// Loads every module the default init.lisp loads into ENV, in its order.
///
/// # Panics
///
/// When a module does not parse or fails to load, naming it. `eval-file`
/// would log the failure and go on, which is what makes it worth a test.
pub(crate) fn load_default_modules(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
    for name in default_modules() {
        let source = module_source(&name);
        let ast = Parser::new(&format!("(progn {source})"))
            .next()
            .unwrap_or_else(|why| panic!("{name}.lisp must parse: {why:?}"));
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("loading {name}.lisp: {why:?}"));
    }
}

/// Returns the directory of the manual pages, `man/` at the project root.
pub(crate) fn man_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("man")
}

/// Returns every manual page as (name, text), sorted by name: the `.txt`
/// files of `man/`, which are what the build installs.
pub(crate) fn man_pages() -> Vec<(String, String)> {
    let dir = man_directory();
    let mut pages: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|why| panic!("{}: {why}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .map(|path| {
            let name = path
                .file_stem()
                .expect("a page has a name")
                .to_string_lossy()
                .into_owned();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
            (name, text)
        })
        .collect();
    pages.sort();
    pages
}

/// Returns the functions and macros ENV has that the bare Lisp does not:
/// the editor's primitives and what its modules define.
pub(crate) fn editor_functions(env: &Arc<Env<Ctx>>) -> Vec<String> {
    let base = Env::<Ctx>::new_root();
    risp::setup_base_env(base.clone());
    let lisp: std::collections::BTreeSet<String> = base
        .function_names()
        .into_iter()
        .chain(base.macro_names())
        .collect();
    let mut names: Vec<String> = env
        .function_names()
        .into_iter()
        .chain(env.macro_names())
        .filter(|name| !lisp.contains(name))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Returns the variables the default modules define with `defvar` or
/// `defconst`, as (module, name).
pub(crate) fn module_variables() -> Vec<(String, String)> {
    let mut defined = Vec::new();
    for module in default_modules() {
        for line in module_source(&module).lines() {
            let line = line.trim_start();
            for opener in ["(defvar ", "(defconst "] {
                if let Some(rest) = line.strip_prefix(opener) {
                    let name: String = rest
                        .chars()
                        .take_while(|c| !c.is_whitespace() && *c != ')')
                        .collect();
                    if !name.is_empty() {
                        defined.push((module.clone(), name));
                    }
                }
            }
        }
    }
    defined
}

/// Whether NAME is private by convention: a `--` in it.
pub(crate) fn is_private(name: &str) -> bool {
    name.contains("--")
}

/// Returns the names in one of the lists in `doc_exceptions/`: one per line,
/// with blank lines and `#` comments skipped.
pub(crate) fn exceptions(list: &str) -> Vec<String> {
    list.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Returns the names TEXT quotes the way Emacs does, `name': a Lisp symbol
/// of two characters or more, starting with a lower-case letter or `*`.
///
/// Keys (`C-g', `q') and forms (`(yield)') are not names and are skipped.
pub(crate) fn quoted_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (index, _) in text.match_indices('`') {
        let after = &text[index + 1..];
        let Some(end) = after.find(|c: char| c == '\'' || c == '`' || c.is_whitespace()) else {
            continue;
        };
        let name = &after[..end];
        let symbol = name.chars().count() >= 2
            && name.starts_with(|c: char| c.is_ascii_lowercase() || c == '*')
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "*+!?<>=/-".contains(c));
        if after[end..].starts_with('\'') && symbol {
            names.push(name.to_string());
        }
    }
    names
}

/// Returns every name the editor in ENV can be asked about: its functions,
/// macros and variables, and the manual pages, modules, faces, themes and
/// scheduler options by name.
pub(crate) fn known_names(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> BTreeSet<String> {
    let mut known: BTreeSet<String> = env
        .function_names()
        .into_iter()
        .chain(env.macro_names())
        .chain(env.variable_names())
        .collect();
    known.extend(man_pages().into_iter().map(|(name, _)| name));
    known.extend(default_modules());
    let listed = |src: &str| -> Vec<String> {
        let ast = Parser::new(src).next().expect("the form must parse");
        let answer = eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("{src}: {why:?}"));
        answer
            .iter()
            .map(|item| match item {
                LispExp::String(text) => text.to_string(),
                LispExp::Symbol(name) => name.to_string(),
                other => format!("{other:?}"),
            })
            .collect()
    };
    known.extend(listed("(list-faces)"));
    known.extend(listed("(available-themes)"));
    // A property list: the names are every other item.
    known.extend(listed("(work-options)").into_iter().step_by(2));
    known
}
