<!-- Product-neutral account connector. The application backend owns all effects
  (starting device flows, polling logins, storing keys, signing out); this
  component only renders state and forwards user intent through app-supplied
  async callbacks. It never spawns agents, polls, or touches credentials. -->
<script lang="ts">
  import {
    authAccount,
    isAuthenticated,
    validateChallenge,
    type AuthState,
    type Challenge
  } from '../auth.js';

  interface AccountRef {
    id: string;
    label: string;
    kind: 'chatgpt' | 'opencode_go' | (string & {});
  }

  let {
    account,
    state: authState = 'unknown' as AuthState,
    busy = false,
    challenge = null as Challenge | null,
    onStart,
    onCancel,
    onKeySubmit,
    onDisconnect
  }: {
    account: AccountRef;
    state: AuthState;
    busy: boolean;
    challenge?: Challenge | null;
    onStart: () => void | Promise<void>;
    onCancel: () => void | Promise<void>;
    onKeySubmit: (key: string) => void | Promise<void>;
    onDisconnect?: () => void | Promise<void>;
  } = $props();

  let key = $state('');
  let copyError = $state('');

  // Renamed locally: a binding called `state` would shadow the `$state` rune analysis.
  const authenticated: boolean = $derived(isAuthenticated(authState));
  const accountName: string | null = $derived(authAccount(authState));
  const validChallenge: Challenge | null = $derived(
    challenge != null && validateChallenge(challenge) ? challenge : null
  );
  const isDeviceCodeAccount: boolean = $derived(account.kind === 'chatgpt');
  const isKeyAccount: boolean = $derived(account.kind === 'opencode_go');
  const keyInputId: string = $derived(`ccht-key-${account.id.replace(/[^a-zA-Z0-9_-]/g, '-')}`);
  const canSubmitKey: boolean = $derived(key.trim().length >= 8 && !busy);

  // Callbacks run application effects; the app surfaces their failures
  // through its own operation state, so a rejection here stays silent.
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

  function submitKey(event: SubmitEvent): void {
    event.preventDefault();
    const value = key.trim();
    // Clear immediately so the secret never lingers in component state.
    key = '';
    if (value.length < 8) {
      return;
    }
    invoke(() => onKeySubmit(value));
  }

  async function copyAndContinue(): Promise<void> {
    if (!validChallenge) {
      return;
    }
    // Open during the click so browsers do not block the sign-in window after copying.
    if (typeof window !== 'undefined') {
      window.open(validChallenge.verification_url, '_blank', 'noopener,noreferrer');
    }
    try {
      await navigator.clipboard.writeText(validChallenge.user_code);
      copyError = '';
    } catch {
      copyError = 'Could not copy automatically. Copy the code above into the sign-in page.';
    }
  }
</script>

<section class="ccht-account" aria-label={`${account.label} connection`}>
  {#if authenticated}
    <p class="ccht-status" role="status">Connected{#if accountName} · {accountName}{/if}</p>
    <button
      type="button"
      class="ccht-button ccht-disconnect"
      disabled={busy}
      onclick={() => invoke(onDisconnect ?? onCancel)}
    >
      Disconnect
    </button>
  {:else if authState === 'unknown'}
    <p class="ccht-status" role="status">Checking {account.label} connection status…</p>
    {#if busy}
      <button type="button" class="ccht-button ccht-cancel" onclick={() => invoke(onCancel)}>
        Cancel
      </button>
    {/if}
  {:else if isDeviceCodeAccount}
    {#if validChallenge}
      <div class="ccht-challenge" aria-live="polite">
        <p class="ccht-hint">Enter this code on the {account.label} sign-in page:</p>
        <code class="ccht-code">{validChallenge.user_code}</code>
        <button type="button" class="ccht-button" disabled={busy} onclick={() => void copyAndContinue()}>
          Copy code and continue
        </button>
        <p>
          <a
            class="ccht-link"
            href={validChallenge.verification_url}
            target="_blank"
            rel="noopener noreferrer"
          >
            Continue to {account.label}
          </a>
        </p>
        {#if copyError}<p class="ccht-error" role="alert">{copyError}</p>{/if}
      </div>
    {:else if busy}
      <p class="ccht-status" role="status">Starting account connection…</p>
    {:else}
      <p class="ccht-hint">Sign in with your {account.label} account.</p>
      <button type="button" class="ccht-button" onclick={() => invoke(onStart)}>
        Sign in to {account.label}
      </button>
    {/if}
    {#if busy}
      <button type="button" class="ccht-button ccht-cancel" onclick={() => invoke(onCancel)}>
        Cancel
      </button>
    {/if}
  {:else if isKeyAccount}
    <form class="ccht-keyform" onsubmit={submitKey}>
      <label class="ccht-label" for={keyInputId}>
        {account.label} account key
        <input
          id={keyInputId}
          class="ccht-input"
          type="password"
          autocomplete="off"
          spellcheck="false"
          maxlength="4096"
          bind:value={key}
          disabled={busy}
        />
      </label>
      <button type="submit" class="ccht-button" disabled={!canSubmitKey}>
        Connect {account.label}
      </button>
    </form>
    {#if busy}
      <button type="button" class="ccht-button ccht-cancel" onclick={() => invoke(onCancel)}>
        Cancel
      </button>
    {/if}
  {:else}
    <p class="ccht-status" role="status">Not connected</p>
    <p class="ccht-hint">Sign in with your {account.label} account.</p>
    <button type="button" class="ccht-button" disabled={busy} onclick={() => invoke(onStart)}>
      Sign in to {account.label}
    </button>
    {#if busy}
      <button type="button" class="ccht-button ccht-cancel" onclick={() => invoke(onCancel)}>
        Cancel
      </button>
    {/if}
  {/if}
</section>

<style>
  .ccht-account {
    color: var(--ccht-fg, inherit);
  }
  .ccht-status,
  .ccht-hint {
    color: var(--ccht-muted, inherit);
  }
  .ccht-error {
    color: var(--ccht-error, #b91c1c);
  }
  .ccht-code {
    background-color: var(--ccht-code-bg, transparent);
  }
  .ccht-link {
    color: var(--ccht-accent, inherit);
  }
</style>
