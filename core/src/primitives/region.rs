//! The mark, the region, and the kill ring.
use super::*;
use crate::buffer::{Buffer, Mark, mark::region_bounds};
use crate::kill_ring::Direction;

/// The active region in the current buffer, as an ordered offset pair.
fn region_of<B: BufferTrait>(buf: &Buffer<B>) -> Option<(usize, usize)> {
    region_bounds(buf.mark, buf.text.cursor_pos_1d(), buf.text.len())
}

/// Read the region, or fail the way a command should when there isn't one.
///
/// An error rather than a silent no-op: a command that quietly does nothing
/// when the user thought they had a selection is worse than one that says so.
fn require_region<B: BufferTrait>(
    buf: &Buffer<B>,
) -> Result<(usize, usize), EvalError<EditorState<B>>> {
    region_of(buf).ok_or_else(|| {
        EvalError::RuntimeMessage("The mark is not set now, so there is no region".into())
    })
}

/// The text between two offsets.
fn text_between<B: BufferTrait>(text: &B, from: usize, to: usize) -> String {
    (from..to.min(text.len()))
        .filter_map(|i| text.at(i))
        .collect()
}

// ---------------------------------------------------------------------------
// The mark
// ---------------------------------------------------------------------------

pub const SET_MARK_DOC: &str = "(set-mark &optional POSITION): Put the mark at POSITION (default \
         point) and activate it, so the text between mark and point becomes \
         the region. Returns the mark's position.\n\n\
         Any edit deactivates the mark, so a region never survives a change \
         to the text under it.\n\n\
         Example:\n\
         (define-key nil \"C- \" 'set-mark)";

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
    Ok(ELispExp::number(ctx.mutate_buffer(
        ctx.get_current_buffer(),
        |buf| {
            let at = at
                .unwrap_or_else(|| buf.text.cursor_pos_1d())
                .min(buf.text.len());
            buf.mark = Some(Mark::new(at));
            at as f64
        },
    )))
});

pub const DEACTIVATE_MARK_DOC: &str = "(deactivate-mark): Stop the region being in force, so it is no \
         longer highlighted and region commands no longer act on it. The \
         mark's position is kept, so `exchange-point-and-mark' can still go \
         back to it.";

primitive!(deactivate_mark, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        if let Some(mark) = buf.mark.as_mut() {
            mark.active = false;
        }
    });
    Ok(ELispExp::nil())
});

pub const MARK_DOC: &str = "(mark): Return the position of the mark in the current buffer, or nil \
         if it has never been set. Returns the position whether or not the \
         mark is active -- use `use-region-p' to ask whether there is a region \
         to act on.";

primitive!(mark, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let buf = buf.read().expect("read lock on buffer");
    Ok(match buf.mark {
        Some(mark) => ELispExp::number(mark.at.min(buf.text.len()) as f64),
        None => ELispExp::nil(),
    })
});

pub const USE_REGION_P_DOC: &str = "(use-region-p): Return t if there is an active region in the \
         current buffer, nil otherwise. This is the question a command should \
         ask before operating on a region.";

primitive!(use_region_p, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let buf = buf.read().expect("read lock on buffer");
    Ok(match region_of(&buf) {
        Some(_) => ELispExp::symbol("t".into()),
        None => ELispExp::nil(),
    })
});

pub const REGION_BEGINNING_DOC: &str = "(region-beginning): Return the position of the start of the \
         region -- whichever of point and mark comes first. Signals an error \
         if the mark is not active.";

primitive!(region_beginning, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let buf = buf.read().expect("read lock on buffer");
    Ok(ELispExp::number(require_region(&buf)?.0 as f64))
});

pub const REGION_END_DOC: &str = "(region-end): Return the position of the end of the region -- \
         whichever of point and mark comes last. Signals an error if the mark \
         is not active.";

primitive!(region_end, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let buf = buf.read().expect("read lock on buffer");
    Ok(ELispExp::number(require_region(&buf)?.1 as f64))
});

pub const EXCHANGE_POINT_AND_MARK_DOC: &str = "(exchange-point-and-mark): Put point where the mark \
         is and the mark where point was, activating the region. Signals an \
         error if the mark has never been set.\n\n\
         This works on an inactive mark too -- going back to where you were is \
         most of what a mark is for.";

primitive!(exchange_point_and_mark, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
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

pub const MARK_WHOLE_BUFFER_DOC: &str = "(mark-whole-buffer): Put point at the beginning of the \
         buffer and the mark at the end, making the whole buffer the region.";

primitive!(mark_whole_buffer, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        buf.mark = Some(Mark::new(buf.text.len()));
        buf.text.cursor_move(0, 0);
    });
    Ok(ELispExp::nil())
});

// ---------------------------------------------------------------------------
// The kill ring
// ---------------------------------------------------------------------------

pub const KILL_REGION_DOC: &str = "(kill-region): Delete the region and save it to the kill ring, \
         from where `yank' puts it back. Signals an error if the mark is not \
         active.\n\n\
         Example:\n\
         (define-key nil \"C-w\" 'kill-region)";

primitive!(kill_region, _args, _env, ctx, {
    let text = ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        let (start, end) = require_region(buf)?;
        let text = text_between(&buf.text, start, end);
        edits::delete_range(buf, start, end);
        Ok::<_, EvalError<EditorState<B>>>(text)
    })?;
    ctx.kill(text, Direction::Forward);
    Ok(ELispExp::nil())
});

pub const KILL_RING_SAVE_DOC: &str = "(kill-ring-save): Save the region to the kill ring without \
         deleting it, and deactivate the mark. Signals an error if the mark is \
         not active.\n\n\
         Example:\n\
         (define-key nil \"M-w\" 'kill-ring-save)";

primitive!(kill_ring_save, _args, _env, ctx, {
    let text = ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        let (start, end) = require_region(buf)?;
        let text = text_between(&buf.text, start, end);
        // Copying is not an edit, so nothing else would deactivate the mark --
        // but leaving the region highlighted after a copy would suggest the
        // next command still applies to it.
        if let Some(mark) = buf.mark.as_mut() {
            mark.active = false;
        }
        Ok::<_, EvalError<EditorState<B>>>(text)
    })?;
    ctx.kill(text, Direction::Forward);
    Ok(ELispExp::nil())
});

pub const KILL_NEW_DOC: &str = "(kill-new STRING): Add STRING to the kill ring as the most recent \
         entry, as if it had been killed. Does nothing for an empty string.";

primitive!(kill_new, args, _env, ctx, {
    match args.first() {
        Some(ELispExp::String(text)) => {
            ctx.kill(text.to_string(), Direction::Forward);
            Ok(ELispExp::nil())
        }
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
});

pub const CURRENT_KILL_DOC: &str = "(current-kill &optional N): Return the text N kills back in the \
         ring (default 0, the most recent) without changing anything, or nil \
         if the ring does not go back that far.";

primitive!(current_kill, args, _env, ctx, {
    let n = match args.first() {
        None => 0,
        Some(ELispExp::Number(n)) => n.floor().max(0.0) as usize,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
    };
    Ok(match ctx.nth_kill(n) {
        Some(text) => ELispExp::string(text),
        None => ELispExp::nil(),
    })
});

pub const YANK_DOC: &str = "(yank): Insert the most recent kill at point, and set the mark at the \
         start of what was inserted so the yanked text is the region. Returns \
         the text, or nil if the kill ring is empty.\n\n\
         Example:\n\
         (define-key nil \"C-y\" 'yank)";

primitive!(yank, _args, _env, ctx, {
    let Some(text) = ctx.current_kill() else {
        return Ok(ELispExp::nil());
    };
    let at = ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        let at = buf.text.cursor_pos_1d();
        edits::insert_text(buf, at, &text);
        at
    });
    ctx.note_yank(at, text.chars().count());
    Ok(ELispExp::string(text))
});

pub const YANK_POP_DOC: &str = "(yank-pop): Replace the text just yanked with the kill before it in \
         the ring, walking back through earlier kills. Only meaningful \
         directly after a `yank' or another `yank-pop'; signals an error \
         otherwise, since there would be nothing it could safely remove.\n\n\
         Example:\n\
         (define-key nil \"M-y\" 'yank-pop)";

primitive!(yank_pop, _args, _env, ctx, {
    let Some((at, len)) = ctx.yank_to_replace() else {
        return Err(EvalError::RuntimeMessage(
            "Previous command was not a yank".into(),
        ));
    };
    let Some(text) = ctx.rotate_kill_ring() else {
        return Ok(ELispExp::nil());
    };
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        // Out then in, through the same editing layer as everything else, so
        // one `yank-pop` is one undo step like any other command.
        edits::delete_range(buf, at, at + len);
        edits::insert_text(buf, at, &text);
    });
    ctx.note_yank(at, text.chars().count());
    Ok(ELispExp::string(text))
});

pub const SET_KILL_RING_MAX_DOC: &str = "(set-kill-ring-max N): Keep at most N entries in the kill \
         ring, dropping the oldest when it grows past that. The default is \
         60.";

primitive!(set_kill_ring_max, args, _env, ctx, {
    match args.first() {
        Some(ELispExp::Number(n)) => {
            ctx.set_kill_ring_max(n.floor().max(0.0) as usize);
            Ok(ELispExp::nil())
        }
        other => Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
});

pub const KILL_RING_LENGTH_DOC: &str = "(kill-ring-length): Return how many entries the kill ring \
         currently holds.";

primitive!(kill_ring_length, _args, _env, ctx, {
    Ok(ELispExp::number(ctx.kill_ring_len() as f64))
});
