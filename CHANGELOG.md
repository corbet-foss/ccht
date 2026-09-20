# Changelog

## 0.2.8 - 2026-09-19

- FIXED: `SessionPool` tracks snapshot event sequence numbers per turn
  instead of one shared per-session counter. Turn-less history/config
  updates (no request id) consumed numbers and gapped every later turn
  event, so live snapshots stayed empty while turns completed. Turn-less
  events are now forwarded without touching the snapshot; per-turn
  counters drop on terminal events. Verified live: both `--agent-probe`
  turns render from snapshots with the agent text.
- ADDED: `SessionPool::session_configuration()` — the live session
  configuration of one key. Snapshots only learn configuration from
  streamed updates; this reports what the agent advertised at session
  creation, so applications can name the session's model immediately.

## 0.2.7 - 2026-09-19

- FIXED: `SessionPool` turns carry unique request ids (`ccht-pool-{key}-{n}`
  from a per-key counter). Reusing one id per key collided with the finished
  turn in conversation snapshots and broke the `Prompt` uniqueness contract,
  freezing per-key snapshots after the first turn.
- FIXED: the pool restarts the per-turn event sequence at 1 after each
  terminal event, so later turns pass the snapshot gap check.
- ADDED: `SessionConfiguration::model_display_name()` — advertised option
  label else raw value id. Pure display helper; selection stays with
  `accepts` and the session.
- DOCS: pool rendering recipe (read buffered per-turn text and status back
  from `conversation()` snapshots instead of hand-buffering chunks) and
  `PoolEvent::Ended` handling (clear the key's busy indicator, tell the
  user; it carries no turn result).

## 0.2.6 - 2026-09-19

- `ChatDock.svelte`: new-conversation voice is product-owned via the
  `newConversationLabel` prop (default `'New conversation'`).

## 0.2.5 - 2026-09-19

- No Rust changes. `StepConfig` steps carry `refreshable`: the Refresh row
  renders only for steps that own refreshable controls.

## 0.2.4 - 2026-09-19

- No Rust changes; the Rust API and behavior are unchanged from 0.2.3.
- ADDED: `web/src/components/ChatDock.svelte` — product-neutral bare chat
  panel (messages, composer, history) over the shared `TurnState`
  vocabulary, with app-owned copy, slots, and callbacks.
- ADDED: `web/src/components/StepConfig.svelte` — generic per-step
  backend/model/options form (content-only; the app wraps it in `Dock`).
- ADDED: `web/src/dock.test.ts` + `web/src/auth.test.ts` (`bun test`) and
  component suites (`vitest run`) covering dock state conformance, auth
  contracts, and rail/chat/config behavior.
- FIXED: focus return prefers the live remounted tab over a stale invoker
  node (`Dock.svelte`).

## 0.2.3 - 2026-09-19

- No code changes from 0.2.2. Repack the npm distribution from the verified
  stage (license notices, corresponding source kit, built Wasm). npm 0.2.2
  was published from the bare working tree, is deprecated, and must not
  be used.

## 0.2.2 - 2026-09-19

- No runtime behavior changes to existing APIs; all existing APIs unchanged.
- ADDED: `src/dock.rs` — headless dock state (`DockKind`, `Placement`,
  `DockId`, `DockManager` with open/placement/focus-token/serialize/restore).
  Portable Rust with no I/O; mirrored by `web/src/dock.ts`.
- ADDED: `src/transport.rs` — `TransportKind` (`Stdio` / `Socket`), `Transport`
  trait, `TransportError`, `StdioTransport`, `SocketTransport` (validation
  only, no I/O).
- ADDED: `src/native/env.rs` — `EnvProfile::{strict, permissive}`, `apply()`,
  `apply_to_command()`.
- ADDED: `src/native/drivers/` — `Challenge` + `validate()`, `AccountInfo`,
  `DriverError` + `code()`, `LoginState`, `LoginDriver` trait
  (`async start` / `poll` / `account` / `cancel`). Native-only, no credential
  extraction or persistence; the app owns storage via `CredentialsProvider`.
  Includes `codex.rs` (`CodexDeviceDriver`: spawns the caller's Codex
  app-server, ChatGPT device-code, host-allowlisted challenge, poll deadline,
  projection-only account read) and `opencode.rs` (`OpenCodeKeyDriver`:
  loopback `opencode serve`, ephemeral port, env-only server password, user
  key PUT, connected check; hand-rolled HTTP, no new dependencies).
- ADDED: `web/src/dock.ts` + `web/src/components/Dock.svelte`
  (with `web/README.md` and `web/package.json` / `web/jsr.json` exports) —
  framework-free dock state (`DockKind`, `Placement`, `validateDockId`,
  `createDockManager` with open/placement/focus-token/serialize/restore) and
  a generic Svelte 5 edge rail (unstyled `ccht-dock-` classes, effects via
  app callbacks, never fetches, spawns, or stores).
- ADDED: `web/src/auth.ts` + `web/src/components/AccountConnection.svelte`
  (with `web/README.md` and `web/package.json` exports) — Svelte 5, unstyled
  `ccht-` classes, effects via app callbacks, never spawns agents.
- ADDED: `src/native/pool.rs` — `SessionPool`: keyed native sessions over one
  shared client (lazy per-key sessions, per-key turn serialization, tagged
  `PoolEvent` stream, per-key conversation snapshots for replay).

### Migration notes

- Private drivers become `LoginDriver` impls (`CodexDeviceDriver`,
  `OpenCodeKeyDriver`); adapt call sites to `start` / `poll` / `account` /
  `cancel` and projection-only `AccountInfo`.
- `AccountConnection.svelte` becomes the shared component; keep product copy,
  flows, and operation state in the app and drive effects through
  `onStart` / `onCancel` / `onKeySubmit` / `onDisconnect`.
- `publish_challenge` URL pinning stays server-side; the browser only renders
  a validated challenge.
- careervector-tui gains: its local `AuthState` mirror can become
  `pub use ccht::auth::AuthState`; `BrowserCredentialsProvider` wraps the
  shared `CredentialsProvider` shape.

## 0.2.1 — 2026-09-13

- No runtime behavior changes.

## 0.2.0 — 2026-09-12

- Expose live session configuration and validated model, select and boolean controls.
- Preserve dependent configuration changes and agent notifications across Rust/Wasm snapshots.
- Apply explicit controls when creating, loading or resuming native sessions.
- Close the native connection after an uncertain configuration timeout.


## Additional 0.1.0 distributions

- Add JSR packaging with the original Rust/Wasm runtime and a losslessly
  recompressed corresponding-source kit.
- Add Python conversation-model bindings through PyO3, with a native wheel,
  source distribution and independent source-replacement checks.
- Record each additional distribution's producing source separately; the
  original Rust/npm packages and core release tag remain unchanged.

## 0.1.0 — 2026-09-11

- Rename the unpublished cllm package to ccht.
- Share the Rust conversation model with browsers through @corbet-labs/ccht Wasm.
- Integrate installed upstream agents through the official ACP SDK: typed sessions,
  streaming, permissions, cancellation, capability checks and bounded delivery.
- Remove direct provider HTTP and credential-handling adapters. Applications own
  any explicit local inference backend and emit the shared conversation contract.
