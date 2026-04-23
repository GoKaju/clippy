mod autostart;
mod clipboard;
mod client;
mod discovery;
mod protocol;
mod server;
mod tray;

use clap::{Parser, Subcommand};
use clipboard::ClipboardMonitor;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "clippy", about = "Bidirectional clipboard sync over WebSocket")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Run without system tray (headless)
    #[arg(long, global = true)]
    headless: bool,
}

#[derive(Subcommand)]
enum Command {
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

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let monitor = ClipboardMonitor::new();

    match cli.command {
        // No subcommand: open tray paused, let user pick mode from menu
        None => {
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
                        tracing::error!("server error: {e}");
                        std::process::exit(1);
                    }
                });
            });

            if cli.headless {
                loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
            } else {
                tray::run_tray(monitor, tray::Mode::Server { port, client_count });
            }
        }

        Some(Command::Connect { addr }) => {
            // If no addr given, auto-discover
            let addr = match addr {
                Some(a) => a,
                None => {
                    let found = discovery::find_server(std::time::Duration::from_secs(5));
                    match found {
                        Some(a) => a,
                        None => {
                            eprintln!("No server found on the network. Specify an address or start a server first.");
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

            if cli.headless {
                loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
            } else {
                tray::run_tray(monitor, tray::Mode::Client { addr });
            }
        }
    }

    Ok(())
}
