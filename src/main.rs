// On Windows, suppress the console window when launching the GUI binary.
// Use `clippy-headless` instead if you need stdout/stderr (servers, CI).
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    clippy::run(false)
}
