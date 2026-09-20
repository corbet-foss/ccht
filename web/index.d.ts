/** Upstream ACP payloads are preserved; applications can narrow their supported kinds. */
export interface AcpUpdate { sessionUpdate: string; [key: string]: unknown }
export interface AcpToolCall { toolCallId: string; title: string; status?: string; [key: string]: unknown }
export interface PermissionRequest {
  sessionId: string;
  toolCall: { toolCallId: string; [key: string]: unknown };
  options: Array<{ optionId: string; name: string; kind: string }>;
  [key: string]: unknown;
}
export type CchtEvent =
  | { kind: 'update'; update: AcpUpdate }
  | { kind: 'permission'; request_id: string; request: PermissionRequest }
  | { kind: 'completed'; stop_reason: string }
  | { kind: 'error'; code: string; message: string };
export interface WireEvent {
  version: 1;
  conversation_id: string;
  request_id: string;
  sequence: number;
  event: CchtEvent;
}
export interface TurnState {
  request_id: string;
  last_sequence: number;
  text: string;
  user_text: string;
  user_content: Array<Record<string, unknown>>;
  thought_text: string;
  thought_content: Array<Record<string, unknown>>;
  content: Array<Record<string, unknown>>;
  tools: Record<string, AcpToolCall>;
  permissions: Array<{ request_id: string; request: PermissionRequest }>;
  updates: Record<string, unknown>;
  status: 'streaming' | 'awaiting_permission' | 'completed' | 'cancelled' | 'refused' | 'failed';
  stop_reason: string | null;
  error: { code: string; message: string } | null;
}
export interface ConfigChoice { value: string; name: string; description?: string }
export interface ConfigGroup { group: string; name: string; options: ConfigChoice[] }
export type SessionConfigOption = {
  id: string; name: string; description?: string; category?: string;
} & ({ type: 'select'; currentValue: string; options: ConfigChoice[] | ConfigGroup[] }
  | { type: 'boolean'; currentValue: boolean });
export interface SessionConfiguration {
  options: SessionConfigOption[];
  modes: null | { currentModeId: string; availableModes: Array<{id: string; name: string; description?: string}> };
}
export interface ConversationSnapshot { conversation_id: string; turns: TurnState[]; configuration: SessionConfiguration }
export interface Conversation {
  applyEvent(event: WireEvent | string): ConversationSnapshot;
  snapshot(): ConversationSnapshot;
  free(): void;
}
/** Transport/authentication and persistence belong to the embedding application.
 *
 * Only the first call's `wasm` option initializes the shared module; later
 * calls reuse it. `applyEvent` returns the fresh snapshot and throws on
 * invalid, out-of-scope, gapped, terminal, or freed input. `free` is idempotent.
 */
export function createConversation(conversationId: string, options?: {
  wasm?: URL | string | Request | Response | BufferSource | WebAssembly.Module;
}): Promise<Conversation>;
