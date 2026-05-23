mod tui;
use tui::tui_main;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    tui_main()
}
