pub enum MinibufferPlacement {
    Floating {
        rect: crate::ui::Rect,
        has_border: bool,
    },
    Split {
        height: usize,
    },
}

pub struct MinibufferState {
    pub window: crate::ui::Window,
    pub placement: MinibufferPlacement,
    pub previous_focused_window_id: usize,
}

impl Default for MinibufferState {
    fn default() -> Self {
        Self {
            window: crate::ui::Window {
                id: 0,
                buffer_name: "*Minibuffer*".into(),
                scroll_x: 0,
                scroll_y: 0,
            },
            placement: MinibufferPlacement::Split { height: 3 },
            previous_focused_window_id: 0,
        }
    }
}
