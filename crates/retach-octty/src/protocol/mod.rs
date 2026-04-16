//! Length-prefixed binary protocol for client-server communication over Unix sockets.
//! Messages are serialized with bincode and framed with a 4-byte big-endian length prefix.

pub mod codec;
pub mod messages;

pub use codec::{encode, read_one_message, FrameReader};
pub use messages::{ClientMsg, ConnectMode, ServerMsg, SessionInfo, SpawnRequest};

#[cfg(test)]
mod tests_history_protocol;
