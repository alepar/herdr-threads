//! Hand-off of a sniffed inbound peer connection to the sync coordinator.
//!
//! Responsibility (peer_connection)
//! - Owns: the admission outcome for one connection whose first frame decoded
//!   as a peer hello: hand the stream, that frame and the peer permit to the
//!   [`PeerIntake`], or answer `close{busy}` and drop the stream.
//! - Hides: the intake's channel and the coordinator behind it. A refused offer
//!   (full or stopped), a missing intake (sync not running) and a missing
//!   permit (16 links already) are three reasons for one outcome.
//! - Must not: run the handshake, verify the hello, or keep the stream after a
//!   successful offer; the transport task returns at once, so the 250 ms
//!   shutdown drain never waits on a link.
//! - Role: the transport's only peer-specific step, called by
//!   `serve_connection` after the hello sniff. Link behaviour belongs to
//!   `crate::sync`.

use crate::sync::{
    coordinator::{Busy, PeerIntake, PeerIo, PeerStream},
    frame::{CloseReason, MAX_PEER_CONNECTIONS, PeerFrame, write_peer_frame},
};
use std::{io, time::Duration};
use tokio::{io::AsyncWriteExt, sync::OwnedSemaphorePermit};

/// The bound on writing a `close{busy}` to a peer that will not be served.
const BUSY_CLOSE_BOUND: Duration = Duration::from_secs(1);

/// Offer `stream` (whose hello was already read as `frame`) to `intake`.
/// `permit` is `None` when all peer slots are taken. Returns once the stream is
/// handed over or the refusal is written.
pub(super) async fn hand_off<S: PeerIo + 'static>(
    mut stream: S,
    frame: Vec<u8>,
    permit: Option<OwnedSemaphorePermit>,
    intake: Option<&PeerIntake>,
) -> io::Result<()> {
    let Some(intake) = intake else {
        return refuse(&mut stream, "sync not running").await;
    };
    let Some(permit) = permit else {
        let detail = format!("peer connections full ({MAX_PEER_CONNECTIONS})");
        return refuse(&mut stream, &detail).await;
    };
    let offer = PeerStream {
        stream: Box::new(stream),
        first_frame: frame,
        permit: Box::new(permit),
    };
    match intake.offer(offer) {
        Ok(()) => Ok(()),
        Err(Busy(PeerStream { mut stream, .. })) => refuse(&mut stream, "sync busy").await,
    }
}

async fn refuse<S: PeerIo>(stream: &mut S, detail: &str) -> io::Result<()> {
    let close = PeerFrame::Close {
        reason: CloseReason::Busy,
        detail: Some(detail.to_owned()),
    };
    let write = async {
        write_peer_frame(stream, &close).await?;
        stream.flush().await
    };
    tokio::time::timeout(BUSY_CLOSE_BOUND, write)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "busy close expired"))?
}
