use serde::{Deserialize, Serialize};

/// Messages exchanged over the WebSocket connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    /// A clipboard text update from the remote peer.
    ClipboardUpdate { content: String, hash: String },
    /// A clipboard image update (base64-encoded PNG).
    ImageUpdate { data: String, hash: String },
    /// Acknowledgement of a received update.
    Ack { hash: String },
}

impl Message {
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("serialize message")
    }

    pub fn decode(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_clipboard_update() {
        let msg = Message::ClipboardUpdate {
            content: "hello".into(),
            hash: "abc123".into(),
        };
        let json = msg.encode();
        assert!(json.contains("\"type\":\"ClipboardUpdate\""));
        assert!(json.contains("\"content\":\"hello\""));
        assert!(json.contains("\"hash\":\"abc123\""));
    }

    #[test]
    fn encode_ack() {
        let msg = Message::Ack {
            hash: "def456".into(),
        };
        let json = msg.encode();
        assert!(json.contains("\"type\":\"Ack\""));
        assert!(json.contains("\"hash\":\"def456\""));
    }

    #[test]
    fn decode_clipboard_update() {
        let json = r#"{"type":"ClipboardUpdate","content":"test","hash":"h1"}"#;
        let msg = Message::decode(json).unwrap();
        match msg {
            Message::ClipboardUpdate { content, hash } => {
                assert_eq!(content, "test");
                assert_eq!(hash, "h1");
            }
            _ => panic!("expected ClipboardUpdate"),
        }
    }

    #[test]
    fn decode_ack() {
        let json = r#"{"type":"Ack","hash":"h2"}"#;
        let msg = Message::decode(json).unwrap();
        match msg {
            Message::Ack { hash } => assert_eq!(hash, "h2"),
            _ => panic!("expected Ack"),
        }
    }

    #[test]
    fn decode_invalid_json() {
        assert!(Message::decode("not json").is_err());
    }

    #[test]
    fn decode_missing_type() {
        let json = r#"{"content":"x","hash":"y"}"#;
        assert!(Message::decode(json).is_err());
    }

    #[test]
    fn roundtrip_clipboard_update() {
        let original = Message::ClipboardUpdate {
            content: "emoji: \u{1F600}".into(),
            hash: "abc".into(),
        };
        let decoded = Message::decode(&original.encode()).unwrap();
        match decoded {
            Message::ClipboardUpdate { content, hash } => {
                assert_eq!(content, "emoji: \u{1F600}");
                assert_eq!(hash, "abc");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_ack() {
        let original = Message::Ack { hash: "xyz".into() };
        let decoded = Message::decode(&original.encode()).unwrap();
        match decoded {
            Message::Ack { hash } => assert_eq!(hash, "xyz"),
            _ => panic!("wrong variant"),
        }
    }
}
