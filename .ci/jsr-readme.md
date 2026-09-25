# ccht on JSR

This package shares the exact JavaScript, TypeScript declarations and Wasm
module from `@corbet-labs/ccht` on npm. Its LGPL source kit and notices are
included; the source archive uses XZ compression to fit JSR's package limit.

Save this as `example.ts`:

```ts
import { createConversation } from "jsr:@corbet-labs/ccht@0.2.10";

const wasm = new URL("https://jsr.io/@corbet-labs/ccht/0.2.10/wasm/ccht_bg.wasm");
const conversation = await createConversation("workspace/creator", { wasm });
console.log(conversation.snapshot());
conversation.free();
```

Run `deno run --allow-net=jsr.io example.ts`. Version 0.2.10 requires an explicit
Wasm URL or bytes for the first conversation; subsequent conversations share
the initialized module. JSR exports the JavaScript entrypoint plus `./auth`
and `./dock`; the explicit Wasm subpath in the npm/Vite example below and the
Svelte components belong to the npm package, because JSR exports only
JavaScript and TypeScript modules.

This is the shared conversation model. Native agent execution and login stay
in the application's Rust host; the JavaScript package does not run agents.

---
