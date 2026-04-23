pub mod autostart;
pub mod clipboard;
pub mod client;
pub mod discovery;
pub mod protocol;
pub mod server;
pub mod tray;

use clap::{Parser, Subcommand};
use clipboard::ClipboardMonitor;
use tracing::error;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "clippy", about = "Bidirectional clipboard sync over WebSocket")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Run without system tray (headless)
    #[arg(long, global = true)]
    pub headless: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start as WebSocket server
    Serve {
        #[arg(short, long, default_value_t = 9876)]
        port: u16,
    },
    /// Connect to a running server
    Connect {
        /// Server address (e.g. 192.168.1.50:9876). Omit to auto-discover.
        addr: Option<String>,
    },
}

pub fn run(force_headless: bool) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let headless = cli.headless || force_headless;
    let monitor = ClipboardMonitor::new();

    match cli.command {
        // No subcommand: open tray paused, let user pick mode from menu
        None => {
            if headless {
                error!("No subcommand given. Use `serve` or `connect` in headless mode.");
                std::process::exit(1);
            }
            monitor.set_paused(true);
            tray::run_tray(monitor, tray::Mode::Idle);
        }

        Some(Command::Serve { port }) => {
            let client_count = server::new_client_count();
            let mon = monitor.clone();
            let cc = client_count.clone();

            discovery::start_beacon(port);

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(async {
                    if let Err(e) = server::run(port, mon, cc).await {
                        error!("server error: {e}");
                        std::process::exit(1);
                    }
                });
            });

            if headless {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            } else {
                tray::run_tray(monitor, tray::Mode::Server { port, client_count });
            }
        }

        Some(Command::Connect { addr }) => {
            let addr = match addr {
                Some(a) => a,
                None => {
                    let found = discovery::find_server(std::time::Duration::from_secs(5));
                    match found {
                        Some(a) => a,
                        None => {
                            error!("No server found on the network. Specify an address or start a server first.");
                            std::process::exit(1);
                        }
                    }
                }
            };

            let mon = monitor.clone();
            let addr_clone = addr.clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(async {
                    let _ = client::run(&addr_clone, mon).await;
                });
            });

            if headless {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            } else {
                tray::run_tray(monitor, tray::Mode::Client { addr });
            }
        }
    }

    Ok(())
}
