# ccht for Python

The same Rust conversation model used by ccht's native and Wasm consumers,
available as `pip install ccht`. Python 3.10 or newer is required.

```python
from ccht import Conversation

chat = Conversation("creator")
changed = chat.apply_event({
    "version": 1,
    "conversation_id": "creator",
    "request_id": "turn-1",
    "sequence": 1,
    "event": {
        "kind": "update",
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "Hello"},
        },
    },
})
assert changed
assert chat.snapshot()["turns"][0]["text"] == "Hello"
```

`Conversation.apply_event()` accepts a JSON string or mapping and returns whether
it changed the model. Replayed events return `False`. A wrong conversation,
sequence gap, invalid envelope, or event after completion raises
`ConversationError` (a `ValueError` subclass) without changing the state.
`snapshot()` returns a detached dictionary; `snapshot_json()` returns its JSON.
Tool calls, permissions, thought and non-text content retain the same wire
contract as the Rust and Wasm packages. Sequence numbers start at one per request.

This package binds the portable model through PyO3. It does not expose the Rust
native process host. Your application connects to an authorized host, delivers
its ccht events, and owns UI, prompts, persistence, transport and authorization.
Use the Rust `ccht::native` API to host installed Codex, Claude or OpenCode ACP
executables. No agent, credentials, provider HTTP client or paid fallback is
included in the Python package.

The first binary wheel targets Linux x86-64 (glibc 2.34 or newer) with CPython's
stable ABI. Other
platforms build the source distribution with a compatible Rust compiler, C
linker, Python development headers and the declared maturin build backend.
Only platforms exercised by release CI are claimed as tested.

## License and source replacement

ccht is LGPL-3.0-only WITH LGPL-3.0-linking-exception. Both LGPL and GPL texts and upstream notices accompany
the distributions. The wheel includes its corresponding source distribution at
`ccht/source/ccht-<version>.tar.gz`; PyPI also serves it separately. It contains
the binding, exact published Rust ccht source, every locked Cargo dependency
and Cargo's vendor configuration. No application source is part of this kit.

To rebuild and replace the extension, extract that archive, make your changes,
then run `python -m pip wheel --no-deps .` and install the resulting wheel into
your application's environment. Cargo can build without registry access using
the supplied vendor directory (`CARGO_NET_OFFLINE=true`). Python build tools
and the compiler are separate prerequisites. CI also rebuilds the extracted
source with an empty Cargo cache and runs the same independent consumer against
the replacement. This proves source replacement, not byte-identical compilation.

Release builds use maturin's ABI audit with the actual Rust target. On Nix, the
release check removes the extension's build-host RPATH with `patchelf` and
repacks it with the standard `wheel` tool before testing. This changes no ABI
tags or bundled source. Build the source directly on your own platform to use
its normal linker configuration.

Keep the notices, corresponding source and users' replacement rights when
distributing applications with ccht. Product licensing and installation must
permit the applicable LGPL replacement/debugging rights; consult the included
license texts for the actual terms.
