//! How a mode's text looks: the rules and regions of its grammar, which the
//! highlighter applies a line at a time.
//!
//! Design: `doc/architettura-editor.typ, <lessici>`.
use super::*;
use crate::primitives::args;

/// Returns EXP as a face, which naming makes when nothing has named it yet.
fn face_arg<B: BufferTrait>(exp: &ELispExp<B>) -> Result<Face, EvalError<EditorState<B>>> {
    match exp {
        ELispExp::Symbol(name) | ELispExp::String(name) => Ok(Face::intern(name.as_str())),
        other => Err(EvalError::WrongArgumentType {
            expected: "Symbol or String naming a face".into(),
            got: other.clone(),
        }),
    }
}

/// Returns SOURCE compiled, or `None` when it is no pattern, which the log is
/// told rather than an error raised: a broken pattern costs itself, and not
/// the module it is written in.
pub(super) fn compile<B: BufferTrait>(ctx: &EditorState<B>, source: &str) -> Option<regex::Regex> {
    match regex::Regex::new(source) {
        Ok(pattern) => Some(pattern),
        Err(why) => {
            ctx.log_diagnostic(&format!("Invalid Regex {source}: {why}"));
            None
        }
    }
}

/// Applies EDIT to the grammar of MODE_NAME, answering t, or nil when there
/// is no such mode. Through `edit_grammar`, so that what was coloured under
/// the old grammar is coloured again under the new one.
fn with_grammar<B: BufferTrait, F>(ctx: &EditorState<B>, mode_name: &str, edit: F) -> ELispExp<B>
where
    F: FnOnce(&mut crate::modes::Grammar),
{
    match ctx.edit_grammar(mode_name, edit) {
        true => ELispExp::t(),
        false => ELispExp::nil(),
    }
}

pub const ADD_SYNTAX_RULE_DOC: &str = "(add-syntax-rule MODE REGEX FACE &optional GROUP NAME): \
         Colour in FACE, in major mode MODE, what the regular expression REGEX matches within a \
         line; with GROUP, only that capture group of the match. With NAME, a symbol, the rule \
         takes the place of the mode's rule of that name, where it stands. Return t, or nil when \
         there is no such mode or REGEX is wrong, which the log says.\n\n\
         Example:\n\
         (add-syntax-rule 'rust-mode \"\\\\b(fn|let|mut)\\\\b\" 'keyword)";

primitive!(add_syntax_rule, args, _env, ctx, {
    if !(3..=5).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }
    let (ELispExp::Symbol(mode_name), ELispExp::String(regex_str)) = (&args[0], &args[1]) else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol, String, Symbol".into(),
            got: ELispExp::string(format!("{:?}", args)),
        });
    };
    let face = face_arg(&args[2])?;
    let group = match args.get(3) {
        None => 0,
        Some(exp) if exp.is_nil() => 0,
        Some(ELispExp::Number(n)) => n.max(0.0) as usize,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
    };

    let name = args::optional_name(args.get(4), "Symbol naming the rule, or nil")?;

    let Some(pattern) = compile(ctx, regex_str) else {
        return Ok(ELispExp::nil());
    };
    let rule = SyntaxRule {
        pattern,
        face,
        group,
        name,
    };
    Ok(with_grammar(ctx, mode_name.as_str(), |grammar| {
        // A rule added under a name the mode has already takes the old rule's
        // place, so that building one again -- risp-mode's words, as
        // functions are defined -- neither grows the grammar nor moves the
        // rule among those it wins or loses against at the same column.
        let named = rule.name.as_deref();
        let old = named.and_then(|name| {
            grammar
                .rules
                .iter()
                .position(|old| old.name.as_deref() == Some(name))
        });
        match old {
            Some(at) => grammar.rules[at] = rule,
            None => grammar.rules.push(rule),
        }
    }))
});

pub const ADD_SYNTAX_REGION_DOC: &str = "(add-syntax-region MODE BEGIN END FACE &optional ESCAPE \
         NESTABLE): Colour in FACE, in major mode MODE, everything from a match of BEGIN to the \
         next match of END, across lines if need be, and nothing else inside. ESCAPE matches text \
         inside that looks like END and is not; NESTABLE non-nil means BEGIN inside opens \
         another. Return t, or nil when there is no such mode or a regular expression is wrong.\n\n\
         Example:\n\
         (add-syntax-region 'rust-mode \"/\\\\*\" \"\\\\*/\" 'comment nil t)";

primitive!(add_syntax_region, args, _env, ctx, {
    if !(4..=6).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
    let (ELispExp::Symbol(mode_name), ELispExp::String(begin_str), ELispExp::String(end_str)) =
        (&args[0], &args[1], &args[2])
    else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol, String, String, Symbol".into(),
            got: ELispExp::string(format!("{:?}", args)),
        });
    };
    let face = face_arg(&args[3])?;
    let escape = match args.get(4) {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(ELispExp::String(escape)) => match compile(ctx, escape) {
            Some(pattern) => Some(pattern),
            None => return Ok(ELispExp::nil()),
        },
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let nestable = args.get(5).is_some_and(|exp| exp.is_truthy());

    // TODO(SIN-1): a BEGIN or END that matches the empty string is taken, and
    // stalls the highlighter.
    let (Some(begin), Some(end)) = (compile(ctx, begin_str), compile(ctx, end_str)) else {
        return Ok(ELispExp::nil());
    };
    Ok(with_grammar(ctx, mode_name.as_str(), |grammar| {
        grammar.regions.push(SyntaxRegion {
            begin,
            end,
            escape,
            face,
            nestable,
        });
    }))
});

/// Registers the primitives of grammars.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("add-syntax-rule", add_syntax_rule, ADD_SYNTAX_RULE_DOC);
    into.function(
        "add-syntax-region",
        add_syntax_region,
        ADD_SYNTAX_REGION_DOC,
    );
}
