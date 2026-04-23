use arboard::Clipboard;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tracing::{debug, warn};

/// Hash clipboard content to a hex string.
pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Shared state tracking the last hash set by remote sync to prevent ping-pong.
#[derive(Clone)]
pub struct ClipboardMonitor {
    last_remote_hash: Arc<Mutex<String>>,
    paused: Arc<AtomicBool>,
}

impl ClipboardMonitor {
    pub fn new() -> Self {
        Self {
            last_remote_hash: Arc::new(Mutex::new(String::new())),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn set_remote_hash(&self, hash: &str) {
        *self.last_remote_hash.lock().unwrap() = hash.to_string();
    }

    pub fn set_clipboard(&self, content: &str) -> Result<(), String> {
        if self.is_paused() {
            return Ok(());
        }
        let hash = hash_content(content);
        self.set_remote_hash(&hash);

        let mut clip = Clipboard::new().map_err(|e| e.to_string())?;
        clip.set_text(content).map_err(|e| e.to_string())?;
        debug!("clipboard set from remote, hash={}", &hash[..12]);
        Ok(())
    }

    pub async fn watch_changes(&self, tx: watch::Sender<Option<(String, String)>>) {
        let mut last_hash = String::new();

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            if self.is_paused() {
                continue;
            }

            let content = {
                let mut clip = match Clipboard::new() {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("clipboard open error: {e}");
                        continue;
                    }
                };
                match clip.get_text() {
                    Ok(t) => t,
                    Err(_) => continue,
                }
            };

            let h = hash_content(&content);
            if h == last_hash {
                continue;
            }
            last_hash = h.clone();

            {
                let remote = self.last_remote_hash.lock().unwrap();
                if *remote == h {
                    debug!("skipping change from remote sync");
                    continue;
                }
            }

            debug!("local clipboard changed, hash={}", &h[..12]);
            let _ = tx.send(Some((content, h)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_content_deterministic() {
        let h1 = hash_content("hello world");
        let h2 = hash_content("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_content_different_for_different_input() {
        let h1 = hash_content("hello");
        let h2 = hash_content("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_content_is_hex_64_chars() {
        let h = hash_content("test");
        assert_eq!(h.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_content_empty_string() {
        let h = hash_content("");
        assert_eq!(h.len(), 64);
        // SHA-256 of "" is well known
        assert_eq!(h, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn monitor_pause_default_false() {
        let mon = ClipboardMonitor::new();
        assert!(!mon.is_paused());
    }

    #[test]
    fn monitor_set_pause() {
        let mon = ClipboardMonitor::new();
        mon.set_paused(true);
        assert!(mon.is_paused());
        mon.set_paused(false);
        assert!(!mon.is_paused());
    }

    #[test]
    fn monitor_pause_is_shared_across_clones() {
        let mon1 = ClipboardMonitor::new();
        let mon2 = mon1.clone();
        mon1.set_paused(true);
        assert!(mon2.is_paused());
    }

    #[test]
    fn monitor_set_remote_hash() {
        let mon = ClipboardMonitor::new();
        mon.set_remote_hash("abc123");
        let stored = mon.last_remote_hash.lock().unwrap().clone();
        assert_eq!(stored, "abc123");
    }

    #[test]
    fn monitor_set_clipboard_skips_when_paused() {
        let mon = ClipboardMonitor::new();
        mon.set_paused(true);
        // Should return Ok without touching clipboard
        assert!(mon.set_clipboard("anything").is_ok());
        // Remote hash should NOT be set since we skipped
        let stored = mon.last_remote_hash.lock().unwrap().clone();
        assert!(stored.is_empty());
    }
}
