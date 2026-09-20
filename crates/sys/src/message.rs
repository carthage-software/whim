use crate::socket::Address;

pub use crate::platform::message::{enable_metadata, receive, send};

pub struct Message {
    pub bytes: Vec<u8>,
    pub peer: Address,
    pub local: Address,
    pub congestion: u8,
    pub interface: u32,
    pub truncated: bool,
}
