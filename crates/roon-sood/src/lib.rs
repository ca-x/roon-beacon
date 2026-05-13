use std::collections::HashMap;
use std::net::SocketAddr;

pub mod discovery;
mod parse;
mod serialize;

pub use discovery::{DiscoveredCore, SoodDiscovery};
pub use parse::parse;
pub use serialize::{serialize_query, serialize_response};

#[derive(Debug, Clone)]
pub struct SoodMessage {
    pub from: SocketAddr,
    pub msg_type: SoodType,
    pub props: HashMap<String, Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoodType {
    Query,
    Response,
}

#[derive(Debug, thiserror::Error)]
pub enum SoodError {
    #[error("buffer too short: need at least 6 bytes, got {0}")]
    BufferTooShort(usize),
    #[error("invalid magic bytes")]
    InvalidMagic,
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("invalid message type: {0}")]
    InvalidType(u8),
    #[error("zero-length property name")]
    ZeroLengthName,
    #[error("truncated TLV: name length {name_len} exceeds remaining {remaining} bytes")]
    TruncatedName { name_len: usize, remaining: usize },
    #[error("truncated TLV: value length header exceeds remaining {remaining} bytes")]
    TruncatedValueHeader { remaining: usize },
    #[error("truncated TLV: value length {value_len} exceeds remaining {remaining} bytes")]
    TruncatedValue { value_len: usize, remaining: usize },
    #[error("invalid UTF-8 in property name")]
    InvalidNameUtf8(#[from] std::string::FromUtf8Error),
    #[error("invalid UTF-8 in property value")]
    InvalidValueUtf8,
    #[error("I/O error: {0}")]
    Io(String),
}

pub const SOOD_PORT: u16 = 9003;
pub const SOOD_MULTICAST_IP: &str = "239.255.90.90";
pub const ROON_CORE_SERVICE_ID: &str = "00720724-5143-4a9b-abac-0e50cba674bb";
