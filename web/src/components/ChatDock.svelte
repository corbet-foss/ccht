<!-- Product-neutral bare chat panel (messages + composer + history). The
  application owns all effects (sending, stopping, history switching,
  clipboard feedback, error surfacing, configuration); this component only
  renders state and forwards user intent through app-supplied callbacks. It
  never fetches, spawns, or stores anything. -->
<script module lang="ts">
  import type { TurnState } from '../../index.js';

  /** One renderable chat message. `activity` carries the live turn detail. */
  export interface ChatDockMessage {
    id: string;
    role: 'user' | 'assistant';
    content: string;
    modelName?: string;
    requestId?: string;
    activity?: TurnState;
  }

  /** One entry in the conversation history picker. */
  export interface ChatHistoryItem {
    id: string;
    title?: string | null;
    message_count?: number;
  }

  /** Default status text for a turn. The application may override it. */
  export function defaultChatStatusLabel(activity: TurnState): string {
    switch (activity.status) {
      case 'streaming':
        return 'Responding…';
      case 'awaiting_permission':
        return 'Needs approval';
      case 'completed':
        return 'Done';
      case 'cancelled':
        return 'Stopped';
      case 'refused':
        return 'Declined';
      case 'failed':
        return 'Failed';
    }
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { TurnState } from '../../index.js';

  let {
    kicker,
    title,
    composerId,
    composerLabel,
    composerPlaceholder,
    sendLabel,
    welcomeTitle,
    welcomeBody,
    welcomeExample,
    newConversationLabel = 'New conversation',
    messages,
    draft = $bindable(''),
    history = [],
    activeHistoryId = null,
    showHistoryPicker = false,
    submitting = false,
    canSend = true,
    controlsLoading = false,
    composerDisabled = false,
    onSend,
    onStop,
    onSelectHistory,
    onCopy,
    onShowActivity,
    statusLabel = defaultChatStatusLabel,
    headerExtra,
    statusNote,
    messageBody,
    activityExtra
  }: {
    kicker: string;
    title: string;
    composerId: string;
    composerLabel: string;
    composerPlaceholder: string;
    sendLabel: string;
    welcomeTitle: string;
    welcomeBody: string;
    welcomeExample: string;
    newConversationLabel?: string;
    messages: ChatDockMessage[];
    draft: string;
    history?: ChatHistoryItem[];
    activeHistoryId?: string | null;
    showHistoryPicker?: boolean;
    submitting?: boolean;
    canSend?: boolean;
    controlsLoading?: boolean;
    composerDisabled?: boolean;
    onSend: (text: string) => void | Promise<void>;
    onStop?: () => void;
    onSelectHistory?: (id: string) => void;
    onCopy?: (id: string, content: string) => void;
    onShowActivity?: (msg: ChatDockMessage) => void;
    statusLabel?: (activity: TurnState) => string;
    headerExtra?: Snippet;
    statusNote?: Snippet;
    messageBody?: Snippet<[ChatDockMessage]>;
    activityExtra?: Snippet<[TurnState]>;
  } = $props();

  const sendDisabled: boolean = $derived(
    submitting || !canSend || controlsLoading || draft.trim().length === 0
  );
  let copiedId: string | null = $state(null);

  function invoke(action: () => void | Promise<void>): void {
    try {
      const result = action();
      if (result instanceof Promise) {
        result.catch(() => {});
      }
    } catch {
      // Handled by the application through its own operation state.
    }
  }

  // The draft is cleared optimistically and restored when the application
  // rejects, so no typed text is lost. The application surfaces the failure
  // through its own operation state.
  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const prompt = draft.trim();
    if (!prompt || submitting || !canSend || controlsLoading) {
      return;
    }
    draft = '';
    try {
      await onSend(prompt);
    } catch {
      if (!draft) {
        draft = prompt;
      }
    }
  }

  function handleComposerKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Enter' || event.shiftKey || event.isComposing) {
      return;
    }
    event.preventDefault();
    if (event.currentTarget instanceof HTMLTextAreaElement) {
      event.currentTarget.form?.requestSubmit();
    }
  }

  // The panel keeps its own Copied feedback; the application may add its own
  // handling through onCopy.
  async function copyMessage(id: string, content: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(content);
      copiedId = id;
    } catch {
      // Clipboard unavailable; the application decides how to report it.
    }
    invoke(() => onCopy?.(id, content));
  }

  function historyTitle(item: ChatHistoryItem): string {
    const label = item.title ?? 'Untitled conversation';
    return typeof item.message_count === 'number'
      ? `${label} · ${item.message_count}`
      : label;
  }
</script>

<div class="ccht-chat-heading">
  <div>
    <p class="ccht-chat-kicker">{kicker}</p>
    <h2 class="ccht-chat-title">{title}</h2>
  </div>
  <div class="ccht-chat-actions">
    {#if messages.length > 0 && onSelectHistory}
      <button type="button" class="ccht-chat-new-chat" onclick={() => invoke(() => onSelectHistory(''))}>
        {newConversationLabel}
      </button>
    {/if}
    {#if headerExtra}{@render headerExtra()}{/if}
  </div>
</div>

{#if statusNote}
  <div class="ccht-chat-status-note">{@render statusNote()}</div>
{/if}

{#if showHistoryPicker}
  <div class="ccht-chat-picker">
    <label for="{composerId}-history">Conversation</label>
    <select
      id="{composerId}-history"
      value={activeHistoryId ?? ''}
      onchange={(event) => invoke(() => onSelectHistory?.(event.currentTarget.value))}
    >
      <option value="">{newConversationLabel}</option>
      {#each history as item (item.id)}
        <option value={item.id}>{historyTitle(item)}</option>
      {/each}
    </select>
  </div>
{/if}

<div class="ccht-chat-messages" aria-live="polite" aria-label="{title} messages">
  {#if messages.length === 0}
    <article class="ccht-chat-message ccht-chat-message-assistant ccht-chat-welcome">
      <strong>{welcomeTitle}</strong>
      <p>{welcomeBody}</p>
      <p class="ccht-chat-example">{welcomeExample}</p>
    </article>
  {/if}
  {#each messages as message (message.id)}
    <article
      class="ccht-chat-message"
      class:ccht-chat-message-user={message.role === 'user'}
      class:ccht-chat-message-assistant={message.role === 'assistant'}
    >
      <strong>{message.role === 'user' ? 'You' : welcomeTitle}</strong>
      {#if messageBody}
        {@render messageBody(message)}
      {:else}
        <p>{message.content}</p>
      {/if}
      {#if message.content}
        <button type="button" class="ccht-chat-copy" onclick={() => void copyMessage(message.id, message.content)}>
          {copiedId === message.id ? 'Copied' : 'Copy'}
        </button>
      {/if}
      {#if message.modelName}<small class="ccht-chat-model">{message.modelName}</small>{/if}
      {#if message.role === 'assistant' && message.requestId && !message.activity && onShowActivity}
        <button type="button" class="ccht-chat-copy" onclick={() => invoke(() => onShowActivity(message))}>
          Show activity
        </button>
      {/if}
      {#if message.activity}
        {@const activity = message.activity}
        {#if activityExtra}
          {@render activityExtra(activity)}
        {:else}
          {#if activity.thought_text}
            <details class="ccht-chat-reasoning">
              <summary>Reasoning</summary>
              <p>{activity.thought_text}</p>
            </details>
          {/if}
          {#each Object.values(activity.tools) as tool (tool.toolCallId)}
            <details class="ccht-chat-tool">
              <summary>{tool.title} · {tool.status ?? 'pending'}</summary>
              <pre>{JSON.stringify(
                { input: tool.rawInput, output: tool.rawOutput, content: tool.content },
                null,
                2
              )}</pre>
            </details>
          {/each}
          {@const usage = activity.updates.usage_update as { used?: unknown; size?: unknown } | undefined}
          {#if typeof usage?.used === 'number'}
            <p class="ccht-chat-usage">
              Context: {usage.used.toLocaleString()} / {typeof usage.size === 'number'
                ? usage.size.toLocaleString()
                : '?'} tokens
            </p>
          {/if}
          {@const plan = activity.updates.plan as
            | { entries?: Array<{ content?: unknown; status?: unknown }> }
            | undefined}
          {#if Array.isArray(plan?.entries) && plan.entries.length > 0}
            <details class="ccht-chat-plan">
              <summary>Plan</summary>
              <ol>
                {#each plan.entries as entry}
                  <li>{String(entry.content ?? '')} · {String(entry.status ?? '')}</li>
                {/each}
              </ol>
            </details>
          {/if}
          {#if activity.permissions.length > 0}
            <p class="ccht-chat-permissions">
              {activity.permissions.length}
              {activity.permissions.length === 1 ? 'permission request' : 'permission requests'}
              pending: {activity.permissions.map((permission) => permission.request_id).join(', ')}
            </p>
          {/if}
          {#if activity.error}
            <p class="ccht-chat-error" role="alert">{activity.error.code}: {activity.error.message}</p>
          {/if}
          <p class="ccht-chat-status">
            <span role="status">{statusLabel(activity)}</span>{#if activity.stop_reason}<span>
                · {activity.stop_reason}</span>{/if}
          </p>
        {/if}
      {/if}
    </article>
  {/each}
</div>

<form class="ccht-chat-composer" onsubmit={submit} aria-busy={submitting}>
  <label class="ccht-chat-sr-only" for={composerId}>{composerLabel}</label>
  <textarea
    id={composerId}
    bind:value={draft}
    onkeydown={handleComposerKeydown}
    rows="3"
    placeholder={composerPlaceholder}
    disabled={composerDisabled}
    required
  ></textarea>
  <div class="ccht-chat-actions">
    {#if submitting && onStop}
      <button type="button" class="ccht-chat-stop" onclick={() => invoke(onStop)}>Stop</button>
    {/if}
    <button type="submit" class="ccht-chat-primary" aria-label={sendLabel} disabled={sendDisabled}>
      {sendLabel}
    </button>
  </div>
</form>

<style>
  .ccht-chat-heading {
    flex: 0 0 auto; display: flex; justify-content: space-between; align-items: start; gap: 0.8rem;
    padding: 0.1rem 0.1rem 0.9rem; border-bottom: 1px solid var(--ccht-border, #223049);
  }
  .ccht-chat-kicker {
    margin: 0; font-size: 0.7rem; font-weight: 850; letter-spacing: 0.12em; text-transform: uppercase;
    color: var(--ccht-kicker, var(--ccht-muted, #8fa0b7));
  }
  .ccht-chat-title { margin: 0.2rem 0 0; font-size: 1.02rem; overflow-wrap: anywhere; }
  .ccht-chat-heading .ccht-chat-actions { display: flex; align-items: center; gap: 0.45rem; }
  .ccht-chat-new-chat {
    min-height: 2.4rem; padding: 0 0.55rem; border: 1px solid var(--ccht-border, #30405c);
    border-radius: 0.5rem; color: var(--ccht-fg, #c2cede); background: var(--ccht-tab-bg, #111c2d);
    font-size: 0.62rem; font-weight: 850; cursor: pointer;
  }
  .ccht-chat-new-chat:hover {
    border-color: var(--ccht-accent, #53708f);
    background: var(--ccht-tab-bg-hover, #17243a);
  }
  .ccht-chat-status-note { flex: 0 0 auto; padding-top: 0.7rem; color: var(--ccht-muted, #8fa0b7); }
  .ccht-chat-picker {
    flex: 0 0 auto; display: grid; grid-template-columns: auto minmax(0, 1fr);
    align-items: center; gap: 0.55rem; padding: 0.7rem 0.1rem 0;
  }
  .ccht-chat-picker label {
    color: var(--ccht-muted, #8fa0b7); font-size: 0.58rem; text-transform: uppercase; font-weight: 750;
  }
  .ccht-chat-picker select {
    min-width: 0; min-height: 2.2rem; padding: 0.4rem 0.5rem;
    border: 1px solid var(--ccht-border, #30405c); border-radius: 0.45rem;
    color: var(--ccht-fg, #c2cede); background: var(--ccht-input-bg, #111c2d);
    font-size: 0.68rem; font-family: inherit;
  }
  .ccht-chat-messages {
    flex: 1 1 auto; min-height: 0; margin: 0.8rem -0.25rem 0; padding: 0 0.25rem 0.5rem;
    overflow: auto; overscroll-behavior: contain;
  }
  .ccht-chat-message {
    margin-bottom: 0.65rem; padding: 0.78rem;
    border: 1px solid var(--ccht-border, #223049); border-radius: 0.65rem;
    background: var(--ccht-message-bg, #111c2d);
  }
  .ccht-chat-message-assistant { background: var(--ccht-message-assistant-bg, #0d1625); }
  .ccht-chat-message-user {
    margin-left: 1.35rem; border-color: var(--ccht-accent, #53708f);
    background: var(--ccht-message-user-bg, #17243a);
  }
  .ccht-chat-message strong { font-size: 0.72rem; letter-spacing: 0.06em; text-transform: uppercase; }
  .ccht-chat-message p {
    margin: 0.35rem 0 0; color: var(--ccht-fg, #c2cede); line-height: 1.55; overflow-wrap: anywhere;
  }
  .ccht-chat-example { color: var(--ccht-muted, #8fa0b7); font-size: 0.75rem; }
  .ccht-chat-copy {
    border: 1px solid var(--ccht-border, #30405c); border-radius: 0.4rem;
    background: var(--ccht-tab-bg, #111c2d); color: var(--ccht-fg, #c2cede);
    padding: 0.3rem 0.5rem; margin-top: 0.4rem; cursor: pointer; font-size: 0.65rem;
  }
  .ccht-chat-model { display: block; color: var(--ccht-muted, #8fa0b7); margin-top: 0.4rem; overflow-wrap: anywhere; }
  .ccht-chat-reasoning, .ccht-chat-plan { margin-top: 0.5rem; color: var(--ccht-muted, #8fa0b7); }
  .ccht-chat-reasoning summary, .ccht-chat-plan summary, .ccht-chat-tool summary {
    cursor: pointer; font-size: 0.75rem;
  }
  .ccht-chat-tool { margin-top: 0.5rem; color: var(--ccht-muted, #8fa0b7); }
  .ccht-chat-tool pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: 0.7rem; }
  .ccht-chat-usage, .ccht-chat-permissions, .ccht-chat-status {
    color: var(--ccht-muted, #8fa0b7); font-size: 0.75rem;
  }
  .ccht-chat-error { color: var(--ccht-error, #b91c1c); }
  .ccht-chat-composer {
    flex: 0 0 auto; margin: 0 -0.1rem -0.1rem; padding: 0.8rem 0.1rem 0.1rem;
    border-top: 1px solid var(--ccht-border, #223049); display: grid; gap: 0.72rem;
  }
  .ccht-chat-composer textarea {
    width: 100%; min-height: 5rem; max-height: 12rem; resize: vertical;
    padding: 0.75rem 0.82rem; border: 1px solid var(--ccht-border, #30405c); border-radius: 0.5rem;
    color: var(--ccht-fg, inherit); background: var(--ccht-input-bg, #111c2d);
    outline: 0; font: inherit; line-height: 1.5;
  }
  .ccht-chat-composer .ccht-chat-actions { display: grid; gap: 0.55rem; }
  .ccht-chat-primary {
    min-height: 2.75rem; padding: 0.78rem 1rem; border: 0; border-radius: 0.5rem;
    color: var(--ccht-primary-fg, #e2eaf5); background: var(--ccht-primary-bg, #53708f);
    font-weight: 850; cursor: pointer; width: 100%;
  }
  .ccht-chat-primary:hover { background: var(--ccht-primary-bg-hover, #17243a); }
  .ccht-chat-primary:disabled { cursor: not-allowed; opacity: 0.48; }
  .ccht-chat-stop {
    min-height: 2.4rem; padding: 0.55rem 0.85rem; border: 1px solid var(--ccht-border, #30405c);
    border-radius: 0.5rem; color: var(--ccht-fg, #c2cede); background: var(--ccht-tab-bg, #111c2d);
    font-size: 0.72rem; font-weight: 850; cursor: pointer;
  }
  .ccht-chat-sr-only {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0;
  }
  @media (max-width: 640px) {
    .ccht-chat-message-user { margin-left: 0.75rem; }
  }
</style>
