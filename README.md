# Clippy — Bidirectional Clipboard Sync

Sync your clipboard between Mac and Windows (or any two machines) in real time over WebSocket.

Copy on one machine, paste on the other. That simple.

## Features

- **Auto-sync bidirectional** — clipboard changes are detected every 500ms and pushed instantly via WebSocket
- **Auto-discovery** — the client finds the server automatically on the LAN via UDP broadcast (no need to type IPs)
- **System tray / menu bar** — native icon on macOS menu bar and Windows system tray with status, controls, and mode selection
- **Pause / Resume** — toggle sync from the tray menu
- **Start at login** — one-click autostart setup from the tray (LaunchAgent on Mac, Registry on Windows)
- **Anti ping-pong** — hash-based dedup prevents infinite loops when clipboard is set remotely
- **Multiple clients** — the server accepts N concurrent connections
- **Headless mode** — run without GUI on servers or CI with `--headless`
- **Cross-platform** — single codebase, compiles natively for macOS and Windows

## Quick Start

### Option 1: No arguments (interactive)

```bash
./clippy
```

Opens the tray icon in idle mode. Pick **Start as Server** or **Connect (auto-discover)** from the menu.

### Option 2: CLI

```bash
# Machine A (server)
./clippy serve --port 9876

# Machine B (client, auto-discover)
./clippy connect

# Machine B (client, manual IP)
./clippy connect 192.168.1.50:9876
```

### Headless (no tray)

```bash
./clippy --headless serve --port 9876
```

## Tray Menu

| Item | Description |
|------|-------------|
| Status | Shows mode, port, and connected client count |
| Start as Server | Starts WebSocket server + UDP beacon |
| Connect (auto-discover) | Scans LAN for a server and connects |
| Pause sync / Resume sync | Toggles clipboard monitoring |
| Copy IP | Copies `IP:port` to clipboard |
| Start at login | Registers/removes autostart |
| Quit | Exits the app |

### Tray Icons

| Icon | State |
|------|-------|
| Idle (blue) | Server running, no clients connected |
| Connected (green) | At least one client connected |
| Paused (orange) | Sync is paused |

## How It Works

```
Mac                                    Windows
┌─────────────────────┐                ┌─────────────────────┐
│  clippy              │   WebSocket    │  clippy              │
│  - poll clipboard   │◄──────────────►│  - poll clipboard   │
│    every 500ms      │   push changes │    every 500ms      │
│  - WS server        │                │  - WS client        │
│  - UDP beacon       │                │  - UDP discovery    │
└─────────────────────┘                └─────────────────────┘
```

1. Both machines run the same binary
2. One acts as server (`clippy serve`), the other as client (`clippy connect`)
3. Every 500ms each side checks if the clipboard changed (SHA-256 hash comparison)
4. If changed → sends the new content over WebSocket
5. The other side receives → sets its local clipboard
6. Anti ping-pong: ignores changes that match the last remotely-received hash

### Auto-Discovery

The server broadcasts a UDP beacon every 2 seconds on port 9877:

```
CLIPPY_SYNC_V1:9876
```

The client listens on that port, extracts the server IP and WebSocket port, and connects automatically.

## Build

```bash
# macOS native
cargo build --release

# Windows cross-compile from Mac
rustup target add x86_64-pc-windows-gnu
brew install mingw-w64
cargo build --release --target x86_64-pc-windows-gnu
```

Binaries:
- Mac: `target/release/clippy`
- Windows GUI (no console): `target/x86_64-pc-windows-gnu/release/clippy.exe`
- Windows console / headless: `target/x86_64-pc-windows-gnu/release/clippy-headless.exe`

## Project Structure

```
clippy/
├── Cargo.toml
├── assets/
│   ├── idle.png          # Tray icon: server idle
│   ├── connected.png     # Tray icon: clients connected
│   └── paused.png        # Tray icon: sync paused
└── src/
    ├── lib.rs            # Shared CLI logic, parsing, and app entry
    ├── main.rs           # GUI binary (no console on Windows)
    ├── main_headless.rs  # Console binary for servers / CI
    ├── clipboard.rs      # Poll clipboard, detect changes, anti ping-pong
    ├── server.rs         # WebSocket server, multi-client
    ├── client.rs         # WebSocket client
    ├── protocol.rs       # Message types (ClipboardUpdate, Ack)
    ├── discovery.rs      # UDP beacon (server) + scan (client)
    ├── tray.rs           # System tray with menu and mode selection
    └── autostart.rs      # Platform-specific start-at-login
```

## Dependencies

- **arboard** — cross-platform clipboard access
- **tokio + tokio-tungstenite** — async WebSocket
- **tray-icon + tao** — native system tray
- **clap** — CLI argument parsing
- **sha2** — clipboard change detection
- **local-ip-address** — LAN IP detection
- **image** — PNG icon loading

## TODO

- [x] Auto-reconnect — automatically reconnect if the connection is lost
- [x] No console window — suppress shell/cmd window on launch (Windows `#![windows_subsystem = "windows"]`)
- [ ] App icon — set a proper `.ico` / `.icns` application icon for macOS and Windows
- [ ] Screenshot sync (copy a screenshot on one machine, paste it on the other)
- [ ] File sync (copy a file, paste it on the other machine)
