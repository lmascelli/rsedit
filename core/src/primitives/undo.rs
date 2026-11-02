//! Undo and redo, and the primitives that group the history and bound it.
//!
//! Design: `doc/architettura-editor.typ, <undo>`.
use super::*;
use crate::primitives::edits::{Replay, Replayed, edited, replay};
use crate::primitives::movement::goto_offset;

pub const UNDO_JOIN_DOC: &str = "(undo-join): Make the edits this command makes next part of the \
         open undo group, so that they undo together with the ones before. For a change that \
         spans several keys: call it before editing. Does nothing when no group is open.";

primitive!(undo_join, args, _env, ctx, {
    exact_arity(args, 0)?;
    ctx.with_current_buffer_mut(|buf| buf.undo.join_previous_command());
    Ok(ELispExp::nil())
});

pub const UNDO_DOC: &str = "(undo): Undo the latest group of changes in the current buffer, and \
         move point to where it was made. Return t, or nil when there is nothing to undo, or the \
         buffer is read-only, which it says. A group is one command, or a word or two of typing.";

primitive!(undo, _args, _env, ctx, { Ok(replayed(ctx, Replay::Undo)) });

/// Replays the current buffer's history WAY through the third door, puts
/// point where the group was made, and returns what `undo` and `redo` answer:
/// t, or nil, having said so when the buffer is read-only.
fn replayed<B: BufferTrait>(ctx: &EditorState<B>, way: Replay) -> ELispExp<B> {
    let outcome = ctx.with_current_buffer_mut(|buf| {
        let outcome = replay(buf, way);
        if let Replayed::Applied(applied) = outcome {
            goto_offset(&mut buf.text, applied.point);
        }
        outcome
    });
    match outcome {
        Replayed::Applied(_) => ELispExp::t(),
        Replayed::Nothing => ELispExp::nil(),
        // Said once the buffer's lock is let go, as every edit says it.
        Replayed::Refused => edited(ctx, false),
    }
}

pub const REDO_DOC: &str = "(redo): Redo the latest group of changes undone in the current \
         buffer. Return t, or nil when there is nothing to redo, or the buffer is read-only, which \
         it says. An edit made after an undo forgets what that undo took back.";

primitive!(redo, _args, _env, ctx, { Ok(replayed(ctx, Replay::Redo)) });

pub const UNDO_BOUNDARY_DOC: &str = "(undo-boundary): End the current undo group, so that the \
         next change starts a new one and the two undo separately. Does nothing when no group is \
         open.\n\n\
         Example:\n\
         (self-insert \"a\")\n\
         (undo-boundary)\n\
         (self-insert \"b\")   ; (undo) takes back only the b";

primitive!(undo_boundary, _args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| buf.undo.boundary());
    Ok(ELispExp::nil())
});

pub const SET_UNDO_LIMIT_DOC: &str = "(set-undo-limit N): Keep at most N bytes of deleted text in \
         the current buffer's undo history, letting the oldest changes go past that. The latest \
         change is kept however large it is.\n\n\
         Example:\n\
         (set-undo-limit (* 4 1024 1024))";

primitive!(set_undo_limit, args, _env, ctx, {
    let limit = match args.first() {
        Some(ELispExp::Number(n)) => n.floor().max(0.0) as usize,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    ctx.with_current_buffer_mut(|buf| buf.undo.set_limit(limit));
    Ok(ELispExp::nil())
});

/// Registers undo, redo and the primitives that group the history.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("undo-join", undo_join, UNDO_JOIN_DOC);
    into.command("undo", undo, &[], UNDO_DOC);
    into.command("redo", redo, &[], REDO_DOC);
    into.command("undo-boundary", undo_boundary, &[], UNDO_BOUNDARY_DOC);
    into.command(
        "set-undo-limit",
        set_undo_limit,
        &["nUndo limit in bytes: "],
        SET_UNDO_LIMIT_DOC,
    );
}
