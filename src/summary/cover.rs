//! Index-aligned rollup addressing and the displayed-cover algorithm
//! (spec §3). Pure: callers pass the stored block metadata.
use crate::protocol::summary::{FAN_IN, SeqRange, SummarySettings};
use std::collections::BTreeMap;

const FAN: u64 = FAN_IN as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockKey {
    pub level: u32,
    pub index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMeta {
    pub key: BlockKey,
    pub chunking_version: String,
    pub range: SeqRange,
    pub narrative_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverPlan {
    /// Ordered by range; filled only when the plan is Ready, else empty.
    pub cover: Vec<BlockKey>,
    /// Ascending chunk index.
    pub missing_level0: Vec<u64>,
    /// Eligible missing parents (all eight children stored), ascending (level, index).
    pub needed_rollups: Vec<BlockKey>,
    /// Narrative bytes of the working cover, missing parents counted at the nominal size.
    pub narrative_bytes: u64,
    /// Narratives still exceed `display_bytes` with no run of eight left.
    pub over_budget: bool,
}
impl CoverPlan {
    /// No level-0 block missing and no rollup needed: `cover` is displayable.
    pub fn ready(&self) -> bool {
        self.missing_level0.is_empty() && self.needed_rollups.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverError {
    VersionMismatch { expected: String, found: String },
    Inconsistent(&'static str),
}

/// Level n+1 block k covers children 8k..8k+7 of level n.
pub fn parent(key: BlockKey) -> BlockKey {
    BlockKey {
        level: key.level + 1,
        index: key.index / FAN,
    }
}

/// The eight children of a level >= 1 block.
pub fn children(key: BlockKey) -> [BlockKey; 8] {
    debug_assert!(key.level >= 1, "level 0 blocks have no children");
    std::array::from_fn(|i| BlockKey {
        level: key.level - 1,
        index: key.index * FAN + i as u64,
    })
}

fn index_stored(
    chunking_version: &str,
    stored: &[StoredMeta],
) -> Result<BTreeMap<BlockKey, StoredMeta>, CoverError> {
    let mut map = BTreeMap::new();
    for meta in stored {
        if meta.chunking_version != chunking_version {
            return Err(CoverError::VersionMismatch {
                expected: chunking_version.to_string(),
                found: meta.chunking_version.clone(),
            });
        }
        if map.insert(meta.key, meta.clone()).is_some() {
            return Err(CoverError::Inconsistent("duplicate stored block"));
        }
    }
    Ok(map)
}

/// Rollup eligibility: `Some(children)` when all eight children of the
/// rollup block `key` (level 1 or higher) are stored, `None` otherwise.
/// Rejects any stored block of another `chunking_version`.
pub fn rollup_inputs(
    chunking_version: &str,
    key: BlockKey,
    stored: &[StoredMeta],
) -> Result<Option<[BlockKey; 8]>, CoverError> {
    if key.level == 0 {
        return Err(CoverError::Inconsistent("level 0 has no rollup inputs"));
    }
    let map = index_stored(chunking_version, stored)?;
    let kids = children(key);
    Ok(kids.iter().all(|k| map.contains_key(k)).then_some(kids))
}

#[derive(Clone)]
struct Item {
    key: BlockKey,
    range: SeqRange,
    bytes: u64,
    stored: bool,
}

/// The displayed cover: start at level 0, replace the oldest aligned run of
/// eight same-level blocks by its parent while narratives exceed
/// `display_bytes`. A missing parent whose eight children are all stored is a
/// needed rollup, counted at `narrative_bytes` so planning proceeds; a run
/// whose parent cannot yet be built stops the loop.
pub fn plan_cover(
    chunking_version: &str,
    level0_count: u64,
    stored: &[StoredMeta],
    settings: &SummarySettings,
) -> Result<CoverPlan, CoverError> {
    let map = index_stored(chunking_version, stored)?;
    let missing_level0: Vec<u64> = (0..level0_count)
        .filter(|index| {
            !map.contains_key(&BlockKey {
                level: 0,
                index: *index,
            })
        })
        .collect();
    let mut plan = CoverPlan {
        cover: Vec::new(),
        missing_level0,
        needed_rollups: Vec::new(),
        narrative_bytes: 0,
        over_budget: false,
    };
    if !plan.missing_level0.is_empty() {
        return Ok(plan);
    }
    let nominal = u64::from(settings.narrative_bytes);
    let budget = u64::from(settings.display_bytes);
    let mut cover: Vec<Item> = (0..level0_count)
        .map(|index| {
            let meta = &map[&BlockKey { level: 0, index }];
            Item {
                key: meta.key,
                range: meta.range,
                bytes: meta.narrative_bytes,
                stored: true,
            }
        })
        .collect();
    let mut total: u64 = cover.iter().map(|item| item.bytes).sum();
    while total > budget {
        let Some(at) = oldest_run(&cover) else {
            plan.over_budget = true;
            break;
        };
        let run = &cover[at..at + FAN as usize];
        let key = parent(run[0].key);
        let replacement = match map.get(&key) {
            Some(meta) => Item {
                key,
                range: meta.range,
                bytes: meta.narrative_bytes,
                stored: true,
            },
            None if run.iter().all(|item| item.stored) => Item {
                key,
                range: SeqRange {
                    first_seq: run[0].range.first_seq,
                    last_seq: run[run.len() - 1].range.last_seq,
                },
                bytes: nominal,
                stored: false,
            },
            None => break,
        };
        total = total - run.iter().map(|item| item.bytes).sum::<u64>() + replacement.bytes;
        cover.splice(at..at + FAN as usize, [replacement]);
    }
    plan.narrative_bytes = total;
    plan.needed_rollups = cover
        .iter()
        .filter(|item| !item.stored)
        .map(|item| item.key)
        .collect();
    plan.needed_rollups.sort();
    if plan.ready() {
        plan.cover = cover.iter().map(|item| item.key).collect();
    }
    Ok(plan)
}

/// Position of the oldest aligned run of eight same-level blocks in the cover.
fn oldest_run(cover: &[Item]) -> Option<usize> {
    let n = FAN as usize;
    (0..cover.len().saturating_sub(n - 1)).find(|&at| {
        let first = cover[at].key;
        first.index.is_multiple_of(FAN)
            && cover[at..at + n].iter().enumerate().all(|(i, item)| {
                item.key.level == first.level && item.key.index == first.index + i as u64
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;

    fn k(level: u32, index: u64) -> BlockKey {
        BlockKey { level, index }
    }

    fn meta(version: &str, level: u32, index: u64, kib: u64) -> StoredMeta {
        // Ranges are synthetic: each L0 block covers 10 sequences.
        let span = 10 * FAN.pow(level);
        StoredMeta {
            key: k(level, index),
            chunking_version: version.to_string(),
            range: SeqRange {
                first_seq: index * span + 1,
                last_seq: (index + 1) * span,
            },
            narrative_bytes: kib * KIB,
        }
    }

    fn l0(version: &str, count: u64, kib: u64) -> Vec<StoredMeta> {
        (0..count).map(|i| meta(version, 0, i, kib)).collect()
    }

    const V: &str = "cv1-test";

    #[test]
    fn addressing_is_index_aligned() {
        let kids = children(k(1, 2));
        assert_eq!(kids[0], k(0, 16));
        assert_eq!(kids[7], k(0, 23));
        assert!(kids.iter().all(|c| c.level == 0 && parent(*c) == k(1, 2)));
        assert_eq!(parent(k(0, 7)), k(1, 0));
        assert_eq!(parent(k(0, 8)), k(1, 1));
        assert_eq!(parent(k(1, 9)), k(2, 1));
    }

    #[test]
    fn missing_level0_blocks_are_reported_first() {
        let stored = vec![meta(V, 0, 1, 1)];
        let plan = plan_cover(V, 3, &stored, &SummarySettings::default()).unwrap();
        assert_eq!(plan.missing_level0, vec![0, 2]);
        assert!(plan.cover.is_empty() && plan.needed_rollups.is_empty());
        assert!(!plan.ready());
    }

    #[test]
    fn cover_under_budget_stays_level0() {
        let plan = plan_cover(V, 5, &l0(V, 5, 1), &SummarySettings::default()).unwrap();
        assert!(plan.ready() && !plan.over_budget);
        assert_eq!(plan.cover, (0..5).map(|i| k(0, i)).collect::<Vec<_>>());
        assert_eq!(plan.narrative_bytes, 5 * KIB);
        assert!(plan.needed_rollups.is_empty());
    }

    #[test]
    fn oldest_run_is_replaced_first_and_missing_parent_reported() {
        let mut stored = l0(V, 16, 4);
        let plan = plan_cover(V, 16, &stored, &SummarySettings::default()).unwrap();
        assert_eq!(plan.needed_rollups, vec![k(1, 0)]);
        assert!(!plan.ready() && plan.cover.is_empty());
        // 8 x 4 KiB + nominal 3 KiB.
        assert_eq!(plan.narrative_bytes, 35 * KIB);
        stored.push(meta(V, 1, 0, 3));
        let plan = plan_cover(V, 16, &stored, &SummarySettings::default()).unwrap();
        assert!(plan.ready() && !plan.over_budget);
        let mut want = vec![k(1, 0)];
        want.extend((8..16).map(|i| k(0, i)));
        assert_eq!(plan.cover, want);
    }

    #[test]
    fn two_rollup_levels_keep_recent_blocks_at_level0() {
        let settings = SummarySettings::default();
        let mut stored = l0(V, 80, 3);
        stored.extend((0..10).map(|i| meta(V, 1, i, 3)));
        stored.push(meta(V, 2, 0, 3));
        let plan = plan_cover(V, 80, &stored, &settings).unwrap();
        assert!(plan.ready() && !plan.over_budget);
        assert!(plan.narrative_bytes <= u64::from(settings.display_bytes));
        assert_eq!(
            plan.cover[0],
            k(2, 0),
            "oldest history is the level-2 block"
        );
        let newest = *plan.cover.last().unwrap();
        assert_eq!(newest, k(0, 79));
        let tail_l0 = plan.cover.iter().rev().take_while(|b| b.level == 0).count();
        assert_eq!(tail_l0, 8, "most recent blocks stay at level 0");
        // Ordered by range.
        let firsts: Vec<_> = plan
            .cover
            .iter()
            .map(|b| stored.iter().find(|m| m.key == *b).unwrap().range.first_seq)
            .collect();
        assert!(firsts.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn level_two_parent_needs_all_level_one_children_stored() {
        // 64 L0 x 4 KiB, L1 #0..#7 missing: the first run (L0 0..7) needs L1#0.
        let stored = l0(V, 64, 4);
        let plan = plan_cover(V, 64, &stored, &SummarySettings::default()).unwrap();
        // Each oldest run is replaced by a nominal L1 until the budget fits
        // (256 -> 40 needs 8 replacements: 8 x 3 + 7*... all L1 needed).
        assert!(plan.needed_rollups.iter().all(|key| key.level == 1));
        assert!(!plan.needed_rollups.is_empty());
        // A level-2 parent over missing L1 children is not eligible.
        assert_eq!(rollup_inputs(V, k(2, 0), &stored).unwrap(), None);
        assert!(rollup_inputs(V, k(1, 0), &stored).unwrap().is_some());
        assert_eq!(rollup_inputs(V, k(1, 8), &stored).unwrap(), None);
    }

    #[test]
    fn no_run_left_sets_over_budget() {
        let plan = plan_cover(V, 7, &l0(V, 7, 8), &SummarySettings::default()).unwrap();
        assert!(plan.over_budget && plan.ready());
        assert_eq!(plan.cover, (0..7).map(|i| k(0, i)).collect::<Vec<_>>());
    }

    #[test]
    fn foreign_chunking_version_is_rejected() {
        let mut stored = l0(V, 3, 1);
        stored[1].chunking_version = "cv1-other".to_string();
        let err = plan_cover(V, 3, &stored, &SummarySettings::default()).unwrap_err();
        assert_eq!(
            err,
            CoverError::VersionMismatch {
                expected: V.to_string(),
                found: "cv1-other".to_string()
            }
        );
        let mut stored = l0(V, 8, 1);
        stored[3].chunking_version = "cv1-other".to_string();
        assert!(matches!(
            rollup_inputs(V, k(1, 0), &stored),
            Err(CoverError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn determinism() {
        let mut stored = l0(V, 16, 4);
        stored.push(meta(V, 1, 0, 3));
        let a = plan_cover(V, 16, &stored, &SummarySettings::default()).unwrap();
        stored.reverse();
        let b = plan_cover(V, 16, &stored, &SummarySettings::default()).unwrap();
        assert_eq!(a, b);
    }
}
