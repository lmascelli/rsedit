//! The mark, and the region between it and point.
//!
//! Design: `doc/architettura-editor.typ, <mark-overlay>`.

/// Where the mark is, and whether it is currently defining a region.
///
/// Two pieces of state rather than one `Option<usize>`, because a deactivated
/// mark is not a forgotten one: `exchange-point-and-mark` goes back to it, and
/// re-activates it, long after its region stopped being highlighted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    /// Character offset the mark sits at.
    ///
    /// Not moved by edits, which is safe while the mark is active: every edit
    /// deactivates it, so an active mark has had no edit under it since it was
    /// set. An inactive one can go stale, and the two places that read it --
    /// `exchange-point-and-mark` and re-activation -- clamp it to the buffer.
    pub at: usize,
    /// Whether the region is in force: highlighted, and used by the commands
    /// that operate on a region.
    pub active: bool,
    /// Include the character at both endpoints, as modal selections do.
    pub inclusive: bool,
    /// Whether the region between this mark and point is drawn as a
    /// *rectangle*: the columns the lines have in common, rather than the run
    /// from one corner to the other. On the mark, so that the edit that ends
    /// the region ends this too. The rectangle commands work from point and
    /// the mark either way.
    pub rectangle: bool,
}

impl Mark {
    /// Creates an active mark at AT, neither inclusive nor a rectangle.
    pub fn new(at: usize) -> Self {
        Self {
            at,
            active: true,
            inclusive: false,
            rectangle: false,
        }
    }
}

/// Returns the offsets a region covers, in order and clamped to a buffer of
/// LEN characters, or `None` unless MARK is active: an inactive mark is a
/// place to go back to, not a region to act on.
pub fn region_bounds(mark: Option<Mark>, point: usize, len: usize) -> Option<(usize, usize)> {
    let mark = mark.filter(|mark| mark.active)?;
    let start = mark.at.min(point).min(len);
    let end = mark
        .at
        .max(point)
        .saturating_add(usize::from(mark.inclusive))
        .min(len);
    Some((start, end))
}
