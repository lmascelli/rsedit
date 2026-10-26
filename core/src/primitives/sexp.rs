//! Moving over balanced expressions, into and out of lists, and killing
//! expressions. The scanner is `modes::sexp`; the mode's syntax table says
//! what a delimiter is.
//!
//! Design: `doc/architettura-editor.typ, <espressioni-indentazione>`.
use super::*;
use crate::modes::sexp;
use crate::primitives::args::{movement_count, repeat_count};
use crate::primitives::killing::cut_out;
use crate::primitives::movement::goto_offset;
use crate::text::kill_ring::Direction;

pub const FORWARD_SEXP_DOC: &str = "(forward-sexp &optional N): Move point over the next N \
         balanced expressions, 1 by default: a list, a string or an atom, with any prefix in \
         front of it. A negative N moves back. A delimiter in a string or a comment is text, and \
         at the end of a list this does nothing.";

primitive!(forward_sexp, args, _env, ctx, {
    // Before the write lock: this reads the buffer to find its mode.
    let table = ctx.current_syntax_table();
    ctx.with_current_buffer_mut(|buf| {
        let from = buf.text.cursor_pos_1d();
        let count = movement_count(args)?;
        let target = if count < 0 {
            sexp::backward(&buf.text, &table, from, count.unsigned_abs())
        } else {
            sexp::forward(
                &buf.text,
                &table,
                buf.scan_resume(from, &table),
                from,
                count as usize,
            )
        };
        goto_offset(&mut buf.text, target);
        Ok(ELispExp::nil())
    })
});

pub const BACKWARD_SEXP_DOC: &str = "(backward-sexp &optional N): Move point back over N balanced \
         expressions, 1 by default, to the start of the last one, its prefix included: before the \
         quote of 'foo. A negative N moves forward, and at the start of a list this does nothing.";

primitive!(backward_sexp, args, _env, ctx, {
    let table = ctx.current_syntax_table();
    ctx.with_current_buffer_mut(|buf| {
        let from = buf.text.cursor_pos_1d();
        let count = movement_count(args)?;
        let target = if count < 0 {
            sexp::forward(
                &buf.text,
                &table,
                buf.scan_resume(from, &table),
                from,
                count.unsigned_abs(),
            )
        } else {
            sexp::backward(&buf.text, &table, from, count as usize)
        };
        goto_offset(&mut buf.text, target);
        Ok(ELispExp::nil())
    })
});

pub const KILL_SEXP_DOC: &str = "(kill-sexp &optional N): Kill forward over N balanced \
         expressions, 1 by default: the text `forward-sexp' would move over. It goes on the kill \
         ring.";

primitive!(kill_sexp, args, env, ctx, {
    let table = ctx.current_syntax_table();
    let count = repeat_count(args)?;
    let killed = ctx.with_current_buffer_mut(|buf| {
        let from = buf.text.cursor_pos_1d();
        let to = sexp::forward(
            &buf.text,
            &table,
            buf.scan_resume(from, &table),
            from,
            count,
        );
        cut_out(buf, from, to)
    });
    ctx.kill(killed, Direction::Forward, &env);
    Ok(ELispExp::nil())
});

pub const BACKWARD_KILL_SEXP_DOC: &str = "(backward-kill-sexp &optional N): Kill back over N \
         balanced expressions, 1 by default: the text `backward-sexp' would move over. It goes on \
         the kill ring.";

primitive!(backward_kill_sexp, args, env, ctx, {
    let table = ctx.current_syntax_table();
    let count = repeat_count(args)?;
    let killed = ctx.with_current_buffer_mut(|buf| {
        let to = buf.text.cursor_pos_1d();
        let from = sexp::backward(&buf.text, &table, to, count);
        cut_out(buf, from, to)
    });
    ctx.kill(killed, Direction::Backward, &env);
    Ok(ELispExp::nil())
});

pub const UP_LIST_DOC: &str = "(up-list): Move point out of the list it is in, past its closing \
         delimiter. Does nothing at the top level.\n\n\
         Example:\n\
         (define-key nil \"C-M-n\" 'up-list)";

primitive!(up_list, _args, _env, ctx, {
    let table = ctx.current_syntax_table();
    ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        if let Some(found) =
            sexp::enclosing(&buf.text, &table, buf.scan_resume(point, &table), point)
        {
            goto_offset(&mut buf.text, found.end);
        }
        Ok(ELispExp::nil())
    })
});

pub const BACKWARD_UP_LIST_DOC: &str = "(backward-up-list): Move point back to the start of the \
         list it is in: onto its opening delimiter, or onto the prefix before it, as in '(a b). \
         Does nothing at the top level.";

primitive!(backward_up_list, _args, _env, ctx, {
    let table = ctx.current_syntax_table();
    ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        if let Some(found) =
            sexp::enclosing(&buf.text, &table, buf.scan_resume(point, &table), point)
        {
            goto_offset(&mut buf.text, found.start);
        }
        Ok(ELispExp::nil())
    })
});

pub const DOWN_LIST_DOC: &str = "(down-list): Move point just inside the next list that opens \
         after it. Does nothing when none does; a delimiter in a string or a comment does not \
         count.";

primitive!(down_list, _args, _env, ctx, {
    let table = ctx.current_syntax_table();
    ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        if let Some(at) = sexp::down(&buf.text, &table, buf.scan_resume(point, &table), point) {
            goto_offset(&mut buf.text, at);
        }
        Ok(ELispExp::nil())
    })
});

/// Registers the primitives that move over balanced expressions and kill
/// them.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("forward-sexp", forward_sexp, &["p"], FORWARD_SEXP_DOC);
    into.command("backward-sexp", backward_sexp, &["p"], BACKWARD_SEXP_DOC);
    into.command("kill-sexp", kill_sexp, &["p"], KILL_SEXP_DOC);
    into.command(
        "backward-kill-sexp",
        backward_kill_sexp,
        &["p"],
        BACKWARD_KILL_SEXP_DOC,
    );
    into.command("up-list", up_list, &[], UP_LIST_DOC);
    into.command(
        "backward-up-list",
        backward_up_list,
        &[],
        BACKWARD_UP_LIST_DOC,
    );
    into.command("down-list", down_list, &[], DOWN_LIST_DOC);
}
