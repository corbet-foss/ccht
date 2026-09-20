// @ts-self-types="./index.d.ts"
import initialize, { ConversationModel } from './wasm/ccht.js';

let initialization;

/** Initialize the shared Rust/Wasm model. Supply an explicit Wasm URL when bundling.
 *
 * Only the first call's `wasm` option initializes the shared module; later
 * calls reuse it. The returned `applyEvent` applies one ordered event and
 * returns the fresh snapshot (the underlying Rust boolean for "changed" is
 * not exposed); it throws on invalid JSON, scope, gap, terminal, or freed
 * state. `snapshot` returns the parsed render state and `free` is idempotent.
 */
export async function createConversation(conversationId, options = {}) {
  if (!initialization) {
    initialization = initialize(options.wasm === undefined ? undefined : { module_or_path: options.wasm })
      .catch((error) => { initialization = undefined; throw error; });
  }
  await initialization;
  const model = new ConversationModel(conversationId);
  let disposed = false;
  const snapshot = () => {
    if (disposed) throw new Error('ccht conversation has been freed');
    return JSON.parse(model.snapshot());
  };
  return {
    applyEvent(event) {
      if (disposed) throw new Error('ccht conversation has been freed');
      model.applyEvent(typeof event === 'string' ? event : JSON.stringify(event));
      return snapshot();
    },
    snapshot,
    free() { if (!disposed) { disposed = true; model.free(); } },
  };
}
