//! Deterministic chunk boundaries (spec §2). Pure: the caller feeds dense,
//! ascending sequences with their rendered sizes; only sequences at or below
//! the published head are ever fed, so a chunk is full only when its closing
//! message is at or below the head.
use crate::protocol::summary::SeqRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSpan {
    pub index: u64,
    pub range: SeqRange,
    pub rendered_bytes: u64,
}

/// A chunk closes at the first message that brings it to `chunk_bytes`. A
/// message of `chunk_bytes` or more closes the current chunk before it and
/// stands alone.
#[derive(Debug, Clone)]
pub struct Chunker {
    chunk_bytes: u64,
    next_index: u64,
    next_seq: u64,
    first: Option<u64>,
    last: u64,
    acc: u64,
}

impl Chunker {
    /// Starts at sequence 1, chunk index 0.
    pub fn new(chunk_bytes: u64) -> Self {
        Self::resume(chunk_bytes, 0, 1)
    }

    /// Continues after a closed chunk: the next chunk has `next_index` and
    /// starts at `next_first_seq`.
    pub fn resume(chunk_bytes: u64, next_index: u64, next_first_seq: u64) -> Self {
        Self {
            chunk_bytes,
            next_index,
            next_seq: next_first_seq,
            first: None,
            last: 0,
            acc: 0,
        }
    }

    fn close(&mut self) -> ChunkSpan {
        let span = ChunkSpan {
            index: self.next_index,
            range: SeqRange {
                first_seq: self.first.take().expect("open chunk"),
                last_seq: self.last,
            },
            rendered_bytes: self.acc,
        };
        self.next_index += 1;
        self.acc = 0;
        span
    }

    /// Feed the next sequence (dense, ascending); returns 0, 1 or 2 closed spans.
    pub fn push(&mut self, seq: u64, rendered: u64) -> Vec<ChunkSpan> {
        debug_assert_eq!(seq, self.next_seq, "chunker input must be dense");
        self.next_seq = seq + 1;
        let mut closed = Vec::new();
        if rendered >= self.chunk_bytes && self.first.is_some() {
            closed.push(self.close());
        }
        self.first.get_or_insert(seq);
        self.last = seq;
        self.acc += rendered;
        if self.acc >= self.chunk_bytes {
            closed.push(self.close());
        }
        closed
    }

    /// The open partial tail after the last closed chunk, if any.
    pub fn tail(&self) -> Option<SeqRange> {
        self.first.map(|first_seq| SeqRange {
            first_seq,
            last_seq: self.last,
        })
    }

    pub fn next_index(&self) -> u64 {
        self.next_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(chunk_bytes: u64, sizes: &[u64]) -> (Vec<ChunkSpan>, Option<SeqRange>) {
        let mut chunker = Chunker::new(chunk_bytes);
        let mut spans = Vec::new();
        for (i, size) in sizes.iter().enumerate() {
            spans.extend(chunker.push(i as u64 + 1, *size));
        }
        (spans, chunker.tail())
    }

    fn r(first_seq: u64, last_seq: u64) -> SeqRange {
        SeqRange {
            first_seq,
            last_seq,
        }
    }

    #[test]
    fn chunks_close_at_first_message_reaching_chunk_bytes() {
        let (spans, tail) = walk(1000, &[400, 400, 300, 500]);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].index, 0);
        assert_eq!(spans[0].range, r(1, 3));
        assert_eq!(spans[0].rendered_bytes, 1100);
        assert_eq!(tail, Some(r(4, 4)));
        // Exactly chunk_bytes closes too.
        let (spans, tail) = walk(1000, &[500, 500, 1]);
        assert_eq!(spans[0].range, r(1, 2));
        assert_eq!(tail, Some(r(3, 3)));
    }

    #[test]
    fn oversized_message_stands_alone_and_closes_the_current_chunk() {
        let (spans, tail) = walk(1000, &[300, 1200, 100]);
        assert_eq!(
            spans.iter().map(|s| (s.index, s.range)).collect::<Vec<_>>(),
            vec![(0, r(1, 1)), (1, r(2, 2))]
        );
        assert_eq!(spans[0].rendered_bytes, 300);
        assert_eq!(spans[1].rendered_bytes, 1200);
        assert_eq!(tail, Some(r(3, 3)));
        let (spans, tail) = walk(1000, &[1200]);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].range, r(1, 1));
        assert_eq!(tail, None);
    }

    #[test]
    fn only_full_chunks_below_the_head() {
        // The loader feeds only sequences <= head. With head = 2 the third
        // message (which would close the chunk) is never fed: no span.
        let (spans, tail) = walk(1000, &[600, 300]);
        assert!(spans.is_empty());
        assert_eq!(tail, Some(r(1, 2)));
        let (spans, _) = walk(1000, &[600, 300, 200]);
        assert_eq!(spans[0].range, r(1, 3));
    }

    fn sizes(n: usize) -> Vec<u64> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..n)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                50 + (state >> 33) % 1500
            })
            .collect()
    }

    #[test]
    fn resume_matches_a_fresh_walk() {
        let sizes = sizes(200);
        let (full, full_tail) = walk(1000, &sizes);
        assert!(full.len() > 10);
        for cut in 0..full.len() {
            let boundary = full[cut];
            let mut chunker =
                Chunker::resume(1000, boundary.index + 1, boundary.range.last_seq + 1);
            let mut rest = Vec::new();
            for seq in boundary.range.last_seq + 1..=sizes.len() as u64 {
                rest.extend(chunker.push(seq, sizes[seq as usize - 1]));
            }
            assert_eq!(rest, full[cut + 1..], "resume after chunk {cut}");
            assert_eq!(chunker.tail(), full_tail);
        }
    }

    #[test]
    fn determinism() {
        let sizes = sizes(200);
        assert_eq!(walk(1000, &sizes), walk(1000, &sizes));
    }
}
