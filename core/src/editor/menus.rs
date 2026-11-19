//! The menus' half of the facade: a menu resolved for the moment it opens --
//! its conditions evaluated, its generated entries asked for, the keys of its
//! commands looked up in the current buffer -- the menu bar laid out for the
//! modes of the buffer it is for, the menu a front end is asked to open, and
//! the entry it chose, run as a key runs its command.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.

use super::*;
use crate::feature::minibuffer::PREVIOUS_BUFFER;
use crate::managers::{MenuEntry, SEPARATOR_PREFIX};
use crate::ui::{Menu, MenuItem, MenuItemId, MenuKind, MenuRequest};
use std::collections::HashMap;

/// What the echo area says to a menu while a prompt is open.
const PROMPT_OPEN: &str = "Finish the current prompt, or cancel it with C-g";

/// Returns EXP as the form a menu runs: a command's name as it is, which
/// `run_command_form` calls as `M-x` would, a function as a call to it with
/// no arguments, and a list as the call it writes. `None` for anything else.
pub(crate) fn menu_command<B: BufferTrait>(exp: &ELispExp<B>) -> Option<ELispExp<B>> {
    match exp {
        ELispExp::Symbol(_) if !exp.is_nil() => Some(exp.clone()),
        ELispExp::Form(items) if !items.is_empty() => Some(exp.clone()),
        ELispExp::Cons(_) => Some(ELispExp::form(exp.iter().collect())),
        ELispExp::Lambda(_) | ELispExp::Primitive { .. } | ELispExp::Native { .. } => {
            Some(ELispExp::form(vec![exp.clone()]))
        }
        _ => None,
    }
}

/// Returns the name of the command TARGET, a binding of a keymap, calls with
/// no arguments of its own: what a menu entry with that command shows the
/// keys of. `where-is` reads bindings the same way.
fn bound_command<B: BufferTrait>(target: &ELispExp<B>) -> Option<String> {
    match target {
        ELispExp::Symbol(name) => Some(name.to_string()),
        ELispExp::Form(items) => match items.as_slice() {
            [ELispExp::Symbol(name)] => Some(name.to_string()),
            _ => None,
        },
        ELispExp::Cons(_) => {
            let items: Vec<ELispExp<B>> = target.iter().collect();
            match items.as_slice() {
                [ELispExp::Symbol(name)] => Some(name.to_string()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Returns whether COMMAND, as [`menu_command`] made it, names a function
/// there is: an entry for a module that was not loaded is greyed out rather
/// than failing when chosen.
fn command_exists<B: BufferTrait>(command: &ELispExp<B>, env: &Arc<Env<EditorState<B>>>) -> bool {
    let head = match command {
        ELispExp::Form(items) => items.first(),
        other => Some(other),
    };
    match head {
        Some(ELispExp::Symbol(name)) => env.get_function(name).is_some(),
        _ => true,
    }
}

/// Returns ITEMS with no separator first, last, or after another: what is
/// left of a menu whose generated entries or groups came out empty.
fn tidy(items: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut tidied: Vec<MenuItem> = Vec::with_capacity(items.len());
    for item in items {
        let separator = matches!(item, MenuItem::Separator);
        if separator
            && tidied
                .last()
                .is_none_or(|last| matches!(last, MenuItem::Separator))
        {
            continue;
        }
        tidied.push(item);
    }
    if matches!(tidied.last(), Some(MenuItem::Separator)) {
        tidied.pop();
    }
    tidied
}

/// What lays the menu bar out, read from Lisp before any of the editor's
/// locks is taken: the editor's menus kept for its end, and the buffer a
/// prompt open was opened from, whose menus the bar keeps while the prompt
/// is open rather than those of the prompt's own buffer.
pub(crate) struct BarLayout {
    finals: Vec<String>,
    origin: Option<String>,
}

impl BarLayout {
    /// Returns the layout as Lisp defines it now.
    pub(crate) fn read<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Self {
        Self {
            finals: menu_bar_final_items(env),
            origin: match env.get_variable(PREVIOUS_BUFFER) {
                Some(ELispExp::String(name)) => Some(name.to_string()),
                _ => None,
            },
        }
    }
}

/// What resolving one menu gathers as it goes: the keys of the commands, and
/// what each of its actions runs, in the order of their ids.
struct Resolution<B: BufferTrait> {
    generation: u64,
    /// The shortest keys that run each command in the current buffer.
    keys: HashMap<String, String>,
    actions: Vec<(ELispExp<B>, bool)>,
}

impl<B: BufferTrait> Resolution<B> {
    /// Returns the action LABEL, which runs COMMAND when ENABLED, numbered.
    fn action(&mut self, label: String, command: ELispExp<B>, enabled: bool) -> MenuItem {
        let keys = match &command {
            ELispExp::Symbol(name) => self.keys.get(name.as_str()).cloned().unwrap_or_default(),
            _ => String::new(),
        };
        let id = MenuItemId {
            generation: self.generation,
            index: self.actions.len(),
        };
        self.actions.push((command, enabled));
        MenuItem::Action {
            label,
            enabled,
            keys,
            id,
        }
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns the major mode, and the minor modes on in the order they were
    /// turned on, of the buffer the menu bar is for: the one the prompt open
    /// was opened from, as LAYOUT says, or else the current buffer.
    fn bar_modes(&self, layout: &BarLayout) -> (String, Vec<String>) {
        let modes = |buf: &Buffer<B>| {
            (
                buf.current_mode.clone(),
                buf.minor_modes
                    .iter()
                    .map(|on| on.name.clone())
                    .collect::<Vec<_>>(),
            )
        };
        layout
            .origin
            .as_deref()
            .and_then(|name| self.with_buffer(name, modes))
            .unwrap_or_else(|| self.with_current_buffer(modes))
    }

    /// Returns the labels of the menu bar, laid out by LAYOUT for the buffer
    /// it is for, a separator's as `--`: what a frame names, with no Lisp
    /// evaluated. The buffers, then the menus: one lock at a time.
    pub(crate) fn menu_bar_labels(&self, layout: &BarLayout) -> Vec<String> {
        let (major, minors) = self.bar_modes(layout);
        self.menus(|menus| menus.bar_labels(&major, &minors, &layout.finals))
    }

    /// Returns the shortest keys that run each command in the current buffer,
    /// fewest keys first, then the shortest to write: what a menu shows
    /// beside an entry, as Emacs' menus do.
    fn menu_keys(&self) -> HashMap<String, String> {
        let mut keys: HashMap<String, (usize, String)> = HashMap::new();
        for binding in self.key_bindings() {
            let Some(command) = bound_command(&binding.target) else {
                continue;
            };
            let written = binding.described();
            let rank = (binding.keys.len(), written.len());
            let shorter = keys
                .get(&command)
                .is_none_or(|(count, old)| rank < (*count, old.len()));
            if shorter {
                keys.insert(command, (binding.keys.len(), written));
            }
        }
        keys.into_iter()
            .map(|(command, (_, written))| (command, written))
            .collect()
    }

    /// Returns whether the condition ENABLE allows an entry now: a function's
    /// name is called, any other expression evaluated. One that fails greys
    /// the entry out, and is logged.
    fn menu_condition(&self, enable: &ELispExp<B>, env: &Arc<Env<EditorState<B>>>) -> bool {
        let form = match enable {
            ELispExp::Symbol(name) if env.get_function(name).is_some() => {
                ELispExp::form(vec![enable.clone()])
            }
            ELispExp::Lambda(_) => ELispExp::form(vec![enable.clone()]),
            other => other.clone(),
        };
        match eval(&form, env.clone(), self) {
            Ok(value) => !value.is_nil(),
            Err(why) => {
                self.log_diagnostic(&format!("[ERROR] menu condition {enable:?}: {why:?}"));
                false
            }
        }
    }

    /// Returns the entries FUNCTION gives for a menu opening now: each a
    /// string beginning with `--`, for a separator, or a list (LABEL COMMAND
    /// ENABLED), ENABLED left out for t. What fails, or is not an entry, is
    /// logged and left out.
    fn generated_entries(
        &self,
        function: &ELispExp<B>,
        env: &Arc<Env<EditorState<B>>>,
        resolution: &mut Resolution<B>,
    ) -> Vec<MenuItem> {
        let value = match call_callable(function, &[], env.clone(), self) {
            Ok(value) => value,
            Err(why) => {
                self.log_diagnostic(&format!("[ERROR] menu entries of {function:?}: {why:?}"));
                return Vec::new();
            }
        };
        let mut items = Vec::new();
        for entry in value.iter() {
            match &entry {
                ELispExp::String(label) if label.starts_with(SEPARATOR_PREFIX) => {
                    items.push(MenuItem::Separator)
                }
                ELispExp::Cons(_) => {
                    let parts: Vec<ELispExp<B>> = entry.iter().collect();
                    match parts.as_slice() {
                        [ELispExp::String(label), command, rest @ ..] if rest.len() <= 1 => {
                            let Some(command) = menu_command(command) else {
                                self.log_diagnostic(&format!(
                                    "[ERROR] menu entry {entry:?} of {function:?}: no command"
                                ));
                                continue;
                            };
                            let enabled = command_exists(&command, env)
                                && rest.first().is_none_or(|on| !on.is_nil());
                            items.push(resolution.action(label.to_string(), command, enabled));
                        }
                        _ => self.log_diagnostic(&format!(
                            "[ERROR] menu entry {entry:?} of {function:?}: not (LABEL COMMAND)"
                        )),
                    }
                }
                other => self.log_diagnostic(&format!(
                    "[ERROR] menu entry {other:?} of {function:?}: not an entry"
                )),
            }
        }
        items
    }

    /// Returns ENTRIES resolved: the conditions evaluated, the generated
    /// entries asked for, the menus inside resolved in turn. A menu inside
    /// that comes out empty is left out, unless KEEP_EMPTY_MENUS says
    /// otherwise, as for the menus of the bar, which are there to be seen.
    fn resolve_entries(
        &self,
        entries: &[MenuEntry<B>],
        env: &Arc<Env<EditorState<B>>>,
        resolution: &mut Resolution<B>,
        keep_empty_menus: bool,
    ) -> Vec<MenuItem> {
        let mut items = Vec::new();
        for entry in entries {
            match entry {
                MenuEntry::Item {
                    label,
                    command,
                    enable,
                } => {
                    let enabled = command_exists(command, env)
                        && enable
                            .as_ref()
                            .is_none_or(|enable| self.menu_condition(enable, env));
                    items.push(resolution.action(label.clone(), command.clone(), enabled));
                }
                MenuEntry::Submenu { label, entries } => {
                    let inner = self.resolve_entries(entries, env, resolution, false);
                    if keep_empty_menus || !inner.is_empty() {
                        items.push(MenuItem::Submenu {
                            label: label.clone(),
                            menu: Menu { items: inner },
                        });
                    }
                }
                MenuEntry::Separator { .. } => items.push(MenuItem::Separator),
                MenuEntry::Generated { function, .. } => {
                    items.extend(self.generated_entries(function, env, resolution));
                }
            }
        }
        tidy(items)
    }

    /// Starts resolving a menu: its number, and the keys of the current
    /// buffer.
    fn begin_resolution(&self) -> Resolution<B> {
        Resolution {
            generation: self.menus_mut(|menus| menus.next_generation()),
            keys: self.menu_keys(),
            actions: Vec::new(),
        }
    }

    /// Returns the menu bar, resolved for the current buffer: each of its
    /// menus, with what each can do now. The editor's menus come first, then
    /// those of the buffer's major mode, then those of each minor mode on in
    /// it, in the order they were turned on, then the editor's menus
    /// `menu-bar-final-items` names, such as Help: a separator between two of
    /// those groups, the empty ones left out. The ids of its actions are good
    /// until the menu bar is resolved again.
    ///
    /// Evaluates Lisp -- the conditions, the generated entries -- so it is
    /// asked on the thread that runs the commands, when a menu opens, and not
    /// at every frame: [`FrameSnapshot::menu_bar`] has the labels a bar shows,
    /// one for each entry of the bar, in the same order.
    pub fn menu_bar(&self, env: &Arc<Env<EditorState<B>>>) -> Menu {
        let _budget = self.begin_command();
        let layout = BarLayout::read(env);
        let (major, minors) = self.bar_modes(&layout);
        let entries = self.menus(|menus| menus.bar_for(&major, &minors, &layout.finals));
        let mut resolution = self.begin_resolution();
        let items = self.resolve_entries(&entries, env, &mut resolution, true);
        self.menus_mut(|menus| menus.keep_bar_actions(resolution.generation, resolution.actions));
        Menu { items }
    }

    /// Returns the context menu of the current buffer, resolved: the entries
    /// of its major mode, then those of each minor mode on in it, in the
    /// order they were turned on, then those of every buffer, a separator
    /// between two groups. The ids of its actions are good until a context
    /// menu is resolved again.
    pub fn context_menu(&self, env: &Arc<Env<EditorState<B>>>) -> Menu {
        let _budget = self.begin_command();
        let (major, minors) = self.with_current_buffer(|buf| {
            (
                buf.current_mode.clone(),
                buf.minor_modes
                    .iter()
                    .map(|on| on.name.clone())
                    .collect::<Vec<_>>(),
            )
        });
        let groups: Vec<Vec<MenuEntry<B>>> = self.menus(|menus| {
            std::iter::once(menus.of_mode(&major))
                .chain(minors.iter().map(|minor| menus.of_mode(minor)))
                .chain(std::iter::once(menus.everywhere()))
                .collect()
        });
        let mut resolution = self.begin_resolution();
        let mut items = Vec::new();
        for group in groups {
            items.push(MenuItem::Separator);
            items.extend(self.resolve_entries(&group, env, &mut resolution, false));
        }
        self.menus_mut(|menus| {
            menus.keep_context_actions(resolution.generation, resolution.actions)
        });
        Menu { items: tidy(items) }
    }

    /// Asks the front end to open the menu KIND names, at the cell AT of the
    /// frame or where it chooses, resolved now for the current buffer; and
    /// returns whether there was anything to open. An empty menu asks
    /// nothing, and says so; nor does any menu while a prompt is open, whose
    /// entries would be refused.
    pub(crate) fn request_menu(
        &self,
        kind: MenuKind,
        at: Option<(usize, usize)>,
        env: &Arc<Env<EditorState<B>>>,
    ) -> bool {
        if self.minibuffer_is_open() {
            self.set_echo_message(PROMPT_OPEN);
            return false;
        }
        let (kind, menu) = match kind {
            MenuKind::Context => (kind, self.context_menu(env)),
            MenuKind::Bar { open } => {
                let menu = self.menu_bar(env);
                let open = open.min(menu.items.len().saturating_sub(1));
                (MenuKind::Bar { open }, menu)
            }
        };
        if menu.is_empty() {
            self.set_echo_message("No menu here");
            return false;
        }
        self.menus_mut(|menus| menus.set_request(MenuRequest { kind, menu, at }));
        true
    }

    /// Takes the menu waiting for the front end to open it: on one frame.
    pub(crate) fn take_menu_request(&self) -> Option<MenuRequest> {
        self.menus_mut(|menus| menus.take_request())
    }

    /// Runs the menu entry ID as a key runs its command -- one undo step, the
    /// command hooks, `last-command` -- and returns whether it ran.
    ///
    /// Refused, with a message, when the menu has been resolved again since,
    /// when the entry was greyed out, or while a prompt is open, which a
    /// command asking a question of its own would stack another prompt on.
    pub fn run_menu_item(&self, id: MenuItemId, env: &Arc<Env<EditorState<B>>>) -> bool {
        let Some((command, enabled)) = self.menus(|menus| menus.action(id.generation, id.index))
        else {
            self.set_echo_message("That menu entry is no longer there");
            return false;
        };
        if !enabled {
            self.set_echo_message("That menu entry cannot be chosen now");
            return false;
        }
        if self.minibuffer_is_open() {
            self.set_echo_message(PROMPT_OPEN);
            return false;
        }
        self.run_command_form(command, env);
        true
    }

    /// Returns the command ID runs, as data, for `menu-bar-items` and
    /// `context-menu-items`; nil for an id no longer good.
    pub(crate) fn menu_item_command(&self, id: MenuItemId) -> ELispExp<B> {
        self.menus(|menus| menus.action(id.generation, id.index))
            .map(|(command, _)| risp::form_to_data(&command))
            .unwrap_or_else(ELispExp::nil)
    }
}
