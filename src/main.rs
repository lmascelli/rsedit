mod tui;
use tui::tui_main;
use std::io::{self, IsTerminal};

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    if io::stdin().is_terminal() {
        tui_main()
    } else {
        Ok(())
    }
}
