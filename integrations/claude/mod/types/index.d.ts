// $.state values the herdr-threads mod keeps across module reloads (spec D5, D8).
export type HerdrThreadsTurns = {
  /** Open main-conversation turn ids (events with agentId are ignored). */
  open: string[]
  /** Set after a load with no recorded state until the first turn.complete or 5 s idle. */
  assumedBusy: boolean
  /** Epoch ms of the last aborted main turn while the post-abort hold is active, else null. */
  abortHoldSince: number | null
}
declare module 'claude-code' {
  interface PluginState {
    'herdr-threads': { turns: HerdrThreadsTurns }
  }
}
