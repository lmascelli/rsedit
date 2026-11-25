//! The menus in the window, drawn with egui's own: the menu bar in the panel
//! above the grid, and a menu the editor asks for -- after a right click, or
//! `menu-bar-open` -- as a popup. The editor says what a menu holds and runs
//! what is chosen.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.
use eframe::egui::{self, FocusDirection, Id, Key, LayerId, Order, PopupCloseBehavior, Rect};
use rsedit_core::{Menu, MenuItem, MenuItemId};

/// Draws the entries of MENU into UI, which is a menu's: an action as a
/// button with its keys, greyed out when it cannot be chosen; a menu inside
/// as egui's submenu; a separator as a rule. The action clicked is put in
/// CHOSEN, and the menus close. The down or up arrow, while no entry has the
/// keyboard, gives it to the first or the last entry that can be chosen: the
/// arrows then move between the entries, and Enter or Space chooses one.
/// Returns -1 or 1 when the left or right arrow, while no menu inside is
/// open, asks for the menu beside this one in the bar, and 0 otherwise.
pub(super) fn entries(ui: &mut egui::Ui, menu: &Menu, chosen: &mut Option<MenuItemId>) -> isize {
    let (focused, inside, buttons) = items(ui, menu, chosen);
    let (left, right) = ui.input(|i| {
        (
            i.key_pressed(Key::ArrowLeft),
            i.key_pressed(Key::ArrowRight),
        )
    });
    if !inside && left != right {
        // Kept where it is: egui would move it out of the menu.
        ui.memory_mut(|memory| memory.move_focus(FocusDirection::None));
        return if left { -1 } else { 1 };
    }
    if focused {
        return 0;
    }
    let (down, up) = ui.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp)));
    let target = if down {
        buttons.first()
    } else if up {
        buttons.last()
    } else {
        None
    };
    if let Some(button) = target {
        button.request_focus();
        // Kept there: egui would move it once more at the end of the pass.
        ui.memory_mut(|memory| memory.move_focus(FocusDirection::None));
    }
    0
}

/// Returns which of the COUNT entries of a menu bar, SKIP saying which are
/// not menus, is the menu STEP away from OPEN: past the last comes the
/// first, before the first the last.
pub(super) fn beside_in_bar(
    count: usize,
    open: usize,
    step: isize,
    skip: impl Fn(usize) -> bool,
) -> usize {
    let mut at = open;
    for _ in 0..count {
        at = (at as isize + step).rem_euclid(count as isize) as usize;
        if !skip(at) {
            return at;
        }
    }
    open
}

/// Draws the entries of MENU into UI as `entries` does, and returns whether
/// one of them, or of a menu open inside, has the keyboard, whether a menu
/// inside is open, and the entries that can be chosen.
fn items(
    ui: &mut egui::Ui,
    menu: &Menu,
    chosen: &mut Option<MenuItemId>,
) -> (bool, bool, Vec<egui::Response>) {
    let mut focused = false;
    let mut inside = false;
    let mut buttons = Vec::new();
    for item in &menu.items {
        match item {
            MenuItem::Action {
                label,
                enabled,
                keys,
                id,
            } => {
                let button = egui::Button::new(label.as_str()).shortcut_text(keys.as_str());
                let response = ui.add_enabled(*enabled, button);
                if response.clicked() {
                    *chosen = Some(*id);
                    ui.close();
                }
                focused |= response.has_focus();
                if *enabled {
                    buttons.push(response);
                }
            }
            MenuItem::Submenu { label, menu } => {
                let response = ui.menu_button(label.as_str(), |ui| items(ui, menu, chosen).0);
                focused |= response.response.has_focus() || response.inner == Some(true);
                inside |= response.inner.is_some();
                buttons.push(response.response);
            }
            MenuItem::Separator => {
                ui.separator();
            }
        }
    }
    (focused, inside, buttons)
}

/// A menu open as a popup, outside the bar: what it holds, the rectangle it
/// opens beside, and a number of its own, so that a menu opened again starts
/// with no submenu of the last one open. A menu of the bar keeps the bar and
/// which of its entries it is.
pub(super) struct PopupMenu {
    pub(super) menu: Menu,
    pub(super) beside: Rect,
    pub(super) serial: u64,
    pub(super) open: bool,
    pub(super) bar: Option<(Menu, usize)>,
}

impl PopupMenu {
    /// Shows the popup in CTX, while it is open, and returns the action
    /// chosen in it, if any. A click elsewhere or Escape closes it; in a menu
    /// of the bar, the left and right arrows show the menu beside it instead.
    pub(super) fn show(&mut self, ctx: &egui::Context) -> Option<MenuItemId> {
        let mut chosen = None;
        // One for each menu of the bar it shows: no entry of the last one
        // keeps the keyboard.
        let open = self.bar.as_ref().map(|(_, open)| *open);
        let id = Id::new(("rsedit-menu", self.serial, open));
        let layer = LayerId::new(Order::Foreground, Id::new("rsedit-menus"));
        let menu = &self.menu;
        let step = egui::Popup::new(id, ctx.clone(), self.beside, layer)
            .kind(egui::PopupKind::Menu)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .style(egui::containers::menu::menu_style)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .open_bool(&mut self.open)
            .show(|ui| entries(ui, menu, &mut chosen))
            .map_or(0, |response| response.inner);
        if chosen.is_some() {
            self.open = false;
        } else if step != 0
            && let Some((bar, open)) = &mut self.bar
        {
            *open = beside_in_bar(bar.items.len(), *open, step, |at| {
                matches!(bar.items[at], MenuItem::Separator)
            });
            self.menu = match &bar.items[*open] {
                MenuItem::Submenu { menu, .. } => menu.clone(),
                action => Menu {
                    items: vec![action.clone()],
                },
            };
        }
        chosen
    }
}
