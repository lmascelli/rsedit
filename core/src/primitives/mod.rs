use crate::{
    BufferTrait, ELispExp, EditorState,
    input::{KeyEvent, Keymap, OnUnbound, TransientKeymap, parse_key, parse_key_sequence},
    modes::{CommentStyle, MajorMode, SyntaxClass, SyntaxRegion, SyntaxRule, SyntaxTable},
    ui::Face,
};

use risp::{Env, EvalError, LispContext, LispPrimitive, exact_arity};

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
pub(crate) mod edits;
mod general;
mod help;
mod indentation;
pub(crate) mod io;
mod isearch;
mod killing;
pub(crate) mod lisp_job;
mod macros;
mod match_data;
mod minibuffer;
mod modes;
pub(crate) mod mouse;
pub(crate) mod movement;
mod overlays;
mod rectangle;
mod region;
mod replace;
pub(crate) mod results;
mod scan;
mod sexp;
mod shell;
mod theme;
mod ui;
mod undo;
mod verbs;
mod virtual_text;
mod windows;
mod workers;

/// What a primitives module registers into.
///
/// A value, where this used to be two `macro_rules!` written *inside*
/// `install_primitives`. That is why all 277 registrations lived in that one
/// function: a macro declared in a function body can only be called from that
/// body, so every module's registrations had to be written there, two files
/// away from the code they name. A value can be handed to twenty-five modules,
/// and each one registers its own.
pub(crate) struct Registry<'a, B: BufferTrait> {
    state: &'a EditorState<B>,
    env: &'a std::sync::Arc<Env<EditorState<B>>>,
}

impl<B: BufferTrait> Registry<'_, B> {
    /// Bind NAME to a primitive Lisp can call.
    ///
    /// DOC must begin with the primitive's own call -- `(name ARGS): what it
    /// does` -- because a primitive has no parameter list anything can read,
    /// so that first line *is* the signature the help commands show.
    /// `tests::doc_faithfulness_tests` checks that over every primitive there
    /// is, along with the docstring simply being there.
    pub(crate) fn function(
        &self,
        name: &str,
        pointer: LispPrimitive<EditorState<B>>,
        doc: &'static str,
    ) {
        self.env
            .set_function(name.into(), ELispExp::primitive(pointer, Some(doc.into())));
    }

    /// Bind NAME, and register it as a command `M-x` can reach.
    ///
    /// One call rather than two so that a built-in command's argument spec
    /// sits next to its implementation and its docstring, rather than in a
    /// `.lisp` file that could drift or fail to load. User-defined commands
    /// take the other route, `defcommand`, which expands to a `defun` plus a
    /// `register-command` call; both end up in the same registry, and neither
    /// knows about the other.
    ///
    /// SPECS are parsed here, at boot, so a malformed one is a startup panic
    /// rather than a surprise the first time somebody runs the command.
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

/// Install every primitive the editor provides.
///
/// Each module registers its own, through [`Registry`]; this is the list of
/// modules and nothing else. Alphabetical, because the order genuinely does
/// not matter -- each registration is an insertion into a map under a name no
/// other registration uses -- so alphabetical is the order that makes a
/// missing module visible.
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
    edits::install(&into);
    general::install(&into);
    help::install(&into);
    indentation::install(&into);
    io::install(&into);
    isearch::install(&into);
    killing::install(&into);
    macros::install(&into);
    match_data::install(&into);
    minibuffer::install(&into);
    modes::install(&into);
    mouse::install(&into);
    movement::install(&into);
    overlays::install(&into);
    rectangle::install(&into);
    region::install(&into);
    replace::install(&into);
    results::install(&into);
    scan::install(&into);
    sexp::install(&into);
    shell::install(&into);
    theme::install(&into);
    ui::install(&into);
    undo::install(&into);
    verbs::install(&into);
    virtual_text::install(&into);
    windows::install(&into);
    workers::install(&into);
}
