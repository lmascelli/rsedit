//! The editor's primitives: the names the editor gives Lisp, in a file per
//! concept, each registering its own into a [`Registry`].
//!
//! Design: `doc/architettura-editor.typ, <primitive>`.
use crate::{
    BufferTrait, ELispExp, EditorState,
    input::{KeyEvent, Keymap, OnUnbound, TransientKeymap, parse_key, parse_key_sequence},
    modes::{CommentStyle, MajorMode, SyntaxClass, SyntaxRegion, SyntaxRule, SyntaxTable},
    ui::Face,
};

use risp::{Env, EvalError, LispContext, LispPrimitive, exact_arity};

/// Defines a primitive: a public function with the signature
/// `LispPrimitive` wants, its arguments, environment and editor bound to the
/// three names given.
#[macro_export]
macro_rules! primitive {
    ($func_name:ident, $args:ident, $env:ident, $ctx:ident, $body:block) => {
        pub fn $func_name<B: BufferTrait>(
            $args: &[ELispExp<B>],
            $env: std::sync::Arc<Env<EditorState<B>>>,
            $ctx: &EditorState<B>,
        ) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
            $body
        }
    };
}

pub(crate) mod args;
mod ask;
mod buffers;
mod commands;
mod comments;
mod completion;
mod diagnostics;
pub(crate) mod edits;
mod environment;
mod evaluation;
pub(crate) mod file_names;
mod files;
pub(crate) mod filesystem;
mod fuzzy;
mod grammar;
mod help;
mod indentation;
mod isearch;
mod keymaps;
mod killing;
pub(crate) mod lisp_job;
mod macros;
mod match_data;
mod minibuffer;
mod modes;
pub(crate) mod mouse;
pub(crate) mod movement;
pub(crate) mod occur;
mod overlays;
mod quit;
mod rectangle;
mod region;
mod replace;
pub(crate) mod results;
mod scan;
mod sexp;
mod shell;
mod syntax_table;
mod theme;
mod undo;
mod verbs;
mod virtual_text;
mod windows;
mod workers;

/// What a primitives module registers into: each module's `install` is
/// handed one, and registers its own primitives with it.
pub(crate) struct Registry<'a, B: BufferTrait> {
    state: &'a EditorState<B>,
    env: &'a std::sync::Arc<Env<EditorState<B>>>,
}

impl<B: BufferTrait> Registry<'_, B> {
    /// Binds NAME to POINTER, a primitive, with DOC as its docstring.
    ///
    /// DOC begins with the call, `(name ARGS): what it does`: a primitive has
    /// no parameter list the help commands could read, so that line is its
    /// signature. `doc_faithfulness_tests` checks it on every primitive.
    pub(crate) fn function(
        &self,
        name: &str,
        pointer: LispPrimitive<EditorState<B>>,
        doc: &'static str,
    ) {
        self.env
            .set_function(name.into(), ELispExp::primitive(pointer, Some(doc.into())));
    }

    /// Binds NAME as [`Self::function`] does, and registers it as a command
    /// that collects the arguments SPECS. They are parsed here, at boot: a
    /// spec that does not parse stops the editor with a panic.
    pub(crate) fn command(
        &self,
        name: &str,
        pointer: LispPrimitive<EditorState<B>>,
        specs: &[&str],
        doc: &'static str,
    ) {
        self.function(name, pointer, doc);
        self.state.register_command(
            name,
            specs
                .iter()
                .map(|code| {
                    crate::commands::ArgSpec::parse(code)
                        .unwrap_or_else(|why| panic!("built-in command `{name}`: {why}"))
                })
                .collect(),
        );
    }
}

/// Installs every primitive the editor provides, through each module's
/// `install`. In alphabetical order: no name is registered twice, so the
/// order changes nothing, and in this one a missing module shows.
pub fn install_primitives<B: BufferTrait>(
    state: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
) {
    let into = Registry { state, env };
    ask::install(&into);
    buffers::install(&into);
    commands::install(&into);
    comments::install(&into);
    completion::install(&into);
    diagnostics::install(&into);
    edits::install(&into);
    environment::install(&into);
    evaluation::install(&into);
    file_names::install(&into);
    files::install(&into);
    filesystem::install(&into);
    fuzzy::install(&into);
    grammar::install(&into);
    help::install(&into);
    indentation::install(&into);
    isearch::install(&into);
    keymaps::install(&into);
    killing::install(&into);
    macros::install(&into);
    match_data::install(&into);
    minibuffer::install(&into);
    modes::install(&into);
    mouse::install(&into);
    movement::install(&into);
    occur::install(&into);
    overlays::install(&into);
    quit::install(&into);
    rectangle::install(&into);
    region::install(&into);
    replace::install(&into);
    results::install(&into);
    scan::install(&into);
    sexp::install(&into);
    shell::install(&into);
    syntax_table::install(&into);
    theme::install(&into);
    undo::install(&into);
    verbs::install(&into);
    virtual_text::install(&into);
    windows::install(&into);
    workers::install(&into);
}
