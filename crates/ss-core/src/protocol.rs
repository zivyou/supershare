use serde::{Deserialize, Serialize};

/// Magic bytes for frame identification: "SS" (SuperShare)
pub const FRAME_MAGIC: [u8; 2] = [0x53, 0x53];

/// Maximum frame payload size (16 MB)
pub const MAX_PAYLOAD_SIZE: u32 = 16 * 1024 * 1024;

/// Clipboard chunk size (64 KB)
pub const CLIPBOARD_CHUNK_SIZE: usize = 64 * 1024;

/// Maximum clipboard image size (10 MB compressed)
pub const MAX_CLIPBOARD_IMAGE_SIZE: usize = 10 * 1024 * 1024;

/// Boundary zone width in pixels
pub const BOUNDARY_ZONE_PX: u32 = 5;

/// Heartbeat interval in seconds
pub const HEARTBEAT_INTERVAL_SECS: u64 = 5;

/// Connection timeout in seconds
pub const HEARTBEAT_TIMEOUT_SECS: u64 = 15;

/// Clipboard suppression duration in milliseconds
pub const CLIPBOARD_SUPPRESSION_MS: u64 = 1000;

/// Clipboard polling interval in milliseconds
pub const CLIPBOARD_POLL_INTERVAL_MS: u64 = 200;

/// Message type identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageType {
    MouseMove = 0x01,
    MouseButton = 0x02,
    MouseScroll = 0x03,
    KeyPress = 0x04,
    ClipboardData = 0x05,
    ClipboardBegin = 0x06,
    ClipboardChunk = 0x07,
    ClipboardEnd = 0x08,
    Handshake = 0x09,
    Heartbeat = 0x0A,
    ScreenConfig = 0x0B,
    BoundaryEnter = 0x0C,
    BoundaryLeave = 0x0D,
    MouseDelta = 0x0E,
    PairRequest = 0x0F,
    PairSpake = 0x10,
    PairConfirm = 0x11,
    PairResult = 0x12,
    PairError = 0x13,
}

impl TryFrom<u8> for MessageType {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::MouseMove),
            0x02 => Ok(Self::MouseButton),
            0x03 => Ok(Self::MouseScroll),
            0x04 => Ok(Self::KeyPress),
            0x05 => Ok(Self::ClipboardData),
            0x06 => Ok(Self::ClipboardBegin),
            0x07 => Ok(Self::ClipboardChunk),
            0x08 => Ok(Self::ClipboardEnd),
            0x09 => Ok(Self::Handshake),
            0x0A => Ok(Self::Heartbeat),
            0x0B => Ok(Self::ScreenConfig),
            0x0C => Ok(Self::BoundaryEnter),
            0x0D => Ok(Self::BoundaryLeave),
            0x0E => Ok(Self::MouseDelta),
            0x0F => Ok(Self::PairRequest),
            0x10 => Ok(Self::PairSpake),
            0x11 => Ok(Self::PairConfirm),
            0x12 => Ok(Self::PairResult),
            0x13 => Ok(Self::PairError),
            _ => anyhow::bail!("Unknown message type: 0x{:02X}", value),
        }
    }
}

/// Mouse button identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Button {
    Left = 0x01,
    Right = 0x02,
    Middle = 0x03,
}

/// Clipboard content format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ClipboardFormat {
    Text = 0x01,
    Image = 0x02,
}

/// All protocol messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// Mouse movement: absolute position (x, y) on the global coordinate system
    MouseMove { x: f32, y: f32 },

    /// Mouse movement: relative delta (dx, dy) from evdev
    MouseDelta { dx: f32, dy: f32 },

    /// Mouse button press/release
    MouseButton { button: Button, pressed: bool },

    /// Mouse scroll wheel
    MouseScroll { dx: f32, dy: f32 },

    /// Keyboard key press/release
    KeyPress { keycode: u32, pressed: bool },

    /// Small clipboard data (text only, fits in one message)
    ClipboardData {
        format: ClipboardFormat,
        data: Vec<u8>,
    },

    /// Start of a large clipboard transfer (images)
    ClipboardBegin {
        format: ClipboardFormat,
        total_size: u32,
        width: u32,
        height: u32,
    },

    /// A chunk of clipboard data
    ClipboardChunk { seq: u32, data: Vec<u8> },

    /// End of clipboard transfer, with hash for verification
    ClipboardEnd { hash: [u8; 32] },

    /// Client handshake: protocol version + device name + screen dimensions
    Handshake {
        version: u8,
        name: String,
        screen_width: u32,
        screen_height: u32,
    },

    /// Heartbeat (keep-alive)
    Heartbeat,

    /// Server screen configuration
    ScreenConfig {
        width: u32,
        height: u32,
    },

    /// Mouse entering a remote screen
    BoundaryEnter {
        target_screen: u8,
        enter_x: f32,
        enter_y: f32,
    },

    /// Mouse leaving a remote screen
    BoundaryLeave {
        source_screen: u8,
    },

    /// Pairing: client initiates pairing, announcing its desired device name.
    /// Sent in the clear on the pairing channel before the PAKE exchange.
    PairRequest {
        version: u8,
        name: String,
    },

    /// Pairing: a SPAKE2 protocol message (opaque bytes). Exchanged in both
    /// directions to establish a shared session key from the PIN.
    PairSpake {
        msg: Vec<u8>,
    },

    /// Pairing: AEAD-encrypted provisioning payload sent by the client once the
    /// session key is derived. Carries the client's public key + device name.
    PairConfirm {
        nonce: Vec<u8>,
        ciphertext: Vec<u8>,
    },

    /// Pairing: AEAD-encrypted result sent by the server. Carries the signed
    /// client certificate and the CA certificate (PEM) on success.
    PairResult {
        nonce: Vec<u8>,
        ciphertext: Vec<u8>,
    },

    /// Pairing: failure notification with a human-readable reason.
    PairError {
        reason: String,
    },
}

impl Message {
    /// Get the message type identifier
    pub fn msg_type(&self) -> MessageType {
        match self {
            Message::MouseMove { .. } => MessageType::MouseMove,
            Message::MouseDelta { .. } => MessageType::MouseDelta,
            Message::MouseButton { .. } => MessageType::MouseButton,
            Message::MouseScroll { .. } => MessageType::MouseScroll,
            Message::KeyPress { .. } => MessageType::KeyPress,
            Message::ClipboardData { .. } => MessageType::ClipboardData,
            Message::ClipboardBegin { .. } => MessageType::ClipboardBegin,
            Message::ClipboardChunk { .. } => MessageType::ClipboardChunk,
            Message::ClipboardEnd { .. } => MessageType::ClipboardEnd,
            Message::Handshake { .. } => MessageType::Handshake,
            Message::Heartbeat => MessageType::Heartbeat,
            Message::ScreenConfig { .. } => MessageType::ScreenConfig,
            Message::BoundaryEnter { .. } => MessageType::BoundaryEnter,
            Message::BoundaryLeave { .. } => MessageType::BoundaryLeave,
            Message::PairRequest { .. } => MessageType::PairRequest,
            Message::PairSpake { .. } => MessageType::PairSpake,
            Message::PairConfirm { .. } => MessageType::PairConfirm,
            Message::PairResult { .. } => MessageType::PairResult,
            Message::PairError { .. } => MessageType::PairError,
        }
    }

    /// Serialize the message payload (excluding frame header)
    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        bincode::serialize(self).map_err(|e| anyhow::anyhow!("bincode serialize error: {e}"))
    }

    /// Deserialize a message from payload bytes given the message type
    pub fn decode(msg_type: MessageType, data: &[u8]) -> anyhow::Result<Self> {
        bincode::deserialize(data).map_err(|e| anyhow::anyhow!("bincode deserialize error for {:?}: {e}", msg_type))
    }
}

/// A complete wire frame: header + payload
pub struct Frame {
    pub msg_type: MessageType,
    pub payload: Vec<u8>,
}

impl Frame {
    /// Encode frame to bytes: [Magic: 2][Type: 1][Length: 4 LE][Payload]
    pub fn to_bytes(&self) -> Vec<u8> {
        let len = self.payload.len() as u32;
        let mut buf = Vec::with_capacity(2 + 1 + 4 + self.payload.len());
        buf.extend_from_slice(&FRAME_MAGIC);
        buf.push(self.msg_type as u8);
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Header size: 2 (magic) + 1 (type) + 4 (length) = 7 bytes
    pub const HEADER_SIZE: usize = 7;
}

/// Clipboard content for change detection
#[derive(Debug, Clone)]
pub enum ClipboardContent {
    Text(String),
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

impl ClipboardContent {
    /// Compute blake3 hash of the content
    pub fn hash(&self) -> [u8; 32] {
        let data = match self {
            ClipboardContent::Text(s) => s.as_bytes(),
            ClipboardContent::Image { rgba, .. } => rgba.as_slice(),
        };
        let h = blake3::hash(data);
        *h.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip a message through encode/decode (the payload portion of a frame).
    fn roundtrip(msg: Message) {
        let mt = msg.msg_type();
        let bytes = msg.encode().expect("encode");
        let decoded = Message::decode(mt, &bytes).expect("decode");
        assert_eq!(decoded.msg_type(), mt);
    }

    #[test]
    fn pairing_variants_roundtrip() {
        roundtrip(Message::PairRequest {
            version: 1,
            name: "laptop".to_string(),
        });
        roundtrip(Message::PairSpake {
            msg: vec![1, 2, 3, 4, 5],
        });
        roundtrip(Message::PairConfirm {
            nonce: vec![9; 12],
            ciphertext: vec![7; 64],
        });
        roundtrip(Message::PairResult {
            nonce: vec![3; 12],
            ciphertext: vec![8; 256],
        });
        roundtrip(Message::PairError {
            reason: "wrong PIN".to_string(),
        });
    }

    #[test]
    fn pairing_message_types_map_back() {
        for mt in [
            MessageType::PairRequest,
            MessageType::PairSpake,
            MessageType::PairConfirm,
            MessageType::PairResult,
            MessageType::PairError,
        ] {
            let byte = mt as u8;
            assert_eq!(MessageType::try_from(byte).unwrap(), mt);
        }
    }
}
