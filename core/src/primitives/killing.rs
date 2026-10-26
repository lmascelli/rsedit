//! Killing and yanking: the commands that cut text onto the kill ring, and
//! those that read the ring back into the buffer.
//!
//! Design: `doc/architettura-editor.typ, <uccidere>`.
use super::*;
use crate::buffer::Buffer;
use crate::primitives::args::repeat_count;
use crate::primitives::movement::{
    line_bounds, paragraph_backward, paragraph_forward, word_backward, word_forward,
};
use crate::primitives::region::require_region;
use crate::text::kill_ring::Direction;

/// Deletes the text between FROM and TO and returns it, or "" when there was
/// nothing to delete or the buffer refused.
///
/// The caller puts it on the ring once the buffer's closure has returned: the
/// ring is a compartment of its own, never taken under a buffer's lock.
pub(crate) fn cut_out<B: BufferTrait>(buf: &mut Buffer<B>, from: usize, to: usize) -> String {
    let start = from.min(to);
    let end = from.max(to).min(buf.text.len());
    if start >= end {
        return String::new();
    }
    let text = buf.text.slice(start, end);
    // Nothing comes back from a refusal: text on the ring that is still in
    // the buffer would be doubled by the next yank.
    // TODO(BUF-4): nor is the refusal said: a kill in a read-only buffer is
    // silent.
    if !edits::delete_range(buf, start, end) {
        return String::new();
    }
    text
}

// ---------------------------------------------------------------------------
// Killing what a motion crosses
// ---------------------------------------------------------------------------
//
// Each command takes the text the motion of the same name crosses, asking
// the same function where it ends (`movement`). A kill copies what it takes
// to the ring, and a deletion (`delete-char`) does not, as in Emacs.

pub const KILL_LINE_DOC: &str = "(kill-line): Kill from point to the end of the line, or the \
         newline when point is at the end already, joining the next line on. The text goes on the \
         kill ring.";

primitive!(kill_line, _args, env, ctx, {
    let killed = {
        ctx.with_current_buffer_mut(|buf| {
            let from = buf.text.cursor_pos_1d();
            let (line, _) = buf.text.cursor_pos();
            let end_of_line = line_bounds(&buf.text, line).1;
            // At the end of a line the newline goes instead, so that C-k
            // repeated swallows a paragraph rather than stalling at each line
            // end.
            let to = if from == end_of_line {
                from + 1
            } else {
                end_of_line
            };
            cut_out(buf, from, to)
        })
    };
    ctx.kill(killed, Direction::Forward, &env);
    Ok(ELispExp::nil())
});

pub const KILL_WHOLE_LINE_DOC: &str = "(kill-whole-line): Kill the line point is on, with its \
         newline. The text goes on the kill ring.";

primitive!(kill_whole_line, _args, env, ctx, {
    let killed = {
        ctx.with_current_buffer_mut(|buf| {
            let (line, _) = buf.text.cursor_pos();
            let (from, end_of_line) = line_bounds(&buf.text, line);
            let to = (end_of_line + 1).min(buf.text.len());
            cut_out(buf, from, to)
        })
    };
    ctx.kill(killed, Direction::Forward, &env);
    Ok(ELispExp::nil())
});

pub const KILL_WORD_DOC: &str = "(kill-word &optional N): Kill forward to the end of the Nth \
         word, 1 by default: the text `forward-word' would move over. It goes on the kill ring.";

primitive!(kill_word, args, env, ctx, {
    // Read before the buffer is locked. It can fail, and a `?` inside the
    // closure would return from the closure rather than from the primitive.
    let count = repeat_count(args)?;
    let killed = ctx.with_current_buffer_mut(|buf| {
        let from = buf.text.cursor_pos_1d();
        let to = word_forward(&buf.text, from, count);
        cut_out(buf, from, to)
    });
    ctx.kill(killed, Direction::Forward, &env);
    Ok(ELispExp::nil())
});

pub const BACKWARD_KILL_WORD_DOC: &str = "(backward-kill-word &optional N): Kill back to the \
         start of the Nth word before point, 1 by default: the text `backward-word' would move \
         over. It goes on the kill ring.";

primitive!(backward_kill_word, args, env, ctx, {
    let count = repeat_count(args)?;
    let killed = ctx.with_current_buffer_mut(|buf| {
        let to = buf.text.cursor_pos_1d();
        let from = word_backward(&buf.text, to, count);
        cut_out(buf, from, to)
    });
    ctx.kill(killed, Direction::Backward, &env);
    Ok(ELispExp::nil())
});

pub const KILL_PARAGRAPH_DOC: &str = "(kill-paragraph): Kill forward to the end of the paragraph: \
         the text `forward-paragraph' would move over. It goes on the kill ring.";

primitive!(kill_paragraph, _args, env, ctx, {
    let killed = {
        ctx.with_current_buffer_mut(|buf| {
            let from = buf.text.cursor_pos_1d();
            let (line, _) = buf.text.cursor_pos();
            let target_line = paragraph_forward(&buf.text, line);
            let to = buf.text.cursor_2d_to_1d(target_line, 0);
            cut_out(buf, from, to)
        })
    };
    ctx.kill(killed, Direction::Forward, &env);
    Ok(ELispExp::nil())
});

pub const BACKWARD_KILL_PARAGRAPH_DOC: &str = "(backward-kill-paragraph): Kill back to the start \
         of the paragraph: the text `backward-paragraph' would move over. It goes on the kill \
         ring.";

primitive!(backward_kill_paragraph, _args, env, ctx, {
    let killed = {
        ctx.with_current_buffer_mut(|buf| {
            let to = buf.text.cursor_pos_1d();
            let (line, _) = buf.text.cursor_pos();
            let target_line = paragraph_backward(&buf.text, line);
            let from = buf.text.cursor_2d_to_1d(target_line, 0);
            cut_out(buf, from, to)
        })
    };
    ctx.kill(killed, Direction::Backward, &env);
    Ok(ELispExp::nil())
});

// ---------------------------------------------------------------------------
// The region, and the ring itself
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
        // Out then in, through the two doors, so that one `yank-pop` is one
        // undo step like any other command. The second step is not attempted
        // if the first was refused, which would leave the replacement in a
        // buffer the old text never left.
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

/// Registers the primitives that kill and yank.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("kill-line", kill_line, &[], KILL_LINE_DOC);
    into.command("kill-whole-line", kill_whole_line, &[], KILL_WHOLE_LINE_DOC);
    into.command("kill-word", kill_word, &["p"], KILL_WORD_DOC);
    into.command(
        "backward-kill-word",
        backward_kill_word,
        &["p"],
        BACKWARD_KILL_WORD_DOC,
    );
    into.command("kill-paragraph", kill_paragraph, &[], KILL_PARAGRAPH_DOC);
    into.command(
        "backward-kill-paragraph",
        backward_kill_paragraph,
        &[],
        BACKWARD_KILL_PARAGRAPH_DOC,
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
    into.function("kill-new", kill_new, KILL_NEW_DOC);
    into.function("current-kill", current_kill, CURRENT_KILL_DOC);
    into.function("kill-ring-length", kill_ring_length, KILL_RING_LENGTH_DOC);
}
