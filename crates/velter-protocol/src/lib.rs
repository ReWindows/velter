//! Versioned, transport-independent framing shared by local agents, the CLI,
//! GUI, and collaboration services.

use std::fmt;

pub const MAGIC: [u8; 4] = *b"VLTR";
pub const HEADER_LEN: usize = 24;
pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum MessageKind {
    Request = 1,
    Response = 2,
    Event = 3,
    Error = 4,
}

impl TryFrom<u16> for MessageKind {
    type Error = ProtocolError;
    fn try_from(value: u16) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Request),
            2 => Ok(Self::Response),
            3 => Ok(Self::Event),
            4 => Ok(Self::Error),
            _ => Err(ProtocolError::UnknownKind(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub major: u16,
    pub minor: u16,
    pub kind: MessageKind,
    pub flags: u16,
    pub request_id: u64,
    pub payload_len: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub header: Header,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    Truncated,
    BadMagic,
    UnknownKind(u16),
    PayloadTooLarge(usize),
    LengthMismatch { declared: usize, actual: usize },
    FieldTooLarge(usize),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProtocolError {}

impl Frame {
    pub fn new(
        major: u16,
        minor: u16,
        kind: MessageKind,
        request_id: u64,
        payload: Vec<u8>,
    ) -> Result<Self, ProtocolError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(ProtocolError::PayloadTooLarge(payload.len()));
        }
        Ok(Self {
            header: Header {
                major,
                minor,
                kind,
                flags: 0,
                request_id,
                payload_len: payload.len() as u32,
            },
            payload,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(ProtocolError::PayloadTooLarge(self.payload.len()));
        }
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.payload.len());
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&self.header.major.to_le_bytes());
        bytes.extend_from_slice(&self.header.minor.to_le_bytes());
        bytes.extend_from_slice(&(self.header.kind as u16).to_le_bytes());
        bytes.extend_from_slice(&self.header.flags.to_le_bytes());
        bytes.extend_from_slice(&self.header.request_id.to_le_bytes());
        bytes.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() < HEADER_LEN {
            return Err(ProtocolError::Truncated);
        }
        if bytes[..4] != MAGIC {
            return Err(ProtocolError::BadMagic);
        }
        let read16 = |offset| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        let read32 = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let read64 = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        let payload_len = read32(20) as usize;
        if payload_len > MAX_PAYLOAD {
            return Err(ProtocolError::PayloadTooLarge(payload_len));
        }
        let actual = bytes.len() - HEADER_LEN;
        if payload_len != actual {
            return Err(ProtocolError::LengthMismatch {
                declared: payload_len,
                actual,
            });
        }
        Ok(Self {
            header: Header {
                major: read16(4),
                minor: read16(6),
                kind: read16(8).try_into()?,
                flags: read16(10),
                request_id: read64(12),
                payload_len: payload_len as u32,
            },
            payload: bytes[HEADER_LEN..].to_vec(),
        })
    }
}

/// Canonical field stream: u16 tag, u32 byte length, then bytes. Unknown tags
/// can be skipped, allowing compatible protocol extension.
#[derive(Default)]
pub struct Fields {
    bytes: Vec<u8>,
}

impl Fields {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, tag: u16, value: &[u8]) -> Result<(), ProtocolError> {
        if value.len() > u32::MAX as usize {
            return Err(ProtocolError::FieldTooLarge(value.len()));
        }
        self.bytes.extend_from_slice(&tag.to_le_bytes());
        self.bytes
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(value);
        Ok(())
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn iter(&self) -> FieldIter<'_> {
        FieldIter {
            remaining: &self.bytes,
        }
    }
}

pub struct FieldIter<'a> {
    remaining: &'a [u8],
}
impl<'a> Iterator for FieldIter<'a> {
    type Item = Result<(u16, &'a [u8]), ProtocolError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        if self.remaining.len() < 6 {
            self.remaining = &[];
            return Some(Err(ProtocolError::Truncated));
        }
        let tag = u16::from_le_bytes(self.remaining[..2].try_into().unwrap());
        let length = u32::from_le_bytes(self.remaining[2..6].try_into().unwrap()) as usize;
        if self.remaining.len() < 6 + length {
            self.remaining = &[];
            return Some(Err(ProtocolError::Truncated));
        }
        let value = &self.remaining[6..6 + length];
        self.remaining = &self.remaining[6 + length..];
        Some(Ok((tag, value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_round_trip() {
        let frame = Frame::new(1, 0, MessageKind::Request, 42, b"attach".to_vec()).unwrap();
        assert_eq!(Frame::decode(&frame.encode().unwrap()).unwrap(), frame);
    }
    #[test]
    fn rejects_length_mismatch() {
        let mut bytes = Frame::new(1, 0, MessageKind::Event, 0, vec![1])
            .unwrap()
            .encode()
            .unwrap();
        bytes.push(2);
        assert!(matches!(
            Frame::decode(&bytes),
            Err(ProtocolError::LengthMismatch { .. })
        ));
    }
    #[test]
    fn iterates_extensible_fields() {
        let mut fields = Fields::new();
        fields.push(7, b"known").unwrap();
        fields.push(999, b"future").unwrap();
        let values: Vec<_> = fields.iter().map(Result::unwrap).collect();
        assert_eq!(values[1], (999, b"future".as_slice()));
    }
}
