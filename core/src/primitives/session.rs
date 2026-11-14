//! Values kept by name from one run to the next: `set-session-value` writes
//! one as the text `read` turns back into it, and `session-value` reads it
//! back without evaluating it.
//!
//! Design: `doc/architettura-editor.typ, <memoria>`.
use super::*;
use crate::primitives::args;
use risp::{Parser, ParserError, Unreadable, write_readable};

/// Charges one fuel unit for each byte of TEXT, as `read` and
/// `prin1-to-string` do.
fn charge<B: BufferTrait>(
    ctx: &EditorState<B>,
    text: &str,
) -> Result<(), EvalError<EditorState<B>>> {
    ctx.consume_fuel(u32::try_from(text.len()).unwrap_or(u32::MAX))
}

/// Returns the one value written in TEXT, unevaluated, as `read` reads it.
fn read_one<B: BufferTrait>(text: &str) -> Result<ELispExp<B>, ParserError> {
    let mut parser = Parser::for_data(text);
    let value = parser.next::<EditorState<B>>()?;
    match parser.next::<EditorState<B>>() {
        Err(ParserError::VoidExp) => Ok(value),
        Ok(_) => Err(ParserError::UnexpectedToken("a second value".into())),
        Err(why) => Err(why),
    }
}

pub const SESSION_VALUE_DOC: &str = "(session-value NAME &optional DEFAULT): Return the value \
         last kept under NAME, a symbol or a string, with `set-session-value', in this run or \
         an earlier one; DEFAULT when there is none. The value is read back, not evaluated. \
         Until the session is read, after \
         init.lisp, a value of an earlier run is not there yet.\n\n\
         Example:\n\
         (session-value 'my-notes-sort 'name)";

primitive!(session_value, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: if args.is_empty() { 1 } else { 2 },
            got: args.len(),
        });
    }
    let name = args::name(args.first())?;
    let default = args.get(1).cloned().unwrap_or_else(ELispExp::nil);
    let Some(text) = ctx.kept_values(|kept| kept.get(&name)) else {
        return Ok(default);
    };
    charge(ctx, &text)?;
    match read_one::<B>(&text) {
        Ok(value) => Ok(value),
        Err(why) => {
            ctx.log_diagnostic(&format!(
                "[WARNING] The value kept under {name} does not read ({why:?}): {text}"
            ));
            Ok(default)
        }
    }
});

pub const SET_SESSION_VALUE_DOC: &str = "(set-session-value NAME VALUE): Keep VALUE under \
         NAME, a symbol or a string, for `session-value' to return in this run and the next \
         ones, and return VALUE. nil forgets NAME. VALUE is written as `prin1-to-string' \
         writes it, at once: a part with no read syntax, such as a function, signals \
         wrong-type-argument with data (readablep PART). Prefix NAME with the module's name.\n\n\
         Example:\n\
         (set-session-value 'my-notes-sort 'date)";

primitive!(set_session_value, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let name = args::name(args.first())?;
    let value = &args[1];
    if value.is_nil() {
        ctx.kept_values_mut(|kept| kept.forget(&name));
        return Ok(ELispExp::nil());
    }
    let text = write_readable(value).map_err(|why| {
        let offending = match why {
            Unreadable::Value(part) => part,
            Unreadable::TooDeep => value.clone(),
        };
        EvalError::Signal {
            symbol: ELispExp::symbol("wrong-type-argument".into()),
            data: ELispExp::proper_list(vec![ELispExp::symbol("readablep".into()), offending]),
        }
    })?;
    charge(ctx, &text)?;
    ctx.kept_values_mut(|kept| kept.set(&name, text));
    Ok(value.clone())
});

/// Registers the primitives that keep values by name in the session.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("session-value", session_value, SESSION_VALUE_DOC);
    into.function(
        "set-session-value",
        set_session_value,
        SET_SESSION_VALUE_DOC,
    );
}
