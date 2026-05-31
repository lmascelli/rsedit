mod tui;
use std::io::{self, IsTerminal};
use tui::tui_main;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let file_to_open = args.get(1).cloned();
    if io::stdin().is_terminal() {
        tui_main(file_to_open)
    } else {
        Ok(())
    }
}
