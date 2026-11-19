//! A menu as a front end draws it: labels, whether each can be chosen, the
//! keys that do the same, and the token to hand back when one is chosen.
//! The editor resolves a menu from what Lisp registered; how it looks, and
//! how it is navigated, is the front end's.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.

/// What a label begins with to be a separator's: in the menus Lisp registers,
/// and in the labels of a menu bar, [`Menu::bar_labels`] and
/// [`FrameSnapshot::menu_bar`](crate::FrameSnapshot::menu_bar), where the
/// line drawn between two groups of menus is written so. No menu's label
/// begins with it.
pub const MENU_SEPARATOR: &str = "--";

/// Returns whether LABEL, one of a menu bar's, is a separator's.
pub fn is_menu_separator(label: &str) -> bool {
    label.starts_with(MENU_SEPARATOR)
}

/// A menu resolved for the moment it is opened: what it holds, in order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Menu {
    pub items: Vec<MenuItem>,
}

impl Menu {
    /// Returns whether the menu holds nothing.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the labels of the menu's entries that have one, in order: the
    /// names of the menus of a menu bar.
    pub fn labels(&self) -> Vec<&str> {
        self.items.iter().filter_map(MenuItem::label).collect()
    }

    /// Returns a label for each of the menu's entries, a separator's written
    /// as [`MENU_SEPARATOR`]: what a front end draws on a menu bar, the label
    /// at an index naming the entry at the same index.
    pub fn bar_labels(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|item| item.label().unwrap_or(MENU_SEPARATOR).to_string())
            .collect()
    }
}

/// One entry of a [`Menu`].
#[derive(Debug, Clone, PartialEq)]
pub enum MenuItem {
    /// Something to do. `keys` are the keys that run the same command in the
    /// buffer the menu was opened for, written as a binding is, or empty;
    /// `id` is what [`EditorState::run_menu_item`] takes.
    ///
    /// [`EditorState::run_menu_item`]: crate::EditorState::run_menu_item
    Action {
        label: String,
        enabled: bool,
        keys: String,
        id: MenuItemId,
    },
    /// A menu inside this one.
    Submenu { label: String, menu: Menu },
    /// A line between two groups of entries.
    Separator,
}

impl MenuItem {
    /// Returns the entry's label, or `None` for a separator.
    pub fn label(&self) -> Option<&str> {
        match self {
            MenuItem::Action { label, .. } | MenuItem::Submenu { label, .. } => Some(label),
            MenuItem::Separator => None,
        }
    }

    /// Returns whether the entry can be chosen: an action that is enabled, or
    /// a submenu.
    pub fn is_selectable(&self) -> bool {
        match self {
            MenuItem::Action { enabled, .. } => *enabled,
            MenuItem::Submenu { .. } => true,
            MenuItem::Separator => false,
        }
    }
}

/// Which action of which resolved menu an entry runs: valid until the same
/// kind of menu is resolved again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MenuItemId {
    pub(crate) generation: u64,
    pub(crate) index: usize,
}

/// Which menu the editor asks a front end to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    /// The menu of what is under the pointer, or at point.
    Context,
    /// The menu bar, with the menu at index `open` of its entries opened.
    Bar { open: usize },
}

/// A menu the editor asks a front end to open: after a right click, or a
/// command such as `menu-bar-open`. On one frame only, as
/// [`FrameSnapshot::menu`](crate::FrameSnapshot::menu) carries it.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuRequest {
    pub kind: MenuKind,
    pub menu: Menu,
    /// The cell of the frame the menu opens at, column then row, or `None`
    /// when the front end chooses: at point, or under the menu bar.
    pub at: Option<(usize, usize)>,
}
