//! The mark, and the region between it and point.
//!
//! Design: `doc/architettura-editor.typ, <regione>`.
use super::*;
use crate::buffer::{Buffer, Mark, mark::region_bounds};

/// Returns the active region of BUF as ordered offsets, or `None` when the
/// mark is not active.
fn region_of<B: BufferTrait>(buf: &Buffer<B>) -> Option<(usize, usize)> {
    region_bounds(buf.mark, buf.text.cursor_pos_1d(), buf.text.len())
}

/// Returns the active region of BUF, or the error a command gives when there
/// is none: a command run without a selection says so rather than doing
/// nothing.
pub(super) fn require_region<B: BufferTrait>(
    buf: &Buffer<B>,
) -> Result<(usize, usize), EvalError<EditorState<B>>> {
    region_of(buf).ok_or_else(|| {
        EvalError::RuntimeMessage("The mark is not set now, so there is no region".into())
    })
}

pub const SET_MARK_DOC: &str = "(set-mark &optional POSITION INCLUSIVE): Put the mark at \
         POSITION, point by default, and activate it, so that the text between the mark and point \
         is the region. Return the position. With INCLUSIVE, the characters at both ends are \
         inside the region, as in a modal selection.";

primitive!(set_mark, args, _env, ctx, {
    let at = match args.first() {
        None => None,
        Some(ELispExp::Number(n)) => Some(n.floor().max(0.0) as usize),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
    };
    Ok(ELispExp::number(ctx.with_current_buffer_mut(|buf| {
        let at = at
            .unwrap_or_else(|| buf.text.cursor_pos_1d())
            .min(buf.text.len());
        let mut mark = Mark::new(at);
        mark.inclusive = args.get(1).is_some_and(|value| value.is_truthy());
        buf.mark = Some(mark);
        at as f64
    })))
});

pub const DEACTIVATE_MARK_DOC: &str = "(deactivate-mark): End the region without forgetting the \
         mark, so that `exchange-point-and-mark' can still go back to it.";

primitive!(deactivate_mark, _args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        if let Some(mark) = buf.mark.as_mut() {
            mark.active = false;
        }
    });
    Ok(ELispExp::nil())
});

pub const MARK_DOC: &str = "(mark): Return the position of the mark, or nil if it has never been \
         set, whether or not the region is active: `use-region-p' says whether it is.";

primitive!(mark, _args, _env, ctx, {
    Ok(ctx.with_current_buffer(|buf| match buf.mark {
        Some(mark) => ELispExp::number(mark.at.min(buf.text.len()) as f64),
        None => ELispExp::nil(),
    }))
});

pub const USE_REGION_P_DOC: &str = "(use-region-p): Return t when the current buffer has an \
         active region, nil otherwise.";

primitive!(use_region_p, _args, _env, ctx, {
    Ok(ctx.with_current_buffer(|buf| match region_of(buf) {
        Some(_) => ELispExp::symbol("t".into()),
        None => ELispExp::nil(),
    }))
});

pub const REGION_BEGINNING_DOC: &str = "(region-beginning): Return the start of the region: \
         whichever of point and the mark comes first. Signal an error when there is no active \
         region.";

primitive!(region_beginning, _args, _env, ctx, {
    // The closure answers with the `Result` and the `?` happens out here:
    // inside, it would return from the closure rather than from the primitive.
    let start = ctx.with_current_buffer(|buf| require_region(buf).map(|region| region.0))?;
    Ok(ELispExp::number(start as f64))
});

pub const REGION_END_DOC: &str = "(region-end): Return the end of the region: whichever of point \
         and the mark comes last. Signal an error when there is no active region.";

primitive!(region_end, _args, _env, ctx, {
    let end = ctx.with_current_buffer(|buf| require_region(buf).map(|region| region.1))?;
    Ok(ELispExp::number(end as f64))
});

pub const EXCHANGE_POINT_AND_MARK_DOC: &str = "(exchange-point-and-mark): Swap point and the \
         mark, and activate the region. It works on a mark that is no longer active too; with no \
         mark ever set it signals an error.";

primitive!(exchange_point_and_mark, _args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        let Some(mark) = buf.mark else {
            return Err(EvalError::RuntimeMessage(
                "No mark set in this buffer".into(),
            ));
        };
        let point = buf.text.cursor_pos_1d();
        // Clamped rather than trusted: an inactive mark is not adjusted when
        // the text under it changes, so it can be pointing past the end.
        let target = mark.at.min(buf.text.len());
        let (line, col) = buf.text.cursor_1d_to_2d(target);
        buf.text.cursor_move(line, col);
        buf.mark = Some(Mark::new(point));
        Ok(ELispExp::number(target as f64))
    })
});

pub const MARK_WHOLE_BUFFER_DOC: &str = "(mark-whole-buffer): Make the whole buffer the region, \
         with point at its start and the mark at its end.";

primitive!(mark_whole_buffer, _args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        buf.mark = Some(Mark::new(buf.text.len()));
        buf.text.cursor_move(0, 0);
    });
    Ok(ELispExp::nil())
});

pub const KEYBOARD_QUIT_DOC: &str = "(keyboard-quit): Abandon a key sequence or a prefix argument \
         typed halfway, end the region, and say \"Quit\". Return nil. It does not stop a command \
         that is running.";

primitive!(keyboard_quit, _args, _env, ctx, {
    ctx.abandon_pending_input();
    ctx.with_current_buffer_mut(|buf| {
        if let Some(mark) = buf.mark.as_mut() {
            mark.active = false;
        }
    });
    ctx.set_echo_message("Quit");
    Ok(ELispExp::nil())
});

/// Registers the primitives of the mark and the region.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("set-mark", set_mark, &[], SET_MARK_DOC);
    into.command("keyboard-quit", keyboard_quit, &[], KEYBOARD_QUIT_DOC);
    into.command("deactivate-mark", deactivate_mark, &[], DEACTIVATE_MARK_DOC);
    into.command(
        "exchange-point-and-mark",
        exchange_point_and_mark,
        &[],
        EXCHANGE_POINT_AND_MARK_DOC,
    );
    into.command(
        "mark-whole-buffer",
        mark_whole_buffer,
        &[],
        MARK_WHOLE_BUFFER_DOC,
    );
    // Predicates and accessors rather than things to run from M-x.
    into.function("mark", mark, MARK_DOC);
    into.function("use-region-p", use_region_p, USE_REGION_P_DOC);
    into.function("region-beginning", region_beginning, REGION_BEGINNING_DOC);
    into.function("region-end", region_end, REGION_END_DOC);
}
