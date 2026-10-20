//! The mark, the region, and the kill ring.
use super::*;
use crate::buffer::{Buffer, Mark, mark::region_bounds};
use crate::text::kill_ring::Direction;

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

// ---------------------------------------------------------------------------
// The mark
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// The kill ring
// ---------------------------------------------------------------------------

pub const KILL_REGION_DOC: &str = "(kill-region): Kill the region, putting its text on the kill \
         ring. Signal an error when there is no active region.";

primitive!(kill_region, _args, env, ctx, {
    let killed = ctx.with_current_buffer_mut(|buf| {
        let (start, end) = require_region(buf)?;
        let text = buf.text.slice(start, end);
        // Only what actually left the buffer reaches the ring. Saving the text
        // of a refused kill would offer a later `yank' a copy of something
        // that is still there.
        let killed = edits::delete_range(buf, start, end).then_some(text);
        Ok::<_, EvalError<EditorState<B>>>(killed)
    })?;
    let Some(text) = killed else {
        return Ok(edits::edited(ctx, false));
    };
    ctx.kill(text, Direction::Forward, &env);
    Ok(ELispExp::nil())
});

pub const KILL_RING_SAVE_DOC: &str = "(kill-ring-save): Put the text of the region on the kill \
         ring without deleting it, and end the region. Signal an error when there is no active \
         region.";

primitive!(kill_ring_save, _args, env, ctx, {
    let text = ctx.with_current_buffer_mut(|buf| {
        let (start, end) = require_region(buf)?;
        let text = buf.text.slice(start, end);
        // Copying is not an edit, so nothing else would deactivate the mark --
        // but leaving the region highlighted after a copy would suggest the
        // next command still applies to it.
        if let Some(mark) = buf.mark.as_mut() {
            mark.active = false;
        }
        Ok::<_, EvalError<EditorState<B>>>(text)
    })?;
    ctx.kill(text, Direction::Forward, &env);
    Ok(ELispExp::nil())
});

pub const KILL_NEW_DOC: &str = "(kill-new STRING): Put STRING on the kill ring as a kill does. An \
         empty string is ignored.\n\n\
         Example:\n\
         (kill-new (current-line))";

primitive!(kill_new, args, env, ctx, {
    match args.first() {
        Some(ELispExp::String(text)) => {
            ctx.kill(text.to_string(), Direction::Forward, &env);
            Ok(ELispExp::nil())
        }
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
});

pub const CURRENT_KILL_DOC: &str = "(current-kill &optional N): Return the kill N back in the \
         ring, 0 by default for the latest, without changing anything; nil when the ring does not \
         go back that far.";

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

pub const YANK_DOC: &str = "(yank): Insert the latest kill at point and return it, or return nil \
         when the kill ring is empty.";

primitive!(yank, _args, _env, ctx, {
    let Some(text) = ctx.current_kill() else {
        return Ok(ELispExp::nil());
    };
    let inserted = ctx.with_current_buffer_mut(|buf| {
        let at = buf.text.cursor_pos_1d();
        edits::insert_text(buf, at, &text).then_some(at)
    });
    let Some(at) = inserted else {
        return Ok(edits::edited(ctx, false));
    };
    ctx.note_yank(at, text.chars().count());
    Ok(ELispExp::string(text))
});

pub const YANK_POP_DOC: &str = "(yank-pop): Replace the text just yanked with the kill before it \
         in the ring, and return that. Only straight after a `yank' or another `yank-pop': \
         otherwise it signals an error.";

primitive!(yank_pop, _args, _env, ctx, {
    let Some((at, len)) = ctx.yank_to_replace() else {
        return Err(EvalError::RuntimeMessage(
            "Previous command was not a yank".into(),
        ));
    };
    let Some(text) = ctx.rotate_kill_ring() else {
        return Ok(ELispExp::nil());
    };
    let happened = ctx.with_current_buffer_mut(|buf| {
        // Out then in, through the same editing layer as everything else, so
        // one `yank-pop` is one undo step like any other command. The second
        // step is not attempted if the first was refused, which would leave
        // the replacement text in a buffer the old text never left.
        edits::delete_range(buf, at, at + len) && edits::insert_text(buf, at, &text)
    });
    if !happened {
        return Ok(edits::edited(ctx, false));
    }
    ctx.note_yank(at, text.chars().count());
    Ok(ELispExp::string(text))
});

pub const SET_KILL_RING_MAX_DOC: &str = "(set-kill-ring-max N): Keep at most N kills in the ring, \
         dropping the oldest past that.";

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

pub const KILL_RING_LENGTH_DOC: &str = "(kill-ring-length): Return how many kills the ring holds.";

primitive!(kill_ring_length, _args, _env, ctx, {
    Ok(ELispExp::number(ctx.kill_ring_len() as f64))
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

/// Register this module's primitives: point, the mark, and what lies between them.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // ---------------------------------------------------------------
    // The mark, the region, and the kill ring
    // ---------------------------------------------------------------
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
    into.command("kill-region", kill_region, &[], KILL_REGION_DOC);
    into.command("kill-ring-save", kill_ring_save, &[], KILL_RING_SAVE_DOC);
    into.command("yank", yank, &[], YANK_DOC);
    into.command("yank-pop", yank_pop, &[], YANK_POP_DOC);
    into.command(
        "set-kill-ring-max",
        set_kill_ring_max,
        &["nKill ring size: "],
        SET_KILL_RING_MAX_DOC,
    );
    // Predicates and accessors rather than things to run from M-x.
    into.function("mark", mark, MARK_DOC);
    into.function("use-region-p", use_region_p, USE_REGION_P_DOC);
    into.function("region-beginning", region_beginning, REGION_BEGINNING_DOC);
    into.function("region-end", region_end, REGION_END_DOC);
    into.function("kill-new", kill_new, KILL_NEW_DOC);
    into.function("current-kill", current_kill, CURRENT_KILL_DOC);
    into.function("kill-ring-length", kill_ring_length, KILL_RING_LENGTH_DOC);
}
