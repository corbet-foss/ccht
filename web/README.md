# @corbet-labs/ccht

Reusable conversations for your applications, powered by the shared Rust/Wasm model.

The root entry (`createConversation`) owns the conversation state machine. The
`./auth` entry adds typed auth/challenge models mirroring `ccht::auth`, the
`./dock` entry adds framework-free dock state, and
`./components/AccountConnection.svelte` adds the first shared Svelte 5 account
connector alongside the generic `./components/Dock.svelte` edge rail. All
entries are product-neutral: no product names, prompts, or
pricing appear in this package.

```js
import { createConversation } from '@corbet-labs/ccht';
import { isAuthenticated, validateChallenge } from '@corbet-labs/ccht/auth';
import { createDockManager, validateDockId } from '@corbet-labs/ccht/dock';
import AccountConnection from '@corbet-labs/ccht/components/AccountConnection.svelte';
import Dock from '@corbet-labs/ccht/components/Dock.svelte';
```

The Svelte component ships as source and needs `svelte` `^5` (a peer
dependency). It has no other dependencies and imports no CSS framework.

## What is shared vs what the app owns

Shared (this package):

- Conversation state and ordered event replay (Wasm `ConversationModel`).
- Auth/challenge types and validators (`AuthState`, `Challenge`,
  `CredentialsProvider`).
- Presentational account connector markup with stable styling hooks.

Owned by the embedding application:

- Transport: delivering `WireEvent`s (HTTP/WebSocket/SSE), serving the Wasm
  bytes, and polling login operations until they complete.
- Effects: spawning agents, starting or cancelling logins, signing out. These
  never run in browser library code; the component drives them through
  app-supplied async callbacks and the app backend does the work.
- Storage: where credentials persist (OS keychain natively, origin-scoped
  browser storage on the web). The library defines the interface only.
- Product copy: account labels, help text, links, and error wording beyond the
  component's generic strings.

Rule: browser code in this package never spawns agents, performs network I/O,
or holds ambient authority. Service keys are explicit strings passed by the
app, and secrets are opaque bytes that are never logged.

## Auth entry (`@corbet-labs/ccht/auth`)

Plain types and functions, no DOM access.

`AuthState` mirrors the Rust `snake_case` JSON byte-for-byte:

| Rust | JSON | TypeScript |
| --- | --- | --- |
| `Unknown` | `"unknown"` | `'unknown'` |
| `Unauthenticated` | `"unauthenticated"` | `'unauthenticated'` |
| `Authenticated { account }` | `{"authenticated": {"account": "…"}}` | `{ authenticated: { account?: string \| null } }` |

Helpers: `isAuthenticated(state)` mirrors `AuthState::authenticated`,
`authAccount(state)` returns the display-only label (or null), and
`parseAuthState(value)` / `isAuthState(value)` validate decoded JSON. The
label is provenance for the UI, never an authentication proof.

`Challenge` is `{ verification_url, user_code }`. The backend issues it, pins
any provider-specific endpoint, and polls for completion; the browser only
renders a validated challenge. `validateChallenge(value)` accepts an `https:`
URL with a host (no embedded credentials) and a non-empty code of up to 64
ASCII alphanumeric or `-` characters. It never throws, so callers can gate
rendering on it. The component validates internally and renders waiting text
for a missing or malformed challenge instead of an unvalidated link.

`CredentialsProvider` mirrors the Rust trait with JS method names:

```ts
interface CredentialsProvider {
  readCredentials(service: string): Promise<Credential | null>;
  writeCredentials(service: string, credential: Credential): Promise<void>;
  deleteCredentials(service: string): Promise<void>;
}
```

`Credential` is `{ account: string; secret: Uint8Array }`. Encode at the app
boundary (`TextEncoder`/base64) when the store needs text. Service keys are
app-defined (for example `assistant-login:<provider>`); deleting a missing
entry succeeds, and a rejected promise signals store failure. There is no
environment-variable override on the web: the store alone decides, matching
the documented `ApiKeyState` behavior on Wasm.

## Component (`AccountConnection.svelte`)

Svelte 5 runes. Props:

| Prop | Type | Meaning |
| --- | --- | --- |
| `account` | `{ id: string; label: string; kind: 'chatgpt' \| 'opencode_go' \| string }` | Product-neutral identity. Known kinds select the ceremony; unknown kinds fall back to a generic sign-in button. |
| `state` | `AuthState` | Observed login state (defaults to `'unknown'`). |
| `busy` | `boolean` | An app-owned operation is in flight; actions disable, cancel shows. |
| `challenge` | `Challenge \| null` (optional) | Device-code challenge supplied by the app after `onStart` starts polling. |
| `onStart` | `() => void \| Promise<void>` | Begin the login ceremony (for example start a device flow). |
| `onCancel` | `() => void \| Promise<void>` | Abort the in-flight operation; also backs Disconnect when `onDisconnect` is omitted. |
| `onKeySubmit` | `(key: string) => void \| Promise<void>` | Persist a trimmed account key (minimum 8 characters). |
| `onDisconnect` | optional | Sign out when authenticated. Falls back to `onCancel`, which can branch on app state. |

Rendering:

- Unauthenticated `chatgpt`: sign-in button, then waiting text while `busy`,
  then the device-code view (code, copy button, sign-in link) once a valid
  `challenge` arrives. The copy button opens the link during the click (so
  popup blockers do not trigger) and falls back to a manual-copy message when
  the clipboard API is unavailable.
- Unauthenticated `opencode_go`: password key form that clears on submit.
- Authenticated: connection status with the account label (when known) and a
  Disconnect button.
- `unknown` state: neutral checking text with no ceremony buttons.

Callbacks are fire-and-forget from the component's view: the app must handle
its own failures and surface them through its own operation state, which it
renders outside the component. Callback rejections are therefore ignored here
by design.

Accessibility: semantic `section`/`form`/`label`/`button` elements, an
`aria-label` naming the account connection, `role="status"` for status lines,
an `aria-live="polite"` region announcing challenge arrival, and
`role="alert"` for the clipboard fallback. The key input is labelled,
password-masked, and non-persistent.

## Dock entry (`@corbet-labs/ccht/dock`) and component (`Dock.svelte`)

Framework-free dock state plus a generic Svelte 5 edge rail. `DockKind`
(`'chat' | 'config' | 'custom'`) classifies a dock and `Placement`
(`'left' | 'right' | 'bottom' | 'inline'`) places it. The state module is
plain TypeScript with no DOM access.

`validateDockId(id)` returns an error message or null (never throws). Rules:
a non-empty string of at most 64 ASCII `[a-z0-9-_]` characters.

`createDockManager()` returns an independent manager:

| Method | Meaning |
| --- | --- |
| `register(id, kind, placement)` | Register a closed dock. Throws the validation message for a malformed id, `DuplicateDock: <id>` for a repeat. |
| `open(id)` / `close(id)` | Mark a dock open or closed. Unknown ids throw `UnknownDock: <id>`. |
| `openWithFocus(id, token)` | Open a dock and stage an opaque focus token (non-empty string) for the app to consume. |
| `toggle(id)` | Flip a dock's open state and return the new state. |
| `isOpen(id)` | Whether a dock is open. |
| `placement(id)` / `setPlacement(id, placement)` | Read or move a dock's placement. Invalid placements throw `invalid dock placement`. |
| `openDocks()` | Ids of open docks, in registration order. |
| `takeFocusToken()` | Take the staged focus token once, clearing it; null when none is staged. |
| `closeAll()` | Mark every dock closed. |
| `serialize()` | Persist `[{id, kind, placement, open}]` as JSON, in registration order. The focus token is never persisted. |
| `restore(json)` | Replace all state from `serialize` output. Every entry is validated first, so a malformed snapshot (thrown as `invalid dock snapshot: …`) leaves current state untouched, and any staged focus token is cleared. |

`Dock.svelte` renders one named dock as an edge rail for the `left` and
`right` placements. Svelte 5 runes. Props:

| Prop | Type | Meaning |
| --- | --- | --- |
| `side` | `'left' \| 'right'` | Which window edge the rail pins to. |
| `title` | `string` | Panel heading and dialog label. |
| `open` | `boolean` | App-owned visibility state. |
| `onClose` | `() => void` | Close intent (tab is hidden while open; Esc key also closes). |
| `onOpen` | `() => void` | Open intent from the edge tab button. |
| `tabLabel` | `string` | Short tab caption. |
| `tabSummary` | `string` (optional, defaults to `''`) | Detail shown in the tab button and panel head. |
| `closeLabel` | optional `string` | Close button label (defaults to `Close {title}`). |
| `panelId` | `string` | Panel element id, referenced by the tab's `aria-controls`. |
| `children` | `Snippet` | Panel body content. |

Rendering:

- Closed: a fixed edge tab button (`aria-expanded="false"`,
  `aria-controls={panelId}`, labelled `Open {title}: {tabSummary}` or
  `Open {title}`). The panel stays mounted but hidden and inert.
- Open: a `role="dialog"` panel with a head (`h2` title, summary paragraph,
  Close button) and a body rendering `children`.
- Opening moves focus to the first visible, enabled control in the panel
  (or the panel itself); closing returns focus to the tab or the previously
  focused element. Esc closes.
- The `bottom` and `inline` placements are state-only: they persist and
  restore through the manager but have no rail chrome here. Custom hosts
  (for example a ratatui overlay or an inline sheet) render them.

Stable classes: `ccht-dock-tab`, `ccht-dock-tab-text`, `ccht-dock-panel`,
`ccht-dock-head`, `ccht-dock-title`, `ccht-dock-close`, `ccht-dock-body`.
The tab and panel carry `data-side` (`left` / `right`) and the panel carries
`data-open` (`true` / `false`).

CSS variables (with fallbacks when unset): `--ccht-fg`, `--ccht-muted`,
`--ccht-accent`, `--ccht-border`, `--ccht-panel-bg`, `--ccht-tab-bg`,
`--ccht-tab-bg-hover`, `--ccht-fg-bright`.

Rule: the app owns dock effects. The manager never persists, the component
never fetches, spawns, or stores; visibility, persistence, and focus targets
beyond the panel run through app callbacks and app-held state.

## Component (`ChatDock.svelte`)

Product-neutral bare chat panel: messages, composer, and history. All copy
is a prop; all effects are app callbacks. Import product types from the
component module:

```js
import ChatDock, {
  defaultChatStatusLabel,
} from '@corbet-labs/ccht/components/ChatDock.svelte';
```

`ChatDockMessage` is `{ id, role: 'user' | 'assistant', content,
modelName?, requestId?, activity?: TurnState }` (reusing the shared
`TurnState` vocabulary, including all six turn statuses, `stop_reason`,
tools, usage updates, plans, permission requests, and errors).
`ChatHistoryItem` is `{ id, title?, message_count? }`.

Props: `kicker`, `title`, composer `composerId/Label/Placeholder`,
`sendLabel`, welcome `welcomeTitle/Body/Example`, `messages`,
`draft` (bindable), `history` + `activeHistoryId` + `showHistoryPicker`,
`newConversationLabel?` (default `'New conversation'`),
`sending`, `canSend`, `controlsLoading`, `composerDisabled`,
`onSend(text)`, `onStop?`, `onSelectHistory?(id)` (`''` means new),
`onCopy?(id, content)`, `onShowActivity?(msg)`,
`statusLabel?` (defaults to `defaultChatStatusLabel`: Responding… / Needs
approval / Done / Stopped / Declined / Failed).

Optional snippets: `headerExtra` (for example a run badge),
`statusNote` (for example a readiness hint pointing at configuration),
`messageBody(message)` (default: plain paragraph; Markdown rendering stays
app-owned), `activityExtra(activity)` (default generic renderer: Reasoning
details, tool list with raw I/O, context usage, plan, permission
request ids, error, and status with `stop_reason`).

Behavior: Enter without Shift submits (with an `isComposing` guard); the
draft clears optimistically and is restored when `onSend` rejects; Send is
disabled while submitting, `!canSend`, `controlsLoading`, or a blank draft;
Stop renders only while submitting with a handler; Show activity renders
only for assistant turns with a `requestId`, no activity yet, and a
handler. Copy keeps its own Copied feedback and calls `onCopy` too.

Stable classes: `ccht-chat-heading/kicker/title/actions/new-chat`,
`ccht-chat-status-note`, `ccht-chat-picker`, `ccht-chat-messages/message/
message-user/message-assistant/welcome/example/copy/model/reasoning/tool/
usage/plan/permissions/error/status`, `ccht-chat-composer/primary/stop`,
`ccht-chat-sr-only`. Colors resolve through `--ccht-*` variables with
plain fallbacks (see the component source for the full list).

## Component (`StepConfig.svelte`)

Generic per-step backend/model/options form (content only; the app wraps
it in `Dock`). `StepData` carries one step's `id`, `label`,
`backendValue`, `backendOptions`, `backendPlaceholder`, `accountConnected`,
`accountBusy`, `configurationError?`, `modelValue`, `modelOptions`,
`refreshable` (whether the step owns a Refresh row), `extraOptions`
(`SessionConfigOption[]`), and `extraValues`.

Props: `steps: StepData[]`, `refreshing`, `onSelectBackend(id, value)`,
`onSelectModel(id, value)`,
`onSelectOption(id, option, value)`, `onRefresh(id)`. Optional snippets:
`accountSlot({ step })` (rendered for disconnected steps; the app wires
`AccountConnection` there), `headerCopy`, `footerSlot`.

Each step renders a `fieldset`/`legend` group with `{label} assistant`
and `{label} model` selects (stable ids `ccht-step-{id}-assistant`,
`-model`, `-option-{opt}`), model-category options filtered out of the
extras, grouped select choices flattened in order, per-step Refresh with
loading state, and configuration errors as alerts. The `bottom`/`inline`
placements have no rail chrome; like `Dock`, this component never fetches,
spawns, or stores.

## Styling hooks

Unstyled-but-hooked markup: layout comes from the app, colors resolve through
CSS custom properties with plain fallbacks. Stable classes:

`ccht-account`, `ccht-status`, `ccht-hint`, `ccht-error`, `ccht-challenge`,
`ccht-code`, `ccht-button`, `ccht-cancel`, `ccht-disconnect`, `ccht-link`,
`ccht-keyform`, `ccht-label`, `ccht-input`.

CSS variables (with fallbacks when unset): `--ccht-fg`, `--ccht-muted`,
`--ccht-error`, `--ccht-accent`, `--ccht-code-bg`. Example:

```css
:root {
  --ccht-muted: #a5b5cb;
  --ccht-error: #fda4af;
  --ccht-accent: #7bb8ff;
}
```
