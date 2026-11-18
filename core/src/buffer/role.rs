//! What a buffer is for, apart from what it is edited as: the role. A role is
//! given when the buffer is made, before any of its hooks runs, and stays the
//! same for as long as the buffer lives, whatever its major mode becomes. It
//! decides which minor modes the buffer admits: see
//! [`crate::managers::modes::MinorPolicies`].
//!
//! The five roles the editor ships are variants; one a configuration defines
//! with `define-buffer-role` is a [`BufferRole::Defined`], which carries its
//! name and its parent, so that the lineage of any role is read off the role
//! itself, with no registry to ask and no lock to take. Which names an editor
//! knows is the registry's: [`crate::managers::modes::Modes::role_named`].
//!
//! Design: `doc/architettura-editor.typ, <ruoli>`.
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

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
    /// A role a configuration defined: see [`DefinedRole`].
    Defined(&'static DefinedRole),
}

/// A role `define-buffer-role` made: its name, and the role it derives from,
/// or none for a root, which admits only the minor modes permitted for it.
///
/// Interned for the life of the process, so that a role stays `Copy` and its
/// name a `&'static str`: a definition is a line of configuration, the same
/// definition made again gets the same one back, and an editor that knows
/// another role by the same name -- a second editor, a test -- has a
/// definition of its own beside this one.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct DefinedRole {
    name: &'static str,
    parent: Option<BufferRole>,
}

impl DefinedRole {
    /// Returns the definition of NAME deriving from PARENT, made once per
    /// process and shared by every editor that makes the same one.
    pub fn interned(name: &str, parent: Option<BufferRole>) -> &'static DefinedRole {
        static INTERNED: OnceLock<Mutex<HashSet<&'static DefinedRole>>> = OnceLock::new();
        let mut table = INTERNED
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let probe = table
            .iter()
            .find(|known| known.name == name && known.parent == parent)
            .copied();
        probe.unwrap_or_else(|| {
            let made: &'static DefinedRole = Box::leak(Box::new(DefinedRole {
                name: Box::leak(name.to_string().into_boxed_str()),
                parent,
            }));
            table.insert(made);
            made
        })
    }
}

impl BufferRole {
    /// The roles the editor ships, the roots before what derives from them.
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
            BufferRole::Defined(defined) => defined.name,
        }
    }

    /// Returns the role the editor ships called NAME, as [`Self::name`]
    /// spells it. A role a configuration defined is the registry's to find.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }

    /// Returns the role this one derives from, if any.
    pub fn parent(self) -> Option<Self> {
        match self {
            BufferRole::DocumentView => Some(BufferRole::View),
            BufferRole::Defined(defined) => defined.parent,
            _ => None,
        }
    }

    /// Returns this role and those it derives from, nearest first: the order
    /// a rule is looked for in.
    pub fn lineage(self) -> impl Iterator<Item = BufferRole> {
        std::iter::successors(Some(self), |role| role.parent())
    }

    /// Returns the role at the top of this one's lineage.
    pub fn root(self) -> BufferRole {
        self.lineage().last().unwrap_or(self)
    }

    /// Returns whether this role is ANCESTOR or derives from it: a
    /// [`BufferRole::DocumentView`] is a [`BufferRole::View`], and not the
    /// other way round.
    pub fn derives_from(self, ancestor: BufferRole) -> bool {
        self.lineage().any(|role| role == ancestor)
    }

    /// Returns whether a minor mode no rule speaks of is admitted: only in
    /// [`BufferRole::Editing`]. A derived role answers as its root.
    pub fn open_by_default(self) -> bool {
        self.root() == BufferRole::Editing
    }
}
