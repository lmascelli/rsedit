//! What the menus hold, as Lisp registered it -- the menu bar, the context
//! menu of every buffer, and that of each mode -- with the actions of the
//! menus last resolved and the menu waiting for a front end to open it.
//! Holds Lisp, never evaluates it: resolving a menu is `editor/menus.rs`'s.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.
use crate::{ELispExp, buffer::BufferTrait, ui::MenuRequest};
use std::collections::HashMap;

/// The prefix that makes a label a separator's.
pub const SEPARATOR_PREFIX: &str = "--";

/// One entry of a menu as it was registered.
#[derive(Clone, Debug)]
pub enum MenuEntry<B: BufferTrait> {
    /// A command, run as a key runs it; shown greyed out while ENABLE, when
    /// there is one, evaluates to nil.
    Item {
        label: String,
        command: ELispExp<B>,
        enable: Option<ELispExp<B>>,
    },
    /// A menu inside the menu.
    Submenu {
        label: String,
        entries: Vec<MenuEntry<B>>,
    },
    /// A line between groups, known by its label, which begins with `--`.
    Separator { label: String },
    /// The entries FUNCTION returns each time the menu opens, known by NAME,
    /// the function as it was written.
    Generated { name: String, function: ELispExp<B> },
}

impl<B: BufferTrait> MenuEntry<B> {
    /// Returns what the entry is known by among its neighbours: two entries of
    /// a menu are never known by the same.
    fn key(&self) -> &str {
        match self {
            MenuEntry::Item { label, .. }
            | MenuEntry::Submenu { label, .. }
            | MenuEntry::Separator { label } => label,
            MenuEntry::Generated { name, .. } => name,
        }
    }

    /// Returns whether the entry is a menu, which an item of the same label
    /// would replace, and the other way round.
    fn is_submenu(&self) -> bool {
        matches!(self, MenuEntry::Submenu { .. })
    }
}

/// Which registered menu an operation is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuOwner {
    /// The menu bar.
    Bar,
    /// The context menu: of every buffer for `None`, of the buffers where the
    /// mode named is on otherwise.
    Context(Option<String>),
}

/// Why an entry could not be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuPathError {
    /// The path named no label at all.
    Empty,
    /// A label on the way is an entry that holds no menu.
    NotAMenu(String),
    /// The label is a menu's, and an item would take its place, or the
    /// other way round.
    KindClash(String),
}

/// The menus' registry, the actions of the last two menus resolved, and the
/// menu waiting to be opened.
pub struct Menus<B: BufferTrait> {
    bar: Vec<MenuEntry<B>>,
    everywhere: Vec<MenuEntry<B>>,
    by_mode: HashMap<String, Vec<MenuEntry<B>>>,
    /// The number the next resolved menu is given.
    next_generation: u64,
    /// What the actions of the last menu bar resolved run, and whether each
    /// was enabled, with the menu bar's number.
    bar_actions: (u64, Vec<(ELispExp<B>, bool)>),
    /// The same, of the last context menu.
    context_actions: (u64, Vec<(ELispExp<B>, bool)>),
    /// The menu a front end is to open, taken with the next frame.
    request: Option<MenuRequest>,
}

impl<B: BufferTrait> Default for Menus<B> {
    fn default() -> Self {
        Self {
            bar: Vec::new(),
            everywhere: Vec::new(),
            by_mode: HashMap::new(),
            // Zero is no menu's, so that an id made up by a host is stale.
            next_generation: 1,
            bar_actions: (0, Vec::new()),
            context_actions: (0, Vec::new()),
            request: None,
        }
    }
}

impl<B: BufferTrait> Menus<B> {
    /// Returns the top level of OWNER's menu, to change, made when missing.
    fn top_mut(&mut self, owner: &MenuOwner) -> &mut Vec<MenuEntry<B>> {
        match owner {
            MenuOwner::Bar => &mut self.bar,
            MenuOwner::Context(None) => &mut self.everywhere,
            MenuOwner::Context(Some(mode)) => self.by_mode.entry(mode.clone()).or_default(),
        }
    }

    /// Returns the entries at the end of PATH, a list of labels of menus,
    /// within OWNER's menu, making the menus that are missing.
    fn level_mut(
        &mut self,
        owner: &MenuOwner,
        path: &[String],
    ) -> Result<&mut Vec<MenuEntry<B>>, MenuPathError> {
        let mut level = self.top_mut(owner);
        for label in path {
            let at = match level.iter().position(|entry| entry.key() == label) {
                Some(at) => at,
                None => {
                    level.push(MenuEntry::Submenu {
                        label: label.clone(),
                        entries: Vec::new(),
                    });
                    level.len() - 1
                }
            };
            level = match &mut level[at] {
                MenuEntry::Submenu { entries, .. } => entries,
                _ => return Err(MenuPathError::NotAMenu(label.clone())),
            };
        }
        Ok(level)
    }

    /// Adds ENTRY to the menu PATH names within OWNER's, after what is there,
    /// or in the place of the entry known by the same label: so that loading
    /// the same code twice changes nothing. The menus on the way are made.
    pub fn add(
        &mut self,
        owner: &MenuOwner,
        path: &[String],
        entry: MenuEntry<B>,
    ) -> Result<(), MenuPathError> {
        let level = self.level_mut(owner, path)?;
        match level.iter_mut().find(|old| old.key() == entry.key()) {
            Some(old) if old.is_submenu() != entry.is_submenu() => {
                Err(MenuPathError::KindClash(entry.key().to_string()))
            }
            Some(old) => {
                *old = entry;
                Ok(())
            }
            None => {
                level.push(entry);
                Ok(())
            }
        }
    }

    /// Takes away the entry PATH names within OWNER's menu, a menu with all
    /// it holds, and returns whether there was one.
    pub fn remove(&mut self, owner: &MenuOwner, path: &[String]) -> bool {
        let Some((last, menus)) = path.split_last() else {
            return false;
        };
        let mut level = self.top_mut(owner);
        for label in menus {
            let Some(MenuEntry::Submenu { entries, .. }) =
                level.iter_mut().find(|entry| entry.key() == label)
            else {
                return false;
            };
            level = entries;
        }
        let before = level.len();
        level.retain(|entry| entry.key() != last);
        level.len() != before
    }

    /// Returns the entries of the menu bar, copied out.
    pub fn bar(&self) -> Vec<MenuEntry<B>> {
        self.bar.clone()
    }

    /// Returns the labels of the menu bar's own entries, in order: what a
    /// front end draws on the bar.
    pub fn bar_labels(&self) -> Vec<String> {
        self.bar
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item { label, .. } | MenuEntry::Submenu { label, .. } => {
                    Some(label.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// Returns the context menu's entries for every buffer, copied out.
    pub fn everywhere(&self) -> Vec<MenuEntry<B>> {
        self.everywhere.clone()
    }

    /// Returns the context menu's entries for the buffers where MODE is on,
    /// copied out; empty when it has none.
    pub fn of_mode(&self, mode: &str) -> Vec<MenuEntry<B>> {
        self.by_mode.get(mode).cloned().unwrap_or_default()
    }

    /// Returns the number to give the next menu resolved.
    pub fn next_generation(&mut self) -> u64 {
        let generation = self.next_generation;
        self.next_generation += 1;
        generation
    }

    /// Keeps ACTIONS as what the entries of the menu bar numbered GENERATION
    /// run, in the place of the last menu bar's.
    pub fn keep_bar_actions(&mut self, generation: u64, actions: Vec<(ELispExp<B>, bool)>) {
        self.bar_actions = (generation, actions);
    }

    /// The same, for a context menu.
    pub fn keep_context_actions(&mut self, generation: u64, actions: Vec<(ELispExp<B>, bool)>) {
        self.context_actions = (generation, actions);
    }

    /// Returns what action INDEX of the menu numbered GENERATION runs, and
    /// whether it was enabled, or `None` when that menu has been resolved
    /// again since.
    pub fn action(&self, generation: u64, index: usize) -> Option<(ELispExp<B>, bool)> {
        [&self.bar_actions, &self.context_actions]
            .into_iter()
            .find(|(kept, _)| *kept == generation)
            .and_then(|(_, actions)| actions.get(index).cloned())
    }

    /// Asks a front end to open REQUEST, in the place of a request not yet
    /// taken.
    pub fn set_request(&mut self, request: MenuRequest) {
        self.request = Some(request);
    }

    /// Takes the menu waiting to be opened.
    pub fn take_request(&mut self) -> Option<MenuRequest> {
        self.request.take()
    }
}
