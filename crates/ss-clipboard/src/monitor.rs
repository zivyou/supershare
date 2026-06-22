use arboard::Clipboard;
use ss_core::protocol::{
    ClipboardContent, CLIPBOARD_POLL_INTERVAL_MS, CLIPBOARD_SUPPRESSION_MS,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task;

/// A clipboard change detected by the monitor
#[derive(Debug, Clone)]
pub struct ClipboardChange {
    pub content: ClipboardContent,
    pub hash: [u8; 32],
}

/// Clipboard monitor that polls for changes and emits events
pub struct ClipboardMonitor {
    /// Last known clipboard content hash
    last_hash: Option<[u8; 32]>,
    /// Timestamp of the last remote write (for suppression)
    last_remote_write: Option<Instant>,
    /// Remote hash to suppress (prevents re-sending content received from network)
    remote_hash: Option<[u8; 32]>,
}

impl ClipboardMonitor {
    pub fn new() -> Self {
        Self {
            last_hash: None,
            last_remote_write: None,
            remote_hash: None,
        }
    }

    /// Mark that a remote write just happened (suppress local detection for a while)
    pub fn suppress(&mut self, hash: [u8; 32]) {
        self.last_remote_write = Some(Instant::now());
        self.remote_hash = Some(hash);
    }

    /// Check if we should suppress local clipboard changes
    fn is_suppressed(&self, hash: &[u8; 32]) -> bool {
        if let Some(remote_hash) = &self.remote_hash {
            if hash == remote_hash {
                return true;
            }
        }
        if let Some(last_write) = self.last_remote_write {
            if last_write.elapsed() < Duration::from_millis(CLIPBOARD_SUPPRESSION_MS) {
                return true;
            }
        }
        false
    }

    /// Poll the clipboard once and return a change if detected
    pub fn poll(&mut self) -> Option<ClipboardChange> {
        let content = read_clipboard()?;
        let hash = content.hash();

        // Skip if suppressed (content came from remote)
        if self.is_suppressed(&hash) {
            return None;
        }

        // Skip if content hasn't changed
        if self.last_hash.as_ref() == Some(&hash) {
            return None;
        }

        self.last_hash = Some(hash);
        Some(ClipboardChange { content, hash })
    }
}

/// Read current clipboard content
fn read_clipboard() -> Option<ClipboardContent> {
    let mut clipboard = Clipboard::new().ok()?;

    // Try text first
    if let Ok(text) = clipboard.get_text() {
        if !text.is_empty() {
            return Some(ClipboardContent::Text(text));
        }
    }

    // Try image
    if let Ok(image) = clipboard.get_image() {
        let bytes: &[u8] = bytemuck::cast_slice(&image.bytes);
        return Some(ClipboardContent::Image {
            width: image.width as u32,
            height: image.height as u32,
            rgba: bytes.to_vec(),
        });
    }

    None
}

/// Write text to clipboard
pub fn write_clipboard_text(text: &str) -> anyhow::Result<()> {
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text.to_string())?;
    Ok(())
}

/// Write image to clipboard (RGBA pixels)
pub fn write_clipboard_image(width: u32, height: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let mut clipboard = Clipboard::new()?;
    let image = arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: std::borrow::Cow::Borrowed(bytemuck::cast_slice(rgba)),
    };
    clipboard.set_image(image)?;
    Ok(())
}

/// Start the clipboard polling monitor in a background task.
/// Returns a receiver for clipboard changes and a sender to signal suppression.
pub fn start_monitor(
    mut monitor: ClipboardMonitor,
) -> (mpsc::Receiver<ClipboardChange>, mpsc::Sender<[u8; 32]>) {
    let (change_tx, change_rx) = mpsc::channel(16);
    let (suppress_tx, mut suppress_rx) = mpsc::channel::<[u8; 32]>(16);

    tokio::spawn(async move {
        let poll_interval = Duration::from_millis(CLIPBOARD_POLL_INTERVAL_MS);

        loop {
            tokio::select! {
                // Handle suppression signals
                Some(hash) = suppress_rx.recv() => {
                    monitor.suppress(hash);
                }
                // Poll clipboard
                _ = tokio::time::sleep(poll_interval) => {
                    // Run clipboard read in a blocking thread (arboard is not async)
                    let change = task::spawn_blocking({
                        let mut mon = ClipboardMonitor {
                            last_hash: monitor.last_hash,
                            last_remote_write: monitor.last_remote_write,
                            remote_hash: monitor.remote_hash,
                        };
                        move || mon.poll()
                    })
                    .await
                    .ok()
                    .flatten();

                    if let Some(change) = change {
                        // Update monitor state
                        monitor.last_hash = Some(change.hash);
                        let _ = change_tx.send(change).await;
                    }
                }
            }
        }
    });

    (change_rx, suppress_tx)
}
