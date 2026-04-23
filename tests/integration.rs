use std::time::Duration;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tokio_tungstenite::{accept_async, connect_async};
use futures_util::{SinkExt, StreamExt};

/// Test that a WebSocket server and client can exchange clipboard update messages.
#[tokio::test]
async fn server_client_message_exchange() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Server task: accept one connection, receive a message, send one back
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = accept_async(stream).await.unwrap();
        let (mut tx, mut rx) = ws.split();

        // Receive message from client
        let msg = rx.next().await.unwrap().unwrap();
        let text = match msg {
            WsMessage::Text(t) => t.to_string(),
            other => panic!("expected text, got {:?}", other),
        };
        assert!(text.contains("ClipboardUpdate"));
        assert!(text.contains("hello from client"));

        // Send reply
        let reply = r#"{"type":"ClipboardUpdate","content":"hello from server","hash":"s1"}"#;
        tx.send(WsMessage::Text(reply.into())).await.unwrap();
    });

    // Give server a moment to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Client task
    let url = format!("ws://{}", addr);
    let (ws, _) = connect_async(&url).await.unwrap();
    let (mut tx, mut rx) = ws.split();

    // Send a clipboard update
    let msg = r#"{"type":"ClipboardUpdate","content":"hello from client","hash":"c1"}"#;
    tx.send(WsMessage::Text(msg.into())).await.unwrap();

    // Receive reply
    let reply = rx.next().await.unwrap().unwrap();
    let text = match reply {
        WsMessage::Text(t) => t.to_string(),
        other => panic!("expected text, got {:?}", other),
    };
    assert!(text.contains("hello from server"));

    server.await.unwrap();
}

/// Test that multiple clients can connect to the same server.
#[tokio::test]
async fn multiple_clients_connect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        // Accept 3 clients
        for _ in 0..3 {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = accept_async(stream).await.unwrap();
            let (mut tx, _rx) = ws.split();
            let msg = r#"{"type":"Ack","hash":"ok"}"#;
            tx.send(WsMessage::Text(msg.into())).await.unwrap();
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect 3 clients
    for _ in 0..3 {
        let url = format!("ws://{}", addr);
        let (ws, _) = connect_async(&url).await.unwrap();
        let (_tx, mut rx) = ws.split();
        let msg = rx.next().await.unwrap().unwrap();
        let text = match msg {
            WsMessage::Text(t) => t.to_string(),
            other => panic!("expected text, got {:?}", other),
        };
        assert!(text.contains("Ack"));
    }

    server.await.unwrap();
}

/// Test the protocol message roundtrip through WebSocket.
#[tokio::test]
async fn protocol_over_websocket() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = accept_async(stream).await.unwrap();
        let (mut tx, mut rx) = ws.split();

        // Echo back whatever we receive
        while let Some(Ok(msg)) = rx.next().await {
            if let WsMessage::Text(_) = &msg {
                tx.send(msg).await.unwrap();
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let url = format!("ws://{}", addr);
    let (ws, _) = connect_async(&url).await.unwrap();
    let (mut tx, mut rx) = ws.split();

    // Test ClipboardUpdate
    let update = r#"{"type":"ClipboardUpdate","content":"test content","hash":"abc"}"#;
    tx.send(WsMessage::Text(update.into())).await.unwrap();
    let echo = rx.next().await.unwrap().unwrap();
    assert_eq!(echo, WsMessage::Text(update.into()));

    // Test Ack
    let ack = r#"{"type":"Ack","hash":"xyz"}"#;
    tx.send(WsMessage::Text(ack.into())).await.unwrap();
    let echo = rx.next().await.unwrap().unwrap();
    assert_eq!(echo, WsMessage::Text(ack.into()));

    // Close the connection so server loop exits
    tx.send(WsMessage::Close(None)).await.unwrap();
    drop(tx);
    drop(rx);
    server.await.unwrap();
}
