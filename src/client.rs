use crate::clipboard::ClipboardMonitor;
use crate::protocol::Message;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::watch;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tracing::{error, info};

pub async fn run(addr: &str, monitor: ClipboardMonitor) -> anyhow::Result<()> {
    let url = format!("ws://{addr}");
    info!("connecting to {url}");

    let (ws, _) = connect_async(&url).await?;
    info!("connected");

    let (mut ws_tx, mut ws_rx) = ws.split();
    let (tx, mut rx) = watch::channel::<Option<(String, String)>>(None);

    // Start clipboard polling
    let mon = monitor.clone();
    tokio::spawn(async move {
        mon.watch_changes(tx).await;
    });

    // Send local changes to server
    let send_handle = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let val = rx.borrow_and_update().clone();
            if let Some((content, hash)) = val {
                let msg = Message::ClipboardUpdate { content, hash };
                if let Err(e) = ws_tx.send(WsMessage::Text(msg.encode().into())).await {
                    error!("send error: {e}");
                    break;
                }
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
