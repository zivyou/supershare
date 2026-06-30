use ss_core::protocol::{
    ClipboardContent, ClipboardFormat, Message, CLIPBOARD_CHUNK_SIZE, MAX_CLIPBOARD_IMAGE_SIZE,
};

/// Compress image data with zstd
pub fn compress_image(rgba: &[u8]) -> anyhow::Result<Vec<u8>> {
    zstd::encode_all(rgba, 3).map_err(|e| anyhow::anyhow!("zstd compress error: {e}"))
}

/// Decompress image data with zstd
pub fn decompress_image(compressed: &[u8]) -> anyhow::Result<Vec<u8>> {
    zstd::decode_all(compressed).map_err(|e| anyhow::anyhow!("zstd decompress error: {e}"))
}

/// Prepare clipboard content for network transfer.
/// Returns a sequence of messages to send.
pub fn prepare_transfer(content: &ClipboardContent) -> anyhow::Result<Vec<Message>> {
    match content {
        ClipboardContent::Text(text) => {
            // Text fits in a single ClipboardData message
            Ok(vec![Message::ClipboardData {
                format: ClipboardFormat::Text,
                data: text.as_bytes().to_vec(),
            }])
        }
        ClipboardContent::Image {
            width,
            height,
            rgba,
        } => {
            let compressed = compress_image(rgba)?;

            // Check size limit
            if compressed.len() > MAX_CLIPBOARD_IMAGE_SIZE {
                anyhow::bail!(
                    "Compressed image too large: {} bytes (max {})",
                    compressed.len(),
                    MAX_CLIPBOARD_IMAGE_SIZE
                );
            }

            let hash = blake3::hash(rgba);
            let mut messages = Vec::new();

            // Begin message
            messages.push(Message::ClipboardBegin {
                format: ClipboardFormat::Image,
                total_size: compressed.len() as u32,
                width: *width,
                height: *height,
            });

            // Chunk messages
            let mut seq = 0u32;
            for chunk in compressed.chunks(CLIPBOARD_CHUNK_SIZE) {
                messages.push(Message::ClipboardChunk {
                    seq,
                    data: chunk.to_vec(),
                });
                seq += 1;
            }

            // End message with hash
            messages.push(Message::ClipboardEnd {
                hash: *hash.as_bytes(),
            });

            Ok(messages)
        }
    }
}

/// State for reassembling chunked clipboard transfers
pub struct ClipboardReassembler {
    /// Expected total compressed size
    total_size: u32,
    /// Expected image width
    width: u32,
    /// Expected image height
    height: u32,
    /// Collected chunks (seq -> data)
    chunks: Vec<(u32, Vec<u8>)>,
    /// Total bytes received so far
    received: u32,
}

impl ClipboardReassembler {
    pub fn new(total_size: u32, width: u32, height: u32) -> Self {
        Self {
            total_size,
            width,
            height,
            chunks: Vec::new(),
            received: 0,
        }
    }

    /// Add a chunk. Returns true if all chunks have been received.
    pub fn add_chunk(&mut self, seq: u32, data: Vec<u8>) -> bool {
        self.received += data.len() as u32;
        self.chunks.push((seq, data));
        self.received >= self.total_size
    }

    /// Reassemble and decompress the image. Returns (width, height, rgba_pixels).
    pub fn finish(self, expected_hash: &[u8; 32]) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        // Sort chunks by sequence number
        let mut sorted = self.chunks;
        sorted.sort_by_key(|(seq, _)| *seq);

        // Concatenate
        let compressed: Vec<u8> = sorted.into_iter().flat_map(|(_, data)| data).collect();

        // Decompress
        let rgba = decompress_image(&compressed)?;

        // Verify hash
        let actual_hash = blake3::hash(&rgba);
        if actual_hash.as_bytes() != expected_hash {
            anyhow::bail!("Clipboard hash mismatch: data may be corrupted");
        }

        Ok((self.width, self.height, rgba))
    }
}

/// Handle incoming clipboard messages.
/// Returns Some(ClipboardContent) when a transfer is complete.
pub fn handle_clipboard_message(
    msg: &Message,
    reassembler: &mut Option<ClipboardReassembler>,
) -> Option<ClipboardContent> {
    match msg {
        Message::ClipboardData { format, data } => {
            match format {
                ClipboardFormat::Text => {
                    let text = String::from_utf8_lossy(data).to_string();
                    Some(ClipboardContent::Text(text))
                }
                _ => {
                    tracing::warn!("Unsupported clipboard format in ClipboardData");
                    None
                }
            }
        }
        Message::ClipboardBegin {
            format,
            total_size,
            width,
            height,
        } => {
            match format {
                ClipboardFormat::Image => {
                    *reassembler = Some(ClipboardReassembler::new(*total_size, *width, *height));
                }
                _ => {
                    tracing::warn!("Unsupported clipboard format in ClipboardBegin");
                }
            }
            None
        }
        Message::ClipboardChunk { seq, data } => {
            if let Some(ref mut asm) = reassembler {
                asm.add_chunk(*seq, data.clone());
            }
            None
        }
        Message::ClipboardEnd { hash } => {
            if let Some(asm) = reassembler.take() {
                match asm.finish(hash) {
                    Ok((width, height, rgba)) => {
                        Some(ClipboardContent::Image { width, height, rgba })
                    }
                    Err(e) => {
                        tracing::error!("Failed to reassemble clipboard image: {e}");
                        None
                    }
                }
            } else {
                tracing::warn!("Received ClipboardEnd without matching ClipboardBegin");
                None
            }
        }
        _ => None,
    }
}
