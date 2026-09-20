# Agent instructions

Write all code comments and documentation in English.

## Product boundary

- ccht is a reusable conversation library for web, native/TUI and other
  consumers. Share the Rust conversation model with Wasm; keep process
  execution native. ccht owns login MECHANISMS as native optional modules
  (`native::env`, `native::drivers`) and GENERIC UI contracts/components
  (`auth.ts`, `AccountConnection.svelte`); apps own product flows, product
  UI, storage, prompts, and authority.
- Use the official ACP SDK and maintained agent executables. Reuse their
  session and process primitives instead of implementing an agent loop.
- Apps own login flows, credential storage, product UI, and authority. ccht
  drivers run vendor handshakes natively only with projection-only state:
  no provider credential extraction, persistence, or read-back. No direct
  paid model APIs, automatic runtime installation or billable fallback.
  Expose unsupported capabilities and authentication failures explicitly.
- This crate is LGPL-3.0-only WITH LGPL-3.0-linking-exception: combined works
  may link statically or dynamically without relinking duties; library
  modifications stay LGPL. Do not add implementation code available
  only under the full GPL or AGPL. Permissive references (MIT,
  Apache-2.0, OpenCode MIT) may inform implementations with attribution
  in the module docs; preserve their grants and notices.
- Secrets never appear in logs, errors, test fixtures, or chat payloads.
- Permission prompts do not replace an execution boundary. Application
  authorization and supported runtime confinement control actual actions.

## Quality boundary

- `cargo fmt --check`, `cargo clippy --all-targets` (no warnings),
  `cargo test` — all green before every commit.
- Validate protocol/lifecycle behavior using fixtures; no live inference in CI.
- Native and Wasm package consumers must exercise the shared event model.
