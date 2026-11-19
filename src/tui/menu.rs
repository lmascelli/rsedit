//! The menus in the terminal: the menu bar on the row above the windows, and
//! the menus open over them, boxes walked with the arrows or the mouse. The
//! editor says what a menu holds and runs what is chosen; how it looks and
//! how it is walked is here.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.
use super::colour::ColorDepth;
use super::render::{Clip, apply_style, draw_clipped_row, page, reset};
use rsedit_core::width::{cells, fit};
use rsedit_core::{Face, KeyCode, KeyEvent, Menu, MenuItem, MenuItemId, Style, Theme};
use std::io::Write;

/// The cells between two labels of the bar, and before the first.
const BAR_GAP: usize = 2;

/// What a key or a click did to the menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// They are still open, perhaps changed.
    Stay,
    /// They are closed, and nothing is to run.
    Close,
    /// They are closed, and the entry ID is to run.
    Run(MenuItemId),
}

/// Returns the cells each of LABELS takes on the bar, start and end, in
/// order: one gap before the first and between two.
pub fn bar_layout(labels: &[String]) -> Vec<(usize, usize)> {
    let mut x = BAR_GAP / 2;
    labels
        .iter()
        .map(|label| {
            let span = (x, x + cells(label));
            x = span.1 + BAR_GAP;
            span
        })
        .collect()
}

/// Returns which of LABELS the cell X of the bar is over, its gaps counted
/// with the label they come before.
pub fn bar_label_at(labels: &[String], x: usize) -> Option<usize> {
    bar_layout(labels)
        .iter()
        .position(|&(start, end)| x + BAR_GAP / 2 >= start && x < end + BAR_GAP / 2)
}

/// Returns how FACE is drawn in THEME, or FALLBACK while nothing styles it:
/// the menus' faces are a module's, and the bar must still show without it.
fn style_or(theme: &Theme, name: &str, fallback: Style) -> Style {
    match Face::named(name).map(|face| theme.style(face)) {
        Some(style) if !style.is_plain() => style,
        _ => fallback,
    }
}

/// The styles the menus are drawn in: `menu`, `menu-selected` and
/// `menu-disabled`, or the status line's when they are not styled.
struct MenuStyles {
    plain: Style,
    selected: Style,
    disabled: Style,
}

impl MenuStyles {
    fn of(theme: &Theme) -> Self {
        let plain = style_or(theme, "menu", theme.style(Face::MODE_LINE));
        Self {
            selected: style_or(theme, "menu-selected", page(theme)),
            disabled: style_or(
                theme,
                "menu-disabled",
                theme.style(Face::MODE_LINE_INACTIVE),
            ),
            plain,
        }
    }
}

/// Draws the menu bar on row 0 of a terminal WIDTH cells wide: LABELS, the
/// one at OPEN, if any, as selected.
pub fn draw_bar<W: Write>(
    out: &mut W,
    labels: &[String],
    open: Option<usize>,
    theme: &Theme,
    depth: ColorDepth,
    width: usize,
) -> std::io::Result<()> {
    let styles = MenuStyles::of(theme);
    let base = page(theme);
    let clip = Clip {
        width: width as isize,
        height: 1,
        top: 0,
    };
    apply_style(out, &styles.plain, depth)?;
    draw_clipped_row(out, 0, 0, &" ".repeat(width), clip)?;
    for (index, (label, (start, _))) in labels.iter().zip(bar_layout(labels)).enumerate() {
        if open == Some(index) {
            reset(out, &base, depth)?;
            apply_style(out, &styles.selected, depth)?;
        }
        draw_clipped_row(out, start as isize, 0, label, clip)?;
        if open == Some(index) {
            reset(out, &base, depth)?;
            apply_style(out, &styles.plain, depth)?;
        }
    }
    reset(out, &base, depth)
}

/// One box on screen: a menu, the entry the keys are on, and where it is.
#[derive(Clone, Debug, PartialEq)]
struct Level {
    menu: Menu,
    selected: Option<usize>,
    /// The cell of its top-left corner, border included.
    x: isize,
    y: isize,
    /// The cells between its borders.
    inner: usize,
}

impl Level {
    /// Returns MENU in a box whose corner is as near (X, Y) as a screen of
    /// SCREEN cells lets it be, the first entry it can choose selected when
    /// SELECT says so.
    fn new(menu: Menu, x: isize, y: isize, screen: (usize, usize), select: bool) -> Self {
        let inner = menu
            .items
            .iter()
            .map(|item| match item {
                MenuItem::Action { label, keys, .. } => {
                    cells(label) + if keys.is_empty() { 0 } else { 3 + cells(keys) }
                }
                MenuItem::Submenu { label, .. } => cells(label) + 3,
                MenuItem::Separator => 0,
            })
            .max()
            .unwrap_or(0)
            + 2;
        let (width, height) = (inner as isize + 2, menu.items.len() as isize + 2);
        let (screen_w, screen_h) = (screen.0 as isize, screen.1 as isize);
        let x = if x + width > screen_w {
            (screen_w - width).max(0)
        } else {
            x.max(0)
        };
        let y = if y + height > screen_h {
            (screen_h - height).max(0)
        } else {
            y.max(0)
        };
        let selected = select
            .then(|| menu.items.iter().position(MenuItem::is_selectable))
            .flatten();
        Self {
            menu,
            selected,
            x,
            y,
            inner,
        }
    }

    /// Returns the cells the box takes across, borders included.
    fn width(&self) -> isize {
        self.inner as isize + 2
    }

    /// Returns which entry the cell (X, Y) is on, if it is on one.
    fn entry_at(&self, x: isize, y: isize) -> Option<usize> {
        let row = y - self.y - 1;
        let inside = x > self.x && x < self.x + self.width() - 1;
        (inside && (0..self.menu.items.len() as isize).contains(&row)).then_some(row as usize)
    }

    /// Returns whether the cell (X, Y) is on the box, borders included.
    fn covers(&self, x: isize, y: isize) -> bool {
        (self.x..self.x + self.width()).contains(&x)
            && (self.y..self.y + self.menu.items.len() as isize + 2).contains(&y)
    }

    /// Moves the selection STEP entries on, past what cannot be chosen, round
    /// the ends.
    fn step(&mut self, step: isize) {
        let count = self.menu.items.len() as isize;
        if count == 0 {
            return;
        }
        let mut at =
            self.selected
                .map(|at| at as isize)
                .unwrap_or(if step > 0 { -1 } else { count });
        for _ in 0..count {
            at = (at + step).rem_euclid(count);
            if self.menu.items[at as usize].is_selectable() {
                self.selected = Some(at as usize);
                return;
            }
        }
    }

    /// Selects the next entry after the selected one whose label begins with
    /// C, ignoring case.
    fn jump(&mut self, c: char) {
        let count = self.menu.items.len();
        let start = self.selected.map_or(0, |at| at + 1);
        let wanted = c.to_lowercase().next().unwrap_or(c);
        let found = (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&at| {
                let item = &self.menu.items[at];
                item.is_selectable()
                    && item
                        .label()
                        .and_then(|label| label.chars().next())
                        .and_then(|first| first.to_lowercase().next())
                        == Some(wanted)
            });
        if found.is_some() {
            self.selected = found;
        }
    }

    /// Draws the box, in STYLES, on a terminal CLIP.
    fn draw<W: Write>(
        &self,
        out: &mut W,
        styles: &MenuStyles,
        base: &Style,
        depth: ColorDepth,
        clip: Clip,
    ) -> std::io::Result<()> {
        let rule = "─".repeat(self.inner);
        apply_style(out, &styles.plain, depth)?;
        draw_clipped_row(out, self.x, self.y, &format!("┌{rule}┐"), clip)?;
        let bottom = self.y + self.menu.items.len() as isize + 1;
        draw_clipped_row(out, self.x, bottom, &format!("└{rule}┘"), clip)?;
        for (index, item) in self.menu.items.iter().enumerate() {
            let y = self.y + 1 + index as isize;
            let text = match item {
                MenuItem::Separator => {
                    draw_clipped_row(out, self.x, y, &format!("├{rule}┤"), clip)?;
                    continue;
                }
                MenuItem::Action { label, keys, .. } => {
                    let room = self.inner.saturating_sub(2 + cells(keys));
                    format!(" {}{} ", fit(label, room), keys)
                }
                MenuItem::Submenu { label, .. } => {
                    let room = self.inner.saturating_sub(3);
                    format!(" {}▸ ", fit(label, room))
                }
            };
            draw_clipped_row(out, self.x, y, "│", clip)?;
            draw_clipped_row(out, self.x + self.width() - 1, y, "│", clip)?;
            let style = if self.selected == Some(index) {
                &styles.selected
            } else if item.is_selectable() {
                &styles.plain
            } else {
                &styles.disabled
            };
            reset(out, base, depth)?;
            apply_style(out, style, depth)?;
            draw_clipped_row(out, self.x + 1, y, &fit(&text, self.inner), clip)?;
            reset(out, base, depth)?;
            apply_style(out, &styles.plain, depth)?;
        }
        reset(out, base, depth)
    }
}

/// The menus open on screen: the boxes, the first the one opened and each
/// after it a menu inside the one before, and the bar's entries when they
/// were opened from the bar.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenMenus {
    /// The menu bar resolved, and which of its entries is open.
    bar: Option<(Menu, usize)>,
    levels: Vec<Level>,
}

impl OpenMenus {
    /// Returns MENU opened at the cell AT of a screen SCREEN cells large, the
    /// first entry that can be chosen selected when KEYBOARD says the keys
    /// opened it.
    pub fn context(menu: Menu, at: (usize, usize), screen: (usize, usize), keyboard: bool) -> Self {
        Self {
            bar: None,
            levels: vec![Level::new(
                menu,
                at.0 as isize,
                at.1 as isize,
                screen,
                keyboard,
            )],
        }
    }

    /// Returns the menu bar BAR, whose labels are on row 0, with its entry
    /// OPEN open under its label.
    pub fn bar(bar: Menu, open: usize, screen: (usize, usize), keyboard: bool) -> Self {
        let mut menus = Self {
            bar: Some((bar, 0)),
            levels: Vec::new(),
        };
        menus.open_bar_entry(open, screen, keyboard);
        menus
    }

    /// Returns the labels of the bar the menus were opened from, if they were.
    pub fn bar_labels(&self) -> Option<Vec<String>> {
        self.bar
            .as_ref()
            .map(|(bar, _)| bar.labels().into_iter().map(str::to_string).collect())
    }

    /// Returns which entry of the bar is open, if the menus were opened from
    /// the bar.
    pub fn bar_open(&self) -> Option<usize> {
        self.bar.as_ref().map(|(_, open)| *open)
    }

    /// Returns the entry of the bar that is open when it is an action of its
    /// own, enabled, rather than a menu: what a press on its label runs.
    pub fn bar_action(&self) -> Option<MenuItemId> {
        match &self.bar {
            Some((bar, open)) if self.levels.is_empty() => match bar.items.get(*open) {
                Some(MenuItem::Action {
                    id, enabled: true, ..
                }) => Some(*id),
                _ => None,
            },
            _ => None,
        }
    }

    /// Opens entry OPEN of the bar, in the place of the one open: its menu
    /// in a box under its label, or nothing when it is an action.
    fn open_bar_entry(&mut self, open: usize, screen: (usize, usize), keyboard: bool) {
        let Some((bar, at)) = self.bar.as_mut() else {
            return;
        };
        *at = open.min(bar.items.len().saturating_sub(1));
        self.levels.clear();
        let labels: Vec<String> = bar.labels().into_iter().map(str::to_string).collect();
        let x = bar_layout(&labels).get(*at).map_or(0, |(start, _)| *start);
        if let Some(MenuItem::Submenu { menu, .. }) = bar.items.get(*at) {
            self.levels.push(Level::new(
                menu.clone(),
                x as isize - 1,
                1,
                screen,
                keyboard,
            ));
        }
    }

    /// Moves to the entry of the bar STEP on, round the ends.
    fn step_bar(&mut self, step: isize, screen: (usize, usize)) {
        let Some((bar, open)) = &self.bar else {
            return;
        };
        let count = bar.items.len() as isize;
        if count > 0 {
            let next = (*open as isize + step).rem_euclid(count) as usize;
            self.open_bar_entry(next, screen, true);
        }
    }

    /// Opens the menu inside the selected entry of the last box, to the right
    /// of it or, where there is no room, to its left; returns whether there
    /// was one.
    fn enter(&mut self, screen: (usize, usize), keyboard: bool) -> bool {
        let Some(last) = self.levels.last() else {
            return false;
        };
        let Some(at) = last.selected else {
            return false;
        };
        let Some(MenuItem::Submenu { menu, .. }) = last.menu.items.get(at) else {
            return false;
        };
        let mut inner = Level::new(
            menu.clone(),
            last.x + last.width(),
            last.y + at as isize,
            screen,
            keyboard,
        );
        if inner.x < last.x + last.width() {
            inner.x = (last.x - inner.width()).max(0);
        }
        self.levels.push(inner);
        true
    }

    /// Returns what choosing the selected entry of the last box does: runs
    /// an action, opens a menu.
    fn choose(&mut self, screen: (usize, usize)) -> Outcome {
        if self.levels.is_empty() {
            return self.bar_action().map_or(Outcome::Stay, Outcome::Run);
        }
        let Some(last) = self.levels.last() else {
            return Outcome::Stay;
        };
        match last.selected.and_then(|at| last.menu.items.get(at)) {
            Some(MenuItem::Action {
                id, enabled: true, ..
            }) => Outcome::Run(*id),
            Some(MenuItem::Submenu { .. }) => {
                self.enter(screen, true);
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    /// Takes KEY, on a screen SCREEN cells large, and says what it did. The
    /// arrows, C-n, C-p, C-f and C-b walk; Return and the space choose; Esc,
    /// C-g and F10 close; a letter goes to the next entry it begins.
    pub fn key(&mut self, key: &KeyEvent, screen: (usize, usize)) -> Outcome {
        let ctrl = key.modifiers.ctrl;
        match (&key.code, ctrl) {
            (KeyCode::Esc, _) | (KeyCode::Char('g'), true) | (KeyCode::F(10), _) => Outcome::Close,
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) => {
                if let Some(last) = self.levels.last_mut() {
                    last.step(-1);
                }
                Outcome::Stay
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                if let Some(last) = self.levels.last_mut() {
                    last.step(1);
                }
                Outcome::Stay
            }
            (KeyCode::Right, _) | (KeyCode::Char('f'), true) => {
                if !self.enter(screen, true) {
                    self.step_bar(1, screen);
                }
                Outcome::Stay
            }
            (KeyCode::Left, _) | (KeyCode::Char('b'), true) => {
                if self.levels.len() > 1 {
                    self.levels.pop();
                } else {
                    self.step_bar(-1, screen);
                }
                Outcome::Stay
            }
            (KeyCode::Enter, _) | (KeyCode::Char(' '), false) => self.choose(screen),
            (KeyCode::Char(c), false) => {
                if let Some(last) = self.levels.last_mut() {
                    last.jump(*c);
                }
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    /// Takes a press of the left button on the cell (X, Y) of the screen and
    /// says what it did: an entry is chosen, a label of the bar on row 0, when
    /// the menus were opened from the bar, opens its menu, and a press
    /// anywhere else closes the menus.
    pub fn click(&mut self, x: usize, y: usize, screen: (usize, usize)) -> Outcome {
        let (px, py) = (x as isize, y as isize);
        if let Some(depth) = self.levels.iter().rposition(|level| level.covers(px, py)) {
            self.levels.truncate(depth + 1);
            let level = &mut self.levels[depth];
            let Some(at) = level.entry_at(px, py) else {
                return Outcome::Stay;
            };
            if !level.menu.items[at].is_selectable() {
                return Outcome::Stay;
            }
            level.selected = Some(at);
            return self.choose(screen);
        }
        // The labels of the bar the menus came from, which is drawn while
        // they are open even where `menu-bar-mode` gives it no row.
        if y == 0
            && let Some(labels) = self.bar_labels()
        {
            return match bar_label_at(&labels, x) {
                Some(open) if Some(open) == self.bar_open() => Outcome::Close,
                Some(open) => {
                    self.open_bar_entry(open, screen, false);
                    if self.levels.is_empty() {
                        self.choose(screen)
                    } else {
                        Outcome::Stay
                    }
                }
                None => Outcome::Close,
            };
        }
        Outcome::Close
    }

    /// Selects the entry under the pointer at (X, Y), if it is over one.
    pub fn hover(&mut self, x: usize, y: usize) {
        let (px, py) = (x as isize, y as isize);
        if let Some(level) = self
            .levels
            .iter_mut()
            .rev()
            .find(|level| level.covers(px, py))
            && let Some(at) = level.entry_at(px, py)
            && level.menu.items[at].is_selectable()
        {
            level.selected = Some(at);
        }
    }

    /// Draws the boxes over what is on a screen SCREEN cells large, in the
    /// faces of THEME.
    pub fn draw<W: Write>(
        &self,
        out: &mut W,
        theme: &Theme,
        depth: ColorDepth,
        screen: (usize, usize),
    ) -> std::io::Result<()> {
        let styles = MenuStyles::of(theme);
        let base = page(theme);
        let clip = Clip {
            width: screen.0 as isize,
            height: screen.1 as isize,
            top: 0,
        };
        for level in &self.levels {
            level.draw(out, &styles, &base, depth, clip)?;
        }
        Ok(())
    }
}
