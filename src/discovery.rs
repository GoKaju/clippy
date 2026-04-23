use std::net::UdpSocket;
use std::time::Duration;
use tracing::{debug, info, warn};

pub const DISCOVERY_PORT: u16 = 9877;
pub const MAGIC: &[u8] = b"CLIPPY_SYNC_V1";

/// Build the beacon message for a given port.
pub fn beacon_message(ws_port: u16) -> String {
    format!("{}:{}", std::str::from_utf8(MAGIC).unwrap(), ws_port)
}

/// Parse a beacon message, returning the WS port if valid.
pub fn parse_beacon(data: &str) -> Option<u16> {
    let prefix = format!("{}:", std::str::from_utf8(MAGIC).unwrap());
    data.strip_prefix(&prefix)?.parse().ok()
}

/// Server: periodically broadcast presence on the LAN.
/// Sends "CLIPPY_SYNC_V1:<ws_port>" every 2 seconds via UDP broadcast.
pub fn start_beacon(ws_port: u16) {
    std::thread::spawn(move || {
        let socket = match UdpSocket::bind("0.0.0.0:0") {
            Ok(s) => s,
            Err(e) => {
                warn!("discovery beacon bind failed: {e}");
                return;
            }
        };
        let _ = socket.set_broadcast(true);

        let msg = beacon_message(ws_port);
        let addr = format!("255.255.255.255:{DISCOVERY_PORT}");

        loop {
            if let Err(e) = socket.send_to(msg.as_bytes(), &addr) {
                debug!("beacon send error: {e}");
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Client: scan the LAN for a server. Returns the first found "ip:ws_port".
/// Blocks for up to `timeout` duration.
pub fn find_server(timeout: Duration) -> Option<String> {
    let socket = match UdpSocket::bind(format!("0.0.0.0:{DISCOVERY_PORT}")) {
        Ok(s) => s,
        Err(e) => {
            warn!("discovery scan bind failed: {e}");
            return None;
        }
    };
    let _ = socket.set_read_timeout(Some(timeout));

    let mut buf = [0u8; 128];
    info!("scanning for clipboard-sync server...");

    loop {
        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                let data = std::str::from_utf8(&buf[..len]).unwrap_or("");
                if let Some(port) = parse_beacon(data) {
                    let addr = format!("{}:{}", src.ip(), port);
                    info!("found server at {addr}");
                    return Some(addr);
                }
            }
            Err(_) => {
                info!("no server found within timeout");
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_message_format() {
        let msg = beacon_message(9876);
        assert_eq!(msg, "CLIPPY_SYNC_V1:9876");
    }

    #[test]
    fn beacon_message_different_port() {
        let msg = beacon_message(1234);
        assert_eq!(msg, "CLIPPY_SYNC_V1:1234");
    }

    #[test]
    fn parse_beacon_valid() {
        assert_eq!(parse_beacon("CLIPPY_SYNC_V1:9876"), Some(9876));
        assert_eq!(parse_beacon("CLIPPY_SYNC_V1:1234"), Some(1234));
        assert_eq!(parse_beacon("CLIPPY_SYNC_V1:65535"), Some(65535));
    }

    #[test]
    fn parse_beacon_invalid_prefix() {
        assert_eq!(parse_beacon("WRONG_MAGIC:9876"), None);
        assert_eq!(parse_beacon("CLIPPY_SYNC_V2:9876"), None);
        assert_eq!(parse_beacon("9876"), None);
        assert_eq!(parse_beacon(""), None);
    }

    #[test]
    fn parse_beacon_invalid_port() {
        assert_eq!(parse_beacon("CLIPPY_SYNC_V1:notaport"), None);
        assert_eq!(parse_beacon("CLIPPY_SYNC_V1:"), None);
        assert_eq!(parse_beacon("CLIPPY_SYNC_V1:99999"), None); // > u16::MAX
    }

    #[test]
    fn roundtrip_beacon() {
        let msg = beacon_message(9876);
        let port = parse_beacon(&msg);
        assert_eq!(port, Some(9876));
    }

    #[test]
    fn udp_loopback_beacon() {
        // Send a beacon to localhost and verify find logic would parse it
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        let recv_addr = receiver.local_addr().unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        let msg = beacon_message(9876);
        sender.send_to(msg.as_bytes(), recv_addr).unwrap();

        let mut buf = [0u8; 128];
        let (len, _src) = receiver.recv_from(&mut buf).unwrap();
        let data = std::str::from_utf8(&buf[..len]).unwrap();
        assert_eq!(parse_beacon(data), Some(9876));
    }
}
