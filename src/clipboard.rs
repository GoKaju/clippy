use arboard::Clipboard;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use image::codecs::png::PngEncoder;
use image::{ImageBuffer, ImageEncoder, RgbaImage};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tracing::{debug, warn};

/// A clipboard change detected locally — either text or image.
#[derive(Clone, Debug)]
pub enum ClipboardContent {
    Text { content: String, hash: String },
    Image { data_b64: String, hash: String },
}

/// Hash text content to a hex string.
pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Hash raw bytes to a hex string.
pub fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Encode RGBA pixel data to a PNG and return the bytes.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let img: RgbaImage = ImageBuffer::from_raw(width, height, rgba.to_vec())?;
    let mut buf = Vec::new();
    PngEncoder::new(&mut buf)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(buf)
}

/// Shared state tracking the last hash set by remote sync to prevent ping-pong.
#[derive(Clone)]
pub struct ClipboardMonitor {
    last_remote_text_hash: Arc<Mutex<String>>,
    last_remote_image_hash: Arc<Mutex<String>>,
    paused: Arc<AtomicBool>,
}

impl ClipboardMonitor {
    pub fn new() -> Self {
        Self {
            last_remote_text_hash: Arc::new(Mutex::new(String::new())),
            last_remote_image_hash: Arc::new(Mutex::new(String::new())),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    fn set_remote_text_hash(&self, hash: &str) {
        *self.last_remote_text_hash.lock().unwrap() = hash.to_string();
    }

    fn set_remote_image_hash(&self, hash: &str) {
        *self.last_remote_image_hash.lock().unwrap() = hash.to_string();
    }

    fn is_remote_text_hash(&self, hash: &str) -> bool {
        *self.last_remote_text_hash.lock().unwrap() == hash
    }

    fn is_remote_image_hash(&self, hash: &str) -> bool {
        *self.last_remote_image_hash.lock().unwrap() == hash
    }

    /// Set clipboard to text from a remote peer.
    pub fn set_clipboard(&self, content: &str) -> Result<(), String> {
        if self.is_paused() {
            return Ok(());
        }
        let hash = hash_content(content);
        self.set_remote_text_hash(&hash);

        let mut clip = Clipboard::new().map_err(|e| e.to_string())?;
        clip.set_text(content).map_err(|e| e.to_string())?;
        debug!("clipboard set from remote (text), hash={}", &hash[..12]);
        Ok(())
    }

    /// Set clipboard to an image from a remote peer (base64-encoded PNG).
    /// Recomputes the hash from decoded RGBA pixels for consistency with the sender.
    pub fn set_clipboard_image(&self, data_b64: &str, _hash: &str) -> Result<(), String> {
        if self.is_paused() {
            return Ok(());
        }

        let png_bytes = BASE64.decode(data_b64).map_err(|e| format!("base64 decode: {e}"))?;
        let img = image::load_from_memory(&png_bytes).map_err(|e| format!("image decode: {e}"))?;
        let rgba = img.to_rgba8();
        let raw = rgba.clone().into_raw();
        let hash = hash_bytes(&raw);

        let img_data = arboard::ImageData {
            width: rgba.width() as usize,
            height: rgba.height() as usize,
            bytes: std::borrow::Cow::Owned(raw),
        };

        let mut clip = Clipboard::new().map_err(|e| e.to_string())?;
        clip.set_image(img_data).map_err(|e| e.to_string())?;
        self.set_remote_image_hash(&hash);
        debug!("clipboard set from remote (image), hash={}", &hash[..12]);
        Ok(())
    }

    /// Poll the clipboard for changes. Sends text or image changes through the channel.
    /// Uses separate hashes for text and image to avoid one type masking the other.
    /// Image hashes are computed from raw RGBA pixels (deterministic across platforms).
    /// Exits when all receivers are dropped (e.g. on reconnect).
    pub async fn watch_changes(&self, tx: watch::Sender<Option<ClipboardContent>>) {
        let mut last_text_hash = String::new();
        let mut last_image_hash = String::new();

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            // Stop polling if no receivers remain (connection dropped).
            if tx.is_closed() {
                debug!("watch_changes: all receivers dropped, exiting");
                return;
            }

            if self.is_paused() {
                continue;
            }

            let mut clip = match Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    warn!("clipboard open error: {e}");
                    continue;
                }
            };

            // Check for image changes
            let mut sent_image = false;
            if let Ok(img) = clip.get_image() {
                // Hash the raw RGBA bytes — deterministic regardless of PNG encoding
                let h = hash_bytes(&img.bytes);
                if h != last_image_hash {
                    last_image_hash = h.clone();
                    if !self.is_remote_image_hash(&h) {
                        if let Some(png_bytes) = encode_png(img.width as u32, img.height as u32, &img.bytes) {
                            let data_b64 = BASE64.encode(&png_bytes);
                            debug!("local clipboard changed (image), hash={}", &h[..12]);
                            let _ = tx.send(Some(ClipboardContent::Image { data_b64, hash: h }));
                            sent_image = true;
                        } else {
                            warn!("failed to encode clipboard image as PNG");
                        }
                    } else {
                        debug!("skipping image change from remote sync");
                    }
                }
            }

            if sent_image {
                continue;
            }

            // Check for text changes
            match clip.get_text() {
                Ok(t) => {
                    let h = hash_content(&t);
                    if h == last_text_hash {
                        continue;
                    }
                    last_text_hash = h.clone();

                    if self.is_remote_text_hash(&h) {
                        debug!("skipping text change from remote sync");
                        continue;
                    }

                    debug!("local clipboard changed (text), hash={}", &h[..12]);
                    let _ = tx.send(Some(ClipboardContent::Text { content: t, hash: h }));
                }
                Err(_) => continue,
            }
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
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_content_empty_string() {
        let h = hash_content("");
        assert_eq!(h.len(), 64);
        assert_eq!(h, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn hash_bytes_deterministic() {
        let h1 = hash_bytes(b"hello");
        let h2 = hash_bytes(b"hello");
        assert_eq!(h1, h2);
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
        mon.set_remote_text_hash("abc123");
        let stored = mon.last_remote_text_hash.lock().unwrap().clone();
        assert_eq!(stored, "abc123");

        mon.set_remote_image_hash("img456");
        let stored = mon.last_remote_image_hash.lock().unwrap().clone();
        assert_eq!(stored, "img456");
    }

    #[test]
    fn monitor_set_clipboard_skips_when_paused() {
        let mon = ClipboardMonitor::new();
        mon.set_paused(true);
        assert!(mon.set_clipboard("anything").is_ok());
        let stored = mon.last_remote_text_hash.lock().unwrap().clone();
        assert!(stored.is_empty());
    }

    #[test]
    fn encode_png_roundtrip() {
        // 2x2 red image
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
        let png = encode_png(2, 2, &rgba).expect("encode should work");
        assert!(!png.is_empty());
        // Verify it's valid PNG
        let img = image::load_from_memory(&png).expect("should decode back");
        assert_eq!(img.width(), 2);
        assert_eq!(img.height(), 2);
    }
}
