// Headless entry point — no `windows_subsystem` attribute so stdout/stderr
// remain attached to the console on Windows. Always runs in headless mode.

fn main() -> anyhow::Result<()> {
    clippy::run(true)
}
