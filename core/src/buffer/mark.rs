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
}

impl Mark {
    pub fn new(at: usize) -> Self {
        Self { at, active: true }
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
