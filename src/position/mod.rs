//! Build-specific position protocol decoding, independent of player tracking and UI.
//!
//! Feed decrypted **body-only** commands to [`Decoder::receive`]. Events retain
//! wire order and rule boundaries, including nested batched commands. Consumers
//! select their own player and apply a packet's events atomically. Candidate
//! payload errors are retained until selection so incomplete foreign entities
//! cannot invalidate the local player's reading.

mod builtin;
mod decoder;
mod profile_json;
mod schema;
pub mod wire;

pub use builtin::captured_7_0;
pub use decoder::*;
pub use schema::*;
pub use wire::Position;
