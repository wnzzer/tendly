//! Shared process lifecycle, commands, approvals and events for Rust clients.
//!
//! A client ([`NativeClient`]) owns a [`Core`] and adds only its protocol: how
//! it starts, what a message from it means, what follows each of its replies
//! and how one of its requests is answered. The core runs as one task (see
//! [`NativeAdapter`]): every input -- a command from the server, a message or a
//! reply from the client, a request timing out, an approval expiring, the
//! client's output ending -- is handled to the end before the next. A request
//! records what follows its answer (`Then`) instead of awaiting it, so no state
//! is held across a wait.
//!
//! `values.rs` validates and bounds strings and values, `launch.rs`
//! the launch-argument walk and the approval scope, `base.rs` the core's state
//! and the events it emits (`ChatBase`), `connection.rs` the process and the
//! requests waiting on it (`NativeProcess`), `client.rs` the [`NativeClient`]
//! trait, `run.rs` the line reader and the conversation's loop, and
//! `adapter.rs` the [`NativeAdapter`] the server holds.
use std::time::Duration;

mod adapter;
mod base;
mod client;
mod connection;
mod launch;
mod run;
#[cfg(test)]
mod tests;
mod values;

pub use adapter::*;
pub use base::*;
pub use client::*;
pub use launch::*;
use run::*;
pub use values::*;

/// One line from the client. Codex reports a generated image inline as
/// base64, several megabytes for one picture, so this is sized for images.
pub const MAX_NATIVE_LINE: usize = 64 * 1024 * 1024;
/// What may be queued for the client to read (bounded queue).
pub const MAX_NATIVE_INPUT: usize = 4 * 1024 * 1024;
pub const MAX_TEXT: usize = 64 * 1024;
pub const MAX_EVENT_LINE: usize = 192 * 1024;
/// Events sent and not yet taken, past which a conversation stops (bounded queue).
const MAX_UNSENT: usize = 4 * 1024 * 1024;
/// Everything the client writes over the session's life. Each line is read
/// and let go, so this only stops a runaway stream, not an image-heavy session.
const MAX_NATIVE_TOTAL: usize = 2 * 1024 * 1024 * 1024;
const LINE_TIMEOUT: Duration = Duration::from_secs(10);
pub const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_DIAGNOSTIC: usize = 16 * 1024 * 1024;
/// What a request still waiting is failed with when the connection closes.
pub const CANCELLED: &str = "Native client closed.";
