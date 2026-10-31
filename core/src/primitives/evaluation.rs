//! Evaluating Lisp at the top level of the environment: a file, with
//! `eval-file`, and a string, with `eval-string-top-level`.
//!
//! Design: `doc/architettura-editor.typ, <valutazione>`.
use super::*;

pub const EVAL_FILE_DOC: &str = "(eval-file FILE): Evaluate the Lisp in FILE, and return a \
         one-element list holding the value of its last form, or nil when there is nothing to \
         evaluate.\n\n\
         FILE is a path, relative to the working directory. When no such file \
         exists and FILE has no directory, extension or dot in it, FILE.lisp is looked for in \
         each directory of `lisp-path' in turn. A file that is not found is logged; a file that \
         does not parse is skipped. The file is evaluated as init.lisp is, outside any function, \
         so what it defines lasts wherever `eval-file' is called from.\n\n\
         Example:\n\
         (eval-file \"dired\")                          ; dired.lisp, from lisp-path\n\
         (eval-file (expand-file-name \"~/lisp/mine.lisp\"))";

primitive!(eval_file, args, env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else if let Some(ELispExp::String(file)) = args.first() {
        Ok(ctx.eval_file(file, env.clone())?)
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        })
    }
});

pub const EVAL_STRING_TOP_LEVEL_DOC: &str = "(eval-string-top-level STRING): Evaluate the \
         expression in STRING as init.lisp would, outside any function, and return (t VALUE), or \
         (nil WHY) when it does not parse or fails. A defun, a defvar or the setq of a new name \
         lasts, where `eval-string-safe' called inside a function leaves it to that function. \
         M-: evaluates what is typed with it.\n\n\
         Example:\n\
         (eval-string-top-level \"(defun twice (n) (* 2 n))\") => (t twice)";

primitive!(eval_string_top_level, args, env, ctx, {
    exact_arity(args, 1)?;
    let ELispExp::String(source) = &args[0] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[0].clone(),
        });
    };
    // The frames a failed evaluation leaves pushed go with the failure, which
    // is caught here rather than raised: as `eval-string-safe` does.
    let depth = ctx.call_frame_depth();
    let result = match risp::Parser::new(source).next() {
        Ok(ast) => risp::eval(&ast, crate::editor::boot::top_level(&env), ctx),
        Err(why) => Err(EvalError::RuntimeMessage(format!("Parser Error {why:?}"))),
    };
    ctx.truncate_call_frames(depth);
    Ok(match result {
        Ok(value) => ELispExp::proper_list(vec![ELispExp::t(), value]),
        Err(why) => {
            ELispExp::proper_list(vec![ELispExp::nil(), ELispExp::string(format!("{why:?}"))])
        }
    })
});

/// Registers the primitives of evaluation.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("eval-file", eval_file, EVAL_FILE_DOC);
    into.function(
        "eval-string-top-level",
        eval_string_top_level,
        EVAL_STRING_TOP_LEVEL_DOC,
    );
}
