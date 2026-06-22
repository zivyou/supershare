use ss_core::protocol::{Frame, Message, MessageType, FRAME_MAGIC, MAX_PAYLOAD_SIZE};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Read a single framed message from an async reader.
/// Returns the parsed Message, or None on clean EOF.
pub async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> anyhow::Result<Option<Message>> {
    // Read header: 2 (magic) + 1 (type) + 4 (length) = 7 bytes
    let mut header = [0u8; Frame::HEADER_SIZE];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }

    // Verify magic bytes
    if header[0..2] != FRAME_MAGIC {
        anyhow::bail!(
            "Invalid frame magic: expected {:02X?}, got {:02X?}",
            FRAME_MAGIC,
            &header[0..2]
        );
    }

    // Parse type
    let msg_type = MessageType::try_from(header[2])?;

    // Parse length (little-endian u32)
    let length = u32::from_le_bytes([header[3], header[4], header[5], header[6]]);

    // Sanity check
    if length > MAX_PAYLOAD_SIZE {
        anyhow::bail!(
            "Frame payload too large: {length} bytes (max {MAX_PAYLOAD_SIZE})"
        );
    }

    // Read payload
    let mut payload = vec![0u8; length as usize];
    reader.read_exact(&mut payload).await?;

    // Decode message
    let message = Message::decode(msg_type, &payload)?;
    Ok(Some(message))
}

/// Write a single message as a framed packet to an async writer.
pub async fn write_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    message: &Message,
) -> anyhow::Result<()> {
    let payload = message.encode()?;
    let frame = Frame {
        msg_type: message.msg_type(),
        payload,
    };
    let bytes = frame.to_bytes();
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}
