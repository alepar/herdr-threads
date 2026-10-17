//! Complete fixture snapshots using committed progress, never a guessed turn count.

use crate::{
    ports::{HostObservation, SnapshotStageProgress},
    protocol::results::ApiError,
};

/// Stage every target before a fixture seals or publishes its snapshot.
/// The caller retains its original budget and stage; a failed turn propagates
/// immediately. A successful turn may commit only a prefix of its admission.
pub fn stage_all(
    targets: &[HostObservation],
    mut stage: impl FnMut(u64, &[HostObservation]) -> Result<SnapshotStageProgress, ApiError>,
) -> Result<(), ApiError> {
    let mut offset = 0usize;
    while offset < targets.len() {
        let batch = &targets[offset..targets.len().min(offset + 16)];
        let progress = stage(offset as u64, batch)?;
        let visited = usize::from(progress.visited);
        assert!(
            visited > 0 && visited <= batch.len(),
            "snapshot staging must advance"
        );
        offset += visited;
        assert_eq!(
            progress.stage.staged_targets, offset as u64,
            "snapshot staging must advance exactly"
        );
    }
    Ok(())
}
