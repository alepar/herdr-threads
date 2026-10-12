// Pending actual capture: never substitute manufactured success bytes. The
// source-only checkpoint fails explicitly until the parent supplies the real
// elected advertised Watch stream and its producer/build/hash provenance.
export default {
  raw: '',
  provenance: null,
  ids: { lazy: '', message: '', attention: '' },
  passiveHeaders: [] as string[],
  activeHeaders: [] as string[],
  nonmessageHeaders: [] as string[],
  messageHeaders: [] as string[],
}
