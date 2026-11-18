//! What a buffer is for, apart from what it is edited as: the role. A role is
//! given when the buffer is made, before any of its hooks runs, and stays the
//! same for as long as the buffer lives, whatever its major mode becomes. It
//! decides which minor modes the buffer admits: see
//! [`crate::managers::modes::MinorPolicies`].
//!
//! Design: `doc/architettura-editor.typ, <ruoli>`.

/// What a buffer is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BufferRole {
    /// Text somebody edits: a file, `*scratch*`. Open to every minor mode a
    /// rule does not keep out.
    #[default]
    Editing,
    /// The line a question is answered on.
    Prompt,
    /// The list of candidates offered for an answer.
    Completion,
    /// Something to look at rather than edit. Admits only the minor modes
    /// permitted for it.
    View,
    /// A text to read -- a manual page, a help page: a [`BufferRole::View`],
    /// whose rules it inherits where it has none of its own.
    DocumentView,
}

impl BufferRole {
    /// Every role, the roots before what derives from them.
    pub const ALL: [BufferRole; 5] = [
        BufferRole::Editing,
        BufferRole::Prompt,
        BufferRole::Completion,
        BufferRole::View,
        BufferRole::DocumentView,
    ];

    /// Returns the name Lisp sees.
    pub fn name(self) -> &'static str {
        match self {
            BufferRole::Editing => "editing",
            BufferRole::Prompt => "prompt",
            BufferRole::Completion => "completion",
            BufferRole::View => "view",
            BufferRole::DocumentView => "document-view",
        }
    }

    /// Returns the role called NAME, as [`Self::name`] spells it.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }

    /// Returns the role this one derives from, if any.
    pub fn parent(self) -> Option<Self> {
        match self {
            BufferRole::DocumentView => Some(BufferRole::View),
            _ => None,
        }
    }

    /// Returns this role and those it derives from, nearest first: the order
    /// a rule is looked for in.
    pub fn lineage(self) -> impl Iterator<Item = BufferRole> {
        std::iter::successors(Some(self), |role| role.parent())
    }

    /// Returns whether a minor mode no rule speaks of is admitted: only in
    /// [`BufferRole::Editing`]. A derived role answers as its root.
    pub fn open_by_default(self) -> bool {
        self.lineage().last() == Some(BufferRole::Editing)
    }
}
