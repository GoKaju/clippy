use crate::clipboard::{ClipboardContent, ClipboardMonitor};
use crate::protocol::Message;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::watch;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tracing::{error, info, warn};

const RECONNECT_INITIAL_DELAY_MS: u64 = 1_000;
const RECONNECT_MAX_DELAY_MS: u64 = 30_000;
const RECONNECT_MULTIPLIER: u64 = 2;

/// Connect once and run until the connection drops.
async fn run_once(addr: &str, monitor: &ClipboardMonitor) -> anyhow::Result<()> {
    let url = format!("ws://{addr}");
    let (ws, _) = connect_async(&url).await?;
    info!("connected to {url}");

    let (mut ws_tx, mut ws_rx) = ws.split();
    let (tx, mut rx) = watch::channel::<Option<ClipboardContent>>(None);

    // Start clipboard polling
    let mon = monitor.clone();
    tokio::spawn(async move {
        mon.watch_changes(tx).await;
    });

    // Send local changes to server
    let send_handle = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let val = rx.borrow_and_update().clone();
            let msg = match val {
                Some(ClipboardContent::Text { content, hash }) => {
                    Message::ClipboardUpdate { content, hash }
                }
                Some(ClipboardContent::Image { data_b64, hash }) => {
                    Message::ImageUpdate { data: data_b64, hash }
                }
                None => continue,
            };
            if let Err(e) = ws_tx.send(WsMessage::Text(msg.encode().into())).await {
                error!("send error: {e}");
                break;
            }
        }
    });

    // Receive remote changes from server
    let recv_mon = monitor.clone();
    let recv_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            if let WsMessage::Text(text) = msg {
                match Message::decode(&text) {
                    Ok(Message::ClipboardUpdate { content, hash: _ }) => {
                        if let Err(e) = recv_mon.set_clipboard(&content) {
                            error!("set clipboard: {e}");
                        }
                    }
                    Ok(Message::ImageUpdate { data, hash }) => {
                        if let Err(e) = recv_mon.set_clipboard_image(&data, &hash) {
                            error!("set clipboard image: {e}");
                        }
                    }
                    Ok(Message::Ack { .. }) => {}
                    Err(e) => error!("decode error: {e}"),
                }
            }
        }
        info!("server disconnected");
    });

    tokio::select! {
        _ = send_handle => {}
        _ = recv_handle => {}
    }

    Ok(())
}

/// Connect to `addr` and automatically reconnect with exponential backoff
/// whenever the connection is lost. Backoff resets after a successful connection.
pub async fn run(addr: &str, monitor: ClipboardMonitor) -> anyhow::Result<()> {
    let mut delay_ms = RECONNECT_INITIAL_DELAY_MS;

    loop {
        match run_once(addr, &monitor).await {
            Ok(()) => {
                // Was connected successfully — reset backoff
                delay_ms = RECONNECT_INITIAL_DELAY_MS;
                warn!("connection lost, reconnecting in {delay_ms}ms...");
            }
            Err(e) => {
                warn!("connection error: {e}, reconnecting in {delay_ms}ms...");
            }
        }

        sleep(Duration::from_millis(delay_ms)).await;
        delay_ms = (delay_ms * RECONNECT_MULTIPLIER).min(RECONNECT_MAX_DELAY_MS);
    }
}
