//! The menus in the window, drawn with egui's own: the menu bar in the panel
//! above the grid, and a menu the editor asks for -- after a right click, or
//! `menu-bar-open` -- as a popup. The editor says what a menu holds and runs
//! what is chosen.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.
use eframe::egui::{self, Id, LayerId, Order, PopupCloseBehavior, Rect};
use rsedit_core::{Menu, MenuItem, MenuItemId};

/// Draws the entries of MENU into UI, which is a menu's: an action as a
/// button with its keys, greyed out when it cannot be chosen; a menu inside
/// as egui's submenu; a separator as a rule. The action clicked is put in
/// CHOSEN, and the menus close.
pub(super) fn entries(ui: &mut egui::Ui, menu: &Menu, chosen: &mut Option<MenuItemId>) {
    for item in &menu.items {
        match item {
            MenuItem::Action {
                label,
                enabled,
                keys,
                id,
            } => {
                let button = egui::Button::new(label.as_str()).shortcut_text(keys.as_str());
                if ui.add_enabled(*enabled, button).clicked() {
                    *chosen = Some(*id);
                    ui.close();
                }
            }
            MenuItem::Submenu { label, menu } => {
                ui.menu_button(label.as_str(), |ui| entries(ui, menu, chosen));
            }
            MenuItem::Separator => {
                ui.separator();
            }
        }
    }
}

/// A menu open as a popup, outside the bar: what it holds, the rectangle it
/// opens beside, and a number of its own, so that a menu opened again starts
/// with no submenu of the last one open.
pub(super) struct PopupMenu {
    pub(super) menu: Menu,
    pub(super) beside: Rect,
    pub(super) serial: u64,
    pub(super) open: bool,
}

impl PopupMenu {
    /// Shows the popup in CTX, while it is open, and returns the action
    /// chosen in it, if any. A click elsewhere or Escape closes it.
    pub(super) fn show(&mut self, ctx: &egui::Context) -> Option<MenuItemId> {
        let mut chosen = None;
        let id = Id::new(("rsedit-menu", self.serial));
        let layer = LayerId::new(Order::Foreground, Id::new("rsedit-menus"));
        let menu = &self.menu;
        egui::Popup::new(id, ctx.clone(), self.beside, layer)
            .kind(egui::PopupKind::Menu)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .style(egui::containers::menu::menu_style)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .open_bool(&mut self.open)
            .show(|ui| entries(ui, menu, &mut chosen));
        if chosen.is_some() {
            self.open = false;
        }
        chosen
    }
}
