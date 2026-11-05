//! Code generated from `proto/space_mmo.proto`, shared by the server and the bot.

// Generated enums hold messages inline, which is the design of prost's output.
#![allow(clippy::large_enum_variant)]

include!(concat!(env!("OUT_DIR"), "/space_mmo.v1.rs"));

/// Largest frame either side accepts.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
