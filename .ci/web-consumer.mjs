import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createConversation } from '@corbet-labs/ccht';

const wasm = await readFile(new URL(import.meta.resolve('@corbet-labs/ccht/ccht_bg.wasm')));
const creator = await createConversation('creator', { wasm });
const critic = await createConversation('critic');
const event = (sequence, value, conversation_id = 'creator') => ({
  version: 1, conversation_id, request_id: 'turn-1', sequence, event: value,
});
const text = (value) => ({ kind: 'update', update: {
  sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: value },
} });
const first = event(1, text('Hello'));
creator.applyEvent(first);
assert.equal(creator.applyEvent(first).turns[0].text, 'Hello');
assert.throws(() => critic.applyEvent(first));
assert.deepEqual(critic.snapshot().turns, []);
assert.throws(() => creator.applyEvent(event(3, text('gap'))));
assert.equal(creator.snapshot().turns[0].text, 'Hello');
creator.applyEvent(event(2, text(' world')));
creator.applyEvent(event(3, { kind: 'update', update: {
  sessionUpdate: 'tool_call', toolCallId: 'tool-1', title: 'Read document', status: 'pending',
} }));
creator.applyEvent(event(4, { kind: 'update', update: {
  sessionUpdate: 'tool_call_update', toolCallId: 'tool-1', status: 'completed',
} }));
assert.equal(creator.snapshot().turns[0].tools['tool-1'].status, 'completed');
creator.applyEvent(event(5, { kind: 'completed', stop_reason: 'cancelled' }));
assert.equal(creator.snapshot().turns[0].status, 'cancelled');
assert.equal(creator.snapshot().turns[0].text, 'Hello world');
assert.throws(() => creator.applyEvent(event(6, text('late'))));
assert.throws(() => creator.applyEvent('{'));
const settings = await createConversation('settings');
settings.applyEvent({version:1, conversation_id:'settings', request_id:'controls', sequence:1,
  event:{kind:'update',update:{sessionUpdate:'config_option_update',configOptions:[
    {id:'model',name:'Model',category:'model',type:'select',currentValue:'muse',options:[{value:'muse',name:'Muse'}]}
  ]}}});
assert.equal(settings.snapshot().configuration.options[0].currentValue, 'muse');
settings.free();
creator.free(); creator.free();
assert.throws(() => creator.snapshot());
critic.free();
console.log('Packaged Wasm consumer passed: isolated scopes, replay, gaps, tool updates, cancellation, disposal.');
