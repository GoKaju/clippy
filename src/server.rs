use crate::clipboard::ClipboardMonitor;
use crate::protocol::Message;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tracing::{error, info};

/// Shared client counter for tray status display.
pub type ClientCount = Arc<AtomicUsize>;

pub fn new_client_count() -> ClientCount {
    Arc::new(AtomicUsize::new(0))
}

pub async fn run(port: u16, monitor: ClipboardMonitor, client_count: ClientCount) -> anyhow::Result<()> {
    let (tx, _rx) = watch::channel::<Option<(String, String)>>(None);

    let mon = monitor.clone();
    let poll_tx = tx.clone();
    tokio::spawn(async move {
        mon.watch_changes(poll_tx).await;
    });

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await?;
    info!("listening on {addr}");

    loop {
        let (stream, peer) = listener.accept().await?;
        info!("client connected: {peer}");

        let ws = match accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                error!("websocket handshake failed for {peer}: {e}");
                continue;
            }
        };

        client_count.fetch_add(1, Ordering::Relaxed);
        let monitor = monitor.clone();
        let rx = tx.subscribe();
        let cc = client_count.clone();

        tokio::spawn(async move {
            handle_client(ws, monitor, rx, peer).await;
            cc.fetch_sub(1, Ordering::Relaxed);
            info!("client disconnected: {peer}");
        });
    }
}

async fn handle_client(
    ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    monitor: ClipboardMonitor,
    mut rx: watch::Receiver<Option<(String, String)>>,
    peer: std::net::SocketAddr,
) {
    let (mut ws_tx, mut ws_rx) = ws.split();

    let send_handle = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let val = rx.borrow_and_update().clone();
            if let Some((content, hash)) = val {
                let msg = Message::ClipboardUpdate { content, hash };
                if let Err(e) = ws_tx.send(WsMessage::Text(msg.encode().into())).await {
                    error!("[{peer}] send error: {e}");
                    break;
                }
            }
        }
    });

    let recv_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            if let WsMessage::Text(text) = msg {
                match Message::decode(&text) {
                    Ok(Message::ClipboardUpdate { content, hash: _ }) => {
                        if let Err(e) = monitor.set_clipboard(&content) {
                            error!("[{peer}] set clipboard: {e}");
                        }
                    }
                    Ok(Message::Ack { .. }) => {}
                    Err(e) => error!("[{peer}] decode error: {e}"),
                }
            }
        }
    });

    tokio::select! {
        _ = send_handle => {}
        _ = recv_handle => {}
    }
}
