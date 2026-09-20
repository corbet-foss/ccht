/** Browser behavior of the generic bare chat panel (ChatDock.svelte).
 *
 * Covers welcome/message rendering, copy feedback, the send lifecycle
 * (draft clear and restore, disabled gates, Enter/isComposing handling),
 * Stop, history selection, activity gating, and status labels. The default
 * activity renderer is exercised through fixture TurnState values.
 */
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import { afterEach, describe, expect, test, vi } from 'vitest';
import ChatDock from './ChatDock.svelte';

afterEach(() => cleanup());

const copy = {
  kicker: 'Creator · diverges',
  title: 'Brief the generator',
  composerId: 'creator-prompt',
  composerLabel: 'Message the creator',
  composerPlaceholder: 'Describe the name you need…',
  sendLabel: 'Send to creator',
  welcomeTitle: 'creator',
  welcomeBody: 'Talk through the problem first.',
  welcomeExample: 'Try: short .ch names.',
};

function openChat(props: Record<string, unknown> = {}) {
  return render(ChatDock, {
    ...copy,
    messages: [],
    draft: '',
    onSend: () => {},
    ...props,
  });
}

function turn(overrides: Record<string, unknown> = {}) {
  return {
    request_id: 'req-1',
    last_sequence: 2,
    text: 'hello',
    user_text: '',
    user_content: [],
    thought_text: '',
    thought_content: [],
    content: [],
    tools: {},
    permissions: [],
    updates: {},
    status: 'completed',
    stop_reason: 'end_turn',
    error: null,
    ...overrides,
  };
}

describe('welcome and messages', () => {
  test('empty thread shows the welcome article', () => {
    const { getByText } = openChat();
    expect(getByText('Talk through the problem first.')).toBeTruthy();
    expect(getByText('Try: short .ch names.')).toBeTruthy();
  });

  test('user and assistant messages render with copy controls', async () => {
    const onCopy = vi.fn();
    const { getByText, getAllByRole } = openChat({
      onCopy,
      messages: [
        { id: 'u1', role: 'user', content: 'a name' },
        { id: 'a1', role: 'assistant', content: 'how about x', modelName: 'model-x' },
      ],
    });
    expect(getByText('a name')).toBeTruthy();
    expect(getByText('model-x')).toBeTruthy();
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
      configurable: true,
    });
    await fireEvent.click(getAllByRole('button', { name: 'Copy' })[0]);
    expect(onCopy).toHaveBeenCalledWith('u1', 'a name');
    expect(getByText('Copied')).toBeTruthy();
  });

  test('assistant without activity offers Show activity only with a handler', () => {
    const bare = openChat({ messages: [{ id: 'a1', role: 'assistant', content: 'hi', requestId: 'r1' }] });
    expect(bare.queryByRole('button', { name: 'Show activity' })).toBeNull();
    const wired = openChat({
      onShowActivity: () => {},
      messages: [{ id: 'a1', role: 'assistant', content: 'hi', requestId: 'r1' }],
    });
    expect(wired.getByRole('button', { name: 'Show activity' })).toBeTruthy();
  });
});

describe('default activity renderer', () => {
  test('covers reasoning, tools, usage, plan, permissions, error, and status', () => {
    const { getByText } = openChat({
      messages: [
        {
          id: 'a1',
          role: 'assistant',
          content: 'results',
          activity: turn({
            thought_text: 'thinking out loud',
            tools: { t1: { toolCallId: 't1', title: 'Lookup', status: 'done', rawInput: { q: 1 } } },
            updates: { usage_update: { used: 1200, size: 8000 }, plan: { entries: [{ content: 'step', status: 'done' }] } },
            permissions: [{ request_id: 'p1', request: {} }],
            error: { code: 'boom', message: 'went wrong' },
            status: 'failed',
            stop_reason: 'error',
          }),
        },
      ],
    });
    expect(getByText('Reasoning')).toBeTruthy();
    expect(getByText('thinking out loud')).toBeTruthy();
    expect(getByText('Lookup · done')).toBeTruthy();
    expect(getByText('Context: 1,200 / 8,000 tokens')).toBeTruthy();
    expect(getByText('Plan')).toBeTruthy();
    expect(getByText(/1 permission request pending: p1/)).toBeTruthy();
    expect(getByText('boom: went wrong')).toBeTruthy();
    expect(getByText('Failed')).toBeTruthy();
    expect(getByText(/· error/)).toBeTruthy();
  });

  test('status labels cover every turn state', () => {
    const cases: Array<[string, string]> = [
      ['streaming', 'Responding…'],
      ['awaiting_permission', 'Needs approval'],
      ['completed', 'Done'],
      ['cancelled', 'Stopped'],
      ['refused', 'Declined'],
      ['failed', 'Failed'],
    ];
    for (const [status, label] of cases) {
      const view = openChat({ messages: [{ id: 'a1', role: 'assistant', content: 'x', activity: turn({ status }) }] });
      expect(view.getByText(label)).toBeTruthy();
      view.unmount();
    }
  });
});

describe('composer', () => {
  test('send clears the draft and forwards trimmed text', async () => {
    const onSend = vi.fn();
    const { getByLabelText, getByRole } = openChat({ draft: '  hello  ', onSend });
    await fireEvent.click(getByRole('button', { name: 'Send to creator' }));
    expect(onSend).toHaveBeenCalledWith('hello');
    expect((getByLabelText('Message the creator') as HTMLTextAreaElement).value).toBe('');
  });

  test('rejected send restores the draft', async () => {
    const { getByLabelText, getByRole } = openChat({ draft: 'keep me', onSend: () => Promise.reject(new Error('down')) });
    await fireEvent.click(getByRole('button', { name: 'Send to creator' }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect((getByLabelText('Message the creator') as HTMLTextAreaElement).value).toBe('keep me');
  });

  test('send is gated on canSend, loading, and blank drafts', () => {
    for (const props of [{ draft: 'x', canSend: false }, { draft: 'x', controlsLoading: true }, { draft: '   ' }, { draft: 'x', submitting: true }]) {
      const view = openChat(props);
      expect(view.getByRole('button', { name: 'Send to creator' }).hasAttribute('disabled')).toBe(true);
      view.unmount();
    }
    const ready = openChat({ draft: 'x' });
    expect(ready.getByRole('button', { name: 'Send to creator' }).hasAttribute('disabled')).toBe(false);
  });

  test('Enter submits, shift+Enter and composing do not', async () => {
    const onSend = vi.fn();
    const { getByLabelText } = openChat({ draft: 'hi', onSend });
    const box = getByLabelText('Message the creator');
    await fireEvent.keyDown(box, { key: 'Enter', shiftKey: true });
    await fireEvent.keyDown(box, { key: 'Enter', isComposing: true });
    expect(onSend).not.toHaveBeenCalled();
    await fireEvent.keyDown(box, { key: 'Enter' });
    expect(onSend).toHaveBeenCalledWith('hi');
  });

  test('Stop appears only while submitting with a handler', () => {
    expect(openChat({ submitting: true }).queryByRole('button', { name: 'Stop' })).toBeNull();
    const onStop = vi.fn();
    const view = openChat({ submitting: true, onStop });
    expect(view.getByRole('button', { name: 'Stop' })).toBeTruthy();
  });

  test('composer honours the disabled flag', () => {
    const { getByLabelText } = openChat({ composerDisabled: true });
    expect((getByLabelText('Message the creator') as HTMLTextAreaElement).disabled).toBe(true);
  });
});

describe('history', () => {
  test('picker lists conversations and selects through the callback', async () => {
    const onSelectHistory = vi.fn();
    const { getByLabelText } = openChat({
      showHistoryPicker: true,
      onSelectHistory,
      activeHistoryId: 'c1',
      history: [
        { id: 'c1', title: 'First', message_count: 4 },
        { id: 'c2', title: null, message_count: 1 },
      ],
    });
    const select = getByLabelText('Conversation') as HTMLSelectElement;
    expect(select.value).toBe('c1');
    await fireEvent.change(select, { target: { value: 'c2' } });
    expect(onSelectHistory).toHaveBeenCalledWith('c2');
  });

  test('no picker without the flag', () => {
    expect(openChat().queryByLabelText('Conversation')).toBeNull();
  });

  test('new-conversation voice is product-owned', () => {
    const view = openChat({
      newConversationLabel: 'New chat',
      onSelectHistory: () => {},
      messages: [{ id: 'u1', role: 'user', content: 'hi' }],
      showHistoryPicker: true,
    });
    expect(view.getByRole('button', { name: 'New chat' })).toBeTruthy();
    expect(view.getByRole('option', { name: 'New chat' })).toBeTruthy();
  });
});
