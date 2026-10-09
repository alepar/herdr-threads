// $.state values the herdr-threads mod keeps across module reloads (spec D5, D8).
export type HerdrThreadsTurns = {
  /** Open main-conversation turn ids (events with agentId are ignored). */
  open: string[]
  /** Set after a load with no recorded state until the first turn.complete or 5 s idle. */
  assumedBusy: boolean
  /** Epoch ms of the last aborted main turn while the post-abort hold is active, else null. */
  abortHoldSince: number | null
  /**
   * The batch this instance is submitting, written before `$.prompt.submit` and cleared when it
   * resolves; a successor after a reload holds these ids until their turn starts or completes (spec D5,
   * ht-j16.29). `issued` is false until `$.prompt.submit` was called; a successor settles an unissued
   * record only on a `turn.start` that frames its ids, never from the completion of the turn open at
   * its load (ht-j16.31).
   */
  submitting?: { sid: string; ids: string[]; ackable: string[]; at: number; turnId: string | null; issued?: boolean } | null
}
declare module 'claude-code' {
  interface PluginState {
    'herdr-threads': { turns: HerdrThreadsTurns }
  }
}
