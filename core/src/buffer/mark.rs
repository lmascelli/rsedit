//! The mark, and the region between it and point.

/// Where the mark is, and whether it is currently defining a region.
///
/// Two pieces of state rather than one `Option<usize>` because a mark that has
/// been deactivated is not a mark that has been forgotten: `exchange-point-and-mark`
/// goes back to it, and re-activates it, long after the region it defined
/// stopped being highlighted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    /// Character offset the mark sits at.
    ///
    /// Not adjusted when text is inserted or deleted before it, which sounds
    /// like a bug and is not one *while the mark is active*: every edit
    /// deactivates the mark, so an active mark has had no edit under it since
    /// it was set. An inactive one can go stale, and the two places that read
    /// it -- `exchange-point-and-mark` and re-activation -- clamp it to the
    /// buffer rather than trusting it. Real markers that survive edits are
    /// what a mark ring would need, and it does not exist yet.
    pub at: usize,
    /// Whether the region is in force: highlighted, and used by the commands
    /// that operate on a region.
    pub active: bool,
    /// Whether the region between this mark and point is a *rectangle*: the
    /// columns the lines have in common, rather than the ragged run from one
    /// corner to the other.
    ///
    /// # Why it lives on the mark
    ///
    /// Because it is a property of the selection, not of the buffer. Every
    /// edit deactivates the mark, so an edit ends rectangle mode by the same
    /// act that ends the region -- there is no second thing to remember to
    /// turn off, and no way for a buffer to be left in a mode whose selection
    /// has gone.
    ///
    /// It changes what is *drawn* and nothing else. The rectangle commands
    /// work from point and the mark whether or not this is set, because two
    /// corners are two corners; this is how you see which block they make
    /// before you cut it.
    pub rectangle: bool,
}

impl Mark {
    pub fn new(at: usize) -> Self {
        Self {
            at,
            active: true,
            rectangle: false,
        }
    }
}

/// The ordered pair of offsets a region covers, clamped to a buffer of
/// LEN characters.
///
/// Returns `None` unless there is an *active* mark: an inactive mark is a
/// place to go back to, not a region to act on. Point and mark are returned in
/// order, so a caller never has to care which way round the user made the
/// selection.
pub fn region_bounds(mark: Option<Mark>, point: usize, len: usize) -> Option<(usize, usize)> {
    let mark = mark.filter(|mark| mark.active)?;
    let start = mark.at.min(point).min(len);
    let end = mark.at.max(point).min(len);
    Some((start, end))
}

/// The two corners of the rectangle between MARK and POINT, as (line, column)
/// pairs, or `None` when there is no active mark.
///
/// Separate from [`region_bounds`] because the two answer different
/// questions about the same pair of positions -- a run of offsets, or a block
/// of columns -- and neither is derivable from the other without the buffer
/// they came from.
pub fn rectangle_corners<B: crate::buffer::BufferTrait>(
    mark: Option<Mark>,
    text: &B,
) -> Option<crate::text::rectangle::Rectangle> {
    let mark = mark.filter(|mark| mark.active)?;
    let point = text.cursor_pos_1d();
    Some(crate::text::rectangle::Rectangle::between(
        text.cursor_1d_to_2d(mark.at.min(text.len())),
        text.cursor_1d_to_2d(point),
    ))
}
