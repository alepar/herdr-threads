//! One mod watch connection's lifetime (spec D2, D7).
//!
//! The first client frame is a [`WatchWireRequest`]; this module answers it
//! with one [`WatchReply`], then (when accepted) streams [`WatchFrame`]s the
//! registry pushes until a `Close`, a client EOF or daemon shutdown. The
//! registration itself is decided by `LocalService::watch_register` against
//! A2; the transport only checks the elected instance and boot, and admission
//! (a separate 64-slot semaphore, so watch connections never consume the 32
//! ordinary slots beyond the instant they are sniffed).
//!
//! The client never sends after its first frame: any byte, EOF or read error
//! ends the channel through `watch_unregister` (the registry keeps it in
//! reconnect grace). Every write has a five second bound; a write that times
//! out counts as a drop. Shutdown ends the task promptly so the drain budget
//! is not consumed.

use super::*;
use crate::{
    ports::{ModChannelId, ModChannelSink},
    protocol::watch::{
        WatchAccepted, WatchCloseReason, WatchFrame, WatchOutcome, WatchRefusal,
        WatchRefusalReason, WatchReply, WatchWireRequest,
    },
};
use tokio::sync::{OwnedSemaphorePermit, mpsc};

/// Bound of one frame write.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// The registry's end of the connection: frames go to the connection task
/// over an unbounded channel (pushes are rare and tiny); `false` once the
/// task is gone.
struct ChannelSink(mpsc::UnboundedSender<WatchFrame>);
impl ModChannelSink for ChannelSink {
    fn push(&self, frame: WatchFrame) -> bool {
        self.0.send(frame).is_ok() && !self.0.is_closed()
    }
}

fn reply(
    request: &WatchWireRequest,
    instance: &str,
    daemon_boot: &str,
    outcome: WatchOutcome,
) -> WatchReply {
    WatchReply {
        version: PROTOCOL_VERSION,
        request_id: request.request_id.clone(),
        instance: instance.to_owned(),
        daemon_boot: daemon_boot.to_owned(),
        outcome,
    }
}

fn refused(reason: WatchRefusalReason, detail: Option<String>) -> WatchOutcome {
    WatchOutcome::Refused(WatchRefusal { reason, detail })
}

async fn write<T: serde::Serialize>(stream: &mut UnixStream, value: &T) -> io::Result<()> {
    let encoded = encode_json(value)?;
    tokio::time::timeout(WRITE_TIMEOUT, write_frame(stream, &encoded))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "watch write expired"))?
}

/// Answers a watch request with a refusal and ends the connection.
pub(super) async fn refuse(
    mut stream: UnixStream,
    request: &WatchWireRequest,
    instance: &str,
    daemon_boot: &str,
    reason: WatchRefusalReason,
) -> io::Result<()> {
    write(
        &mut stream,
        &reply(request, instance, daemon_boot, refused(reason, None)),
    )
    .await
}

/// Serves one admitted watch connection. `_permit` is the watch slot, held
/// until the connection ends.
// Allowed: per-connection state handed over from the accept loop.
#[allow(clippy::too_many_arguments)]
pub(super) async fn serve(
    mut stream: UnixStream,
    request: WatchWireRequest,
    _permit: OwnedSemaphorePermit,
    instance: String,
    daemon_boot: String,
    handler: Arc<dyn LocalService>,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
) -> io::Result<()> {
    if request.expected_instance != instance {
        let detail = "request instance differs from owner".to_owned();
        let outcome = refused(WatchRefusalReason::SessionMismatch, Some(detail));
        return write(
            &mut stream,
            &reply(&request, &instance, &daemon_boot, outcome),
        )
        .await;
    }
    if let Some(expected) = request
        .expected_boot
        .as_deref()
        .filter(|expected| *expected != daemon_boot)
    {
        let detail =
            format!("request expected daemon boot {expected}; this daemon is boot {daemon_boot}");
        let outcome = refused(WatchRefusalReason::SessionMismatch, Some(detail));
        return write(
            &mut stream,
            &reply(&request, &instance, &daemon_boot, outcome),
        )
        .await;
    }
    if shutdown.is_cancelled() {
        let outcome = refused(WatchRefusalReason::Stopping, None);
        return write(
            &mut stream,
            &reply(&request, &instance, &daemon_boot, outcome),
        )
        .await;
    }

    let (frames_tx, mut frames) = mpsc::unbounded_channel();
    let sink: Arc<dyn ModChannelSink> = Arc::new(ChannelSink(frames_tx));
    let budget = CallBudget {
        deadline: MonoInstant(
            clock
                .monotonic_now()
                .0
                .saturating_add(ORDINARY_TIMEOUT.as_millis() as u64),
        ),
        cancellation: Cancellation::default(),
    };
    let registration = {
        let handler = Arc::clone(&handler);
        let watch = request.watch.clone();
        let budget = budget.clone();
        tokio::task::spawn_blocking(move || handler.watch_register(&watch, sink, &budget))
            .await
            .map_err(io::Error::other)?
    };
    let (channel, attention_version) = match registration {
        Ok(registered) => registered,
        Err(reason) => {
            let outcome = refused(reason, None);
            return write(
                &mut stream,
                &reply(&request, &instance, &daemon_boot, outcome),
            )
            .await;
        }
    };
    let accepted = WatchOutcome::Accepted(WatchAccepted { attention_version });
    if let Err(error) = write(
        &mut stream,
        &reply(&request, &instance, &daemon_boot, accepted),
    )
    .await
    {
        unregister(&handler, &clock, channel).await;
        return Err(error);
    }

    let (mut read_half, mut write_half) = stream.into_split();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                // The registry's own Close{stopping} may already be queued.
                let frame = match frames.try_recv() {
                    Ok(frame @ WatchFrame::Close { .. }) => frame,
                    _ => WatchFrame::Close { reason: WatchCloseReason::Stopping },
                };
                let _ = write_half_frame(&mut write_half, &frame).await;
                return Ok(());
            }
            frame = frames.recv() => {
                let Some(frame) = frame else {
                    unregister(&handler, &clock, channel).await;
                    return Ok(());
                };
                let is_close = matches!(frame, WatchFrame::Close { .. });
                if let Err(error) = write_half_frame(&mut write_half, &frame).await {
                    unregister(&handler, &clock, channel).await;
                    return Err(error);
                }
                if is_close {
                    return Ok(());
                }
            }
            // The client never sends after its first frame: a byte is a
            // protocol error, EOF or an error is a drop.
            _ = read_half.read_u8() => {
                unregister(&handler, &clock, channel).await;
                return Ok(());
            }
        }
    }
}

async fn write_half_frame(
    half: &mut tokio::net::unix::OwnedWriteHalf,
    frame: &WatchFrame,
) -> io::Result<()> {
    let encoded = encode_json(frame)?;
    tokio::time::timeout(WRITE_TIMEOUT, write_frame(half, &encoded))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "watch write expired"))?
}

async fn unregister(
    handler: &Arc<dyn LocalService>,
    clock: &Arc<dyn Clock>,
    channel: ModChannelId,
) {
    let handler = Arc::clone(handler);
    let now = clock.utc_now();
    // The registry call takes a std mutex: keep it off the async worker.
    let _ = tokio::task::spawn_blocking(move || handler.watch_unregister(channel, now)).await;
}

#[cfg(test)]
#[path = "../../../tests/daemon/watch_connection.rs"]
mod tests;
