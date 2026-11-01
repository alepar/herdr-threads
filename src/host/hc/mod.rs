//! The herdr-core (`hc`) adapter's protocol layer.
//!
//! Responsibility
//! - Owns: the JSON-RPC 2.0 line client ([`rpc::HcClient`]) and the tolerant
//!   wire types ([`wire`]) for the herdr-core socket.
//! - Hides: framing, the one-connection-per-call rule, the peer witness
//!   capture and the reply decoding.
//! - Must not: call `terminal.list`, `doc.set` or pass `caller` (it mutates
//!   hc's parent bookkeeping); decide host policy. The `HcHost` adapter that
//!   builds on this layer lands in its own module.
//! - Role: callers never speak hc directly; new hc protocol use goes here.

pub mod rpc;
pub mod wire;

pub use rpc::{HcClient, HcError, HcReply, Snapshot};
pub use wire::{
    AppDoc, CoreInfo, HcEvent, HcEventKind, HcForeground, HcStatus, HcTerminal, HelloReply,
};
