use crate::autostart::AutoStart;
use crate::clipboard::ClipboardMonitor;
use crate::discovery;
use crate::server::{self, ClientCount};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder, TrayIcon};

const ICON_IDLE: &[u8] = include_bytes!("../assets/idle.png");
const ICON_CONNECTED: &[u8] = include_bytes!("../assets/connected.png");
const ICON_PAUSED: &[u8] = include_bytes!("../assets/paused.png");

#[derive(Clone)]
pub enum Mode {
    Idle, // No command given, user picks from menu
    Server { port: u16, client_count: ClientCount },
    Client { addr: String },
}

/// Runtime state that can change when user picks a mode from the menu.
struct RuntimeState {
    mode: Mode,
    client_count: Option<ClientCount>,
}

fn load_icon(png_data: &[u8]) -> Icon {
    let img = image::load_from_memory(png_data)
        .expect("decode icon png")
        .into_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).expect("create icon")
}

fn icon_idle() -> Icon { load_icon(ICON_IDLE) }
fn icon_connected() -> Icon { load_icon(ICON_CONNECTED) }
fn icon_paused() -> Icon { load_icon(ICON_PAUSED) }

fn get_local_ip() -> String {
    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn has_clients(state: &RuntimeState) -> bool {
    match &state.mode {
        Mode::Server { client_count, .. } => client_count.load(Ordering::Relaxed) > 0,
        Mode::Client { .. } => true,
        Mode::Idle => false,
    }
}

fn mode_to_args(mode: &Mode) -> Vec<String> {
    match mode {
        Mode::Server { port, .. } => vec!["serve".into(), "--port".into(), port.to_string()],
        Mode::Client { addr } => vec!["connect".into(), addr.clone()],
        Mode::Idle => vec![],
    }
}

fn update_tray_icon(tray: &TrayIcon, monitor: &ClipboardMonitor, state: &RuntimeState) {
    let icon = if monitor.is_paused() {
        icon_paused()
    } else if has_clients(state) {
        icon_connected()
    } else {
        icon_idle()
    };
    let _ = tray.set_icon(Some(icon));
}

pub fn run_tray(monitor: ClipboardMonitor, initial_mode: Mode) {
    let event_loop: EventLoop<()> = EventLoopBuilder::new().build();

    let state = Arc::new(Mutex::new(RuntimeState {
        mode: initial_mode.clone(),
        client_count: match &initial_mode {
            Mode::Server { client_count, .. } => Some(client_count.clone()),
            _ => None,
        },
    }));

    let menu = Menu::new();

    let status_item = MenuItem::new("Clippy — idle", false, None);
    let start_server_item = MenuItem::new("Start as Server", true, None);
    let connect_item = MenuItem::new("Connect (auto-discover)", true, None);
    let pause_item = MenuItem::new("Pause sync", true, None);
    let copy_ip_item = MenuItem::new(&format!("Copy IP: {}", get_local_ip()), true, None);
    let startup_item = MenuItem::new("Start at login", true, None);
    let quit_item = MenuItem::new("Quit", true, None);

    // Initially hide pause if idle
    let is_idle = matches!(initial_mode, Mode::Idle);
    if !is_idle {
        start_server_item.set_enabled(false);
        connect_item.set_enabled(false);
    }
    pause_item.set_enabled(!is_idle);

    // Set initial status
    if !is_idle {
        match &initial_mode {
            Mode::Server { port, .. } => status_item.set_text(&format!("Server :{port} | 0 clients")),
            Mode::Client { addr } => status_item.set_text(&format!("Connected to {addr}")),
            Mode::Idle => {}
        }
    }

    menu.append_items(&[
        &status_item,
        &PredefinedMenuItem::separator(),
        &start_server_item,
        &connect_item,
        &PredefinedMenuItem::separator(),
        &pause_item,
        &copy_ip_item,
        &startup_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ])
    .expect("append menu items");

    // Check autostart state
    let autostart = AutoStart::new(mode_to_args(&initial_mode));
    if autostart.is_enabled() {
        startup_item.set_text("Disable start at login");
    }

    let initial_icon = if matches!(initial_mode, Mode::Idle) { icon_paused() } else { icon_idle() };
    let _tray = TrayIconBuilder::new()
        .with_icon(initial_icon)
        .with_menu(Box::new(menu))
        .with_tooltip("Clippy — clipboard sync")
        .build()
        .expect("create tray icon");

    let server_id = start_server_item.id().clone();
    let connect_id = connect_item.id().clone();
    let pause_id = pause_item.id().clone();
    let copy_ip_id = copy_ip_item.id().clone();
    let startup_id = startup_item.id().clone();
    let quit_id = quit_item.id().clone();

    let mut last_status_update = Instant::now();

    event_loop.run(move |_event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(100),
        );

        // Periodic status update
        if last_status_update.elapsed().as_secs() >= 2 {
            last_status_update = Instant::now();
            let st = state.lock().unwrap();
            match &st.mode {
                Mode::Server { port, client_count } => {
                    let n = client_count.load(Ordering::Relaxed);
                    status_item.set_text(&format!(
                        "Server :{port} | {n} client{}",
                        if n == 1 { "" } else { "s" }
                    ));
                }
                Mode::Client { addr } => {
                    status_item.set_text(&format!("Connected to {addr}"));
                }
                Mode::Idle => {
                    status_item.set_text("Clippy — idle");
                }
            }

            if !monitor.is_paused() {
                update_tray_icon(&_tray, &monitor, &st);
            }
        }

        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == server_id {
                // Start as server
                let port: u16 = 9876;
                let cc = server::new_client_count();
                let mon = monitor.clone();
                let cc2 = cc.clone();

                monitor.set_paused(false);
                discovery::start_beacon(port);

                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                    rt.block_on(async {
                        if let Err(e) = server::run(port, mon, cc2).await {
                            tracing::error!("server error: {e}");
                        }
                    });
                });

                {
                    let mut st = state.lock().unwrap();
                    st.mode = Mode::Server { port, client_count: cc.clone() };
                    st.client_count = Some(cc);
                }

                start_server_item.set_enabled(false);
                connect_item.set_enabled(false);
                pause_item.set_enabled(true);
                pause_item.set_text("Pause sync");
                status_item.set_text(&format!("Server :{port} | 0 clients"));
                let _ = _tray.set_icon(Some(icon_idle()));

            } else if event.id == connect_id {
                // Auto-discover and connect
                status_item.set_text("Scanning for server...");
                connect_item.set_enabled(false);
                start_server_item.set_enabled(false);

                let mon = monitor.clone();
                let state2 = state.clone();

                // Do discovery + connect in background
                std::thread::spawn({
                    let mon2 = mon.clone();
                    move || {
                        let found = discovery::find_server(Duration::from_secs(5));
                        match found {
                            Some(addr) => {
                                mon2.set_paused(false);
                                let mon3 = mon2.clone();
                                let addr2 = addr.clone();

                                std::thread::spawn(move || {
                                    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                                    rt.block_on(async {
                                        if let Err(e) = crate::client::run(&addr2, mon3).await {
                                            tracing::error!("client error: {e}");
                                        }
                                    });
                                });

                                let mut st = state2.lock().unwrap();
                                st.mode = Mode::Client { addr };
                            }
                            None => {
                                tracing::warn!("no server found");
                                // Will be picked up by status update, re-enable buttons
                                let mut st = state2.lock().unwrap();
                                st.mode = Mode::Idle;
                            }
                        }
                    }
                });

                // Menu updates happen in the periodic tick based on state

            } else if event.id == pause_id {
                let is_paused = monitor.is_paused();
                monitor.set_paused(!is_paused);
                let st = state.lock().unwrap();
                if is_paused {
                    pause_item.set_text("Pause sync");
                    update_tray_icon(&_tray, &monitor, &st);
                } else {
                    pause_item.set_text("Resume sync");
                    let _ = _tray.set_icon(Some(icon_paused()));
                }

            } else if event.id == startup_id {
                let st = state.lock().unwrap();
                let autostart = AutoStart::new(mode_to_args(&st.mode));
                if autostart.is_enabled() {
                    match autostart.disable() {
                        Ok(_) => startup_item.set_text("Start at login"),
                        Err(e) => tracing::error!("autostart disable: {e}"),
                    }
                } else {
                    match autostart.enable() {
                        Ok(_) => startup_item.set_text("Disable start at login"),
                        Err(e) => tracing::error!("autostart enable: {e}"),
                    }
                }

            } else if event.id == copy_ip_id {
                let ip = get_local_ip();
                if let Ok(mut clip) = arboard::Clipboard::new() {
                    let st = state.lock().unwrap();
                    let text = match &st.mode {
                        Mode::Server { port, .. } => format!("{ip}:{port}"),
                        _ => ip,
                    };
                    let _ = clip.set_text(text);
                }

            } else if event.id == quit_id {
                *control_flow = ControlFlow::Exit;
                std::process::exit(0);
            }
        }

        // Re-enable buttons if back to idle (discovery failed)
        {
            let st = state.lock().unwrap();
            if matches!(st.mode, Mode::Idle) {
                if !start_server_item.is_enabled() {
                    start_server_item.set_enabled(true);
                    connect_item.set_enabled(true);
                    pause_item.set_enabled(false);
                }
            }
        }
    });
}
