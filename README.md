# ccht

Reusable conversations for Rust applications, terminals and web frontends.
**ccht** keeps the conversation model in Rust and compiles
that same model to Wasm. Native hosts connect to installed agents using the
[official ACP Rust SDK](https://github.com/agentclientprotocol/rust-sdk).

```text
Web UI ── @corbet-labs/ccht (Wasm) ── application transport/storage
                                              │
TUI / native UI ── ccht::Conversation ── ccht::native ── upstream ACP agent
```

The library owns structured conversation events, ordered replay, native sessions,
streaming, cancellation, permission requests, native login MECHANISMS
(`native::env`, `native::drivers`), and generic web auth contracts/components
(`auth.ts`, `AccountConnection.svelte`). Your application owns product flows,
product UI, instructions, context, tools, user authorization, storage and
HTTP/WebSocket/SSE transport. No Corbet-specific prompts or database are required.

Agent reasoning, tools, and protocol translation remain upstream.
ccht does not implement an agent loop, persist or read back provider credentials,
install agents, call paid model HTTP APIs or select an API-key fallback. Login
drivers project vendor state only; storage, prompts, and authority stay in the app.

## Layers

Core (everywhere, including Wasm): conversation model, `WireEvent` contract,
auth state/store shapes (`src/auth.rs`), and transport validation
(`src/transport.rs`). No I/O, spawning, or storage.

Native effects (optional `native` feature, never Wasm): ACP sessions
(`src/native/`), environment allowlist (`src/native/env.rs` with
`EnvProfile::strict` / `permissive`), and vendor login drivers
(`src/native/drivers/` with `LoginDriver`, `Challenge` + `validate()`,
`AccountInfo`, `DriverError` + `code()`, `LoginState`, `CodexDeviceDriver`,
`OpenCodeKeyDriver`). Drivers run handshakes only and report projection-only
state; the app owns storage via `CredentialsProvider`.

Web bindings + components (`web/`, see `web/README.md`): Wasm conversation
model plus `web/src/auth.ts` mirrors and
`web/src/components/AccountConnection.svelte` (Svelte 5, unstyled `ccht-`
classes, effects via app callbacks, never spawns agents).

Feature flags: `native` is the default feature (ACP sessions, env, drivers;
requires Tokio; native-only). `web` enables Wasm bindings. Use
`default-features = false` for the portable model alone.

Transport: one ACP-style JSON-RPC protocol over two validated endpoints
(`src/transport.rs`): `StdioTransport` (explicit native program + args, no
shell) today, and `SocketTransport` (`ws` / `wss` / `http` / `https` with a
host, shape-only check) for a later Wasm socket bridge. `TransportKind`
(`Stdio` / `Socket`), the `Transport` trait, and `TransportError` describe
and validate endpoints only; they never connect, spawn, or perform I/O, and
errors use fixed text so secrets cannot leak. The app owns actual delivery
and storage.

## Acceptable use — single-operator local use only

ccht is for single-operator local use only. No pooling, sharing, resale, or
multi-tenant hosting of subscriptions. ccht grants no rights to external
runtimes or subscriptions; their own terms apply (see [THIRD-PARTY.md](THIRD-PARTY.md)).

- No credential persistence or read-back in the library. Drivers report
  projection-only state (`AccountInfo`, `AuthState`); the app owns storage
  via `CredentialsProvider`.
- The caller supplies binary paths, homes, and clientInfo; ccht never
  hard-codes product strings.
- Respect upstream gates: device-code settings, forced methods, and usage limits.
- OpenCode use is one operator key to one loopback `opencode serve`; the
  server password is ephemeral and env-only. Use the API-key path for
  CI/programmatic use.

Module details: `src/native/drivers/mod.rs`, `src/native/drivers/codex.rs`,
`src/native/drivers/opencode.rs`, `src/native/env.rs`, `src/transport.rs`,
`web/src/auth.ts`, `web/README.md`.

## Rust

```toml
[dependencies]
ccht = "0.2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

`native` is the default feature. For the portable model alone:

```toml
ccht = { version = "0.2", default-features = false }
```

```rust
use ccht::{Conversation, Event, WireEvent, acp};

let mut conversation = Conversation::new("workspace/creator");
conversation.apply(WireEvent::new(
    "workspace/creator", "turn-1", 1,
    Event::Update {
        update: acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Hello".into()),
        ),
    },
)).unwrap();
assert_eq!(conversation.turns()[0].text, "Hello");
```

[examples/native.rs](examples/native.rs) shows streaming from an explicitly
supplied installed ACP executable. It requires native agent authentication already
configured by the operator. Use `codex-acp`, `claude-agent-acp`, or `opencode acp`
as separate, reviewed runtime installations. Generic ACP executables also work.
No agent/runtime is bundled in the crate or browser package.

`NativeClient::connect` initializes the protocol. Create a session with its working
directory, explicit model (if desired) and application MCP servers. Take the event
receiver before prompting and consume it concurrently with the prompt future.
A session accepts one active prompt; another returns `Busy`. Different sessions
have separate streams and permission identities. History load/resume and model
selection require the agent's advertised capabilities; unsupported requests fail.

`session.handle().configuration()` exposes the agent's ordered model, reasoning,
mode and boolean controls. Call `set_model(id)` or `set_config_option(id, value)`
to change them. The full response replaces the advertised configuration, including
dependent options; configuration notifications update both the native handle and
the portable conversation snapshot. Unknown values fail without a fallback.
`SessionOptions::configuration` applies explicit settings when creating or restoring
a session. Retain the agent session ID and its working directory in your product
storage to resume completed conversations across process restarts. Never replay an
uncertain delivery automatically.

Permissions default to denial. Applications that opt into asking must render the
request, check their authority and respond with one of the advertised choices.
A permission callback is not a sandbox: agent-owned tools may have separate runtime
policy. Configure filesystem/network confinement in the host for your use case.
ccht advertises no client filesystem or terminal capabilities.

Cancellation asks the upstream agent to stop. If it does not stop within the
configured shutdown timeout, ccht closes the SDK connection, affecting **all**
sessions on that connection. Dropping an active prompt also closes its connection.
Use one connection per independent workload when cancellation must be isolated.
Bounded event channels fail explicitly on an unresponsive consumer.

`AgentCommand::with_working_directory` sets the actual agent startup directory
using installed Linux/macOS `env` launchers. Windows currently reports this helper
as unsupported because the SDK launch configuration lacks a startup-directory
field. Session cwd alone does not constrain files read during agent startup.
Linux is covered by process-level tests; macOS support is source-reviewed.

ACP enables `serde_json`'s `preserve_order` feature through Cargo feature
unification. Applications that hash or sign JSON must explicitly canonicalize
object keys; depending on a `Value` map's incidental iteration order is unsafe.

## Browser

```sh
npm install @corbet-labs/ccht
```

```js
import { createConversation } from '@corbet-labs/ccht';
// Vite example: emit the Wasm as a separately replaceable asset.
import wasmUrl from '@corbet-labs/ccht/ccht_bg.wasm?url';

const conversation = await createConversation('workspace/creator', { wasm: wasmUrl });
// After your authenticated application transport delivers a ccht WireEvent:
const snapshot = conversation.applyEvent(event);
render(snapshot.turns);
// When the view is destroyed:
conversation.free();
```

Version 0.2.0 requires an explicit `wasm` URL or bytes on first initialization.
A browser cannot launch native processes: its
application server or desktop host runs `ccht::native`. The package makes no
network connection apart from loading its Wasm module. The host decides which
backend is available and how users authenticate to the application.

## JSR

The JSR package exposes the same Rust/Wasm model:

```ts
import { createConversation } from 'jsr:@corbet-labs/ccht@0.2.0';

const conversation = await createConversation('workspace/creator', {
  wasm: new URL('https://jsr.io/@corbet-labs/ccht/0.2.0/wasm/ccht_bg.wasm'),
});
```

In Deno, allow access with `--allow-net=jsr.io` for this Wasm download.
Applications can also supply their own Wasm bytes or URL. JSR's corresponding
source archive uses `dependencies.tar.xz` to fit its package size limit; the
source files and runtime bytes are the same as the npm distribution.

## Python

Install from [PyPI](https://pypi.org/project/ccht/0.2.0/):

```sh
python -m pip install ccht==0.2.0
```

```python
from ccht import Conversation

conversation = Conversation('workspace/creator')
# Deliver an authorized WireEvent from your application transport.
conversation.apply_event(event)
snapshot = conversation.snapshot()
```

The Python package wraps the published Rust conversation model through PyO3.
It exposes event reduction and snapshots; native agent execution uses a Rust
host with `ccht::native`. See [py/README.md](py/README.md) for the Python API,
binary wheel requirements and source rebuild/replacement instructions.

## Event and persistence contract

`WireEvent` version 1 contains `conversation_id`, `request_id`, a per-request
`sequence` starting at 1, and `event`. Retain the original structured ACP updates:
text, non-text content, tool calls, thought chunks, plans, permissions and errors
are distinct. The model returns a serializable snapshot for your renderer.

Assign identities and sequences on a trusted application host. Never let a browser
select another user's conversation without server-side authorization. Sequence
numbers are **not** SSE cursors or database row IDs. The app stores its own event
log and replays each request in order. Already-applied sequences are ignored;
gaps, a different conversation, malformed events and new events after completion
fail without mutating state. Delivery identity must not be reused for another turn.

A model holds at most 256 turns, 8 MiB per turn and 1 MiB per event. Applications
should page long histories into separate models and limit streamed inputs before
parsing them. Durable transcript storage and reconnection are application concerns;
a serialized snapshot is a render result, not a native-agent session checkpoint.

Namer is the first integration: Creator/Critic have independent conversation
scopes, durable application events, authenticated SSE and cancellation. Its queue
uses a fresh native session per turn with bounded role history supplied by Namer.
CareerVector's TUI reuses the native session API while keeping CV tools, prompts
and revision checks in its own code. Explicit local inference can emit the same
`WireEvent` contract through an application-owned backend.

## Building the Wasm package

Use Rust with `wasm32-unknown-unknown` installed. The checked-in generator uses the
same exact `wasm-bindgen` version as the Rust crate; no global Wasm CLI is required.

```sh
cargo build --locked --release --no-default-features --features web --target wasm32-unknown-unknown
cargo run --locked --release -p ccht-wasm-bundle -- target/wasm32-unknown-unknown/release/ccht.wasm web/wasm
```

Published npm browser archives include the corresponding Rust source and generator
under `source/`, original notices, and the full locked dependency sources in
`source/dependencies.tar.gz`. Cargo's vendor checksums are preserved inside that
archive. To rebuild from the distributed package, extract it inside `source/`:

```sh
cd source
tar -xzf dependencies.tar.gz
cargo build --locked --offline --release --no-default-features --features web --target wasm32-unknown-unknown
cargo run --locked --offline --release -p ccht-wasm-bundle -- target/wasm32-unknown-unknown/release/ccht.wasm ../wasm
```

The supplied `.cargo/config.toml` uses only those vendored sources. The installed
Rust toolchain must include the Wasm target and linker. CI rebuilds this kit with
an empty Cargo cache and tests the replacement module through the same npm API.
Keep that exported interface compatible when replacing the library.

CI verifies the portable Rust model, actual Wasm compilation and packaged Node
consumer, native protocol/process fixtures, dependency notices and an independent
Rust crate consumer. Native fixtures do not call a model or access an account.
Live runtime support must additionally be checked against the installed agent,
its advertised capabilities, login state and execution policy.

## License

ccht is **LGPL-3.0-only WITH LGPL-3.0-linking-exception**. See [LICENSE.md](LICENSE.md) and
[THIRD-PARTY.md](THIRD-PARTY.md). Applications may use different licenses while
preserving the LGPL library rights. Combined works may link statically or
dynamically without a relinking route; library modifications stay LGPL.
Provide the library's corresponding source and notices. Review the complete
license for your distribution method. Contributions are accepted under the
[Contributor License Agreement](CLA.md).

ACP and an account's subscription do not guarantee a billing mode. The operator
must configure the native agent/account and disable unwanted paid usage. Known
ambient API-key variables are cleared at launch and explicit API-key overrides
are rejected, but ccht cannot attest saved agent credentials or account billing
settings. Native runtimes and hosted services retain their own terms; a library
license does not authorize pooling subscriptions or product-specific login flows.
