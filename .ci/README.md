# Focused validation and release preparation

Crow runs `.ci/ccid.toml` through the pinned native `ccid` command library.
The repository commands also work on a suitably provisioned build host.
Workstations stage committed source and inspect evidence; they do not compile it.
GitHub Actions has an equivalent manual public-source check route, which the
tag-triggered release workflow reuses as its credential-free preparation job.

## Manual GitHub Actions route

When hosted execution is available, free, and appropriate for the public inputs,
run `.github/workflows/ci.yml` with a comma-separated `checks` selection
(default `rust,core`). It uses the same repository commands on Linux with two
Cargo build/test threads on current `stable` Rust and provisions only the tools
the selection needs (Wasm target, Node, Deno, uv, patchelf, Actionlint). The
resulting artifact contains package receipts plus `hosted-run.json`, with the
source/archive/workflow identity, locked input hashes, actual tools, and final
check outcome. Hosted evidence does not claim execution by ccid or native
Windows/macOS coverage. Publishing credentials never enter this workflow.

Package selectors export under `<ARTIFACT_ROOT>/ccht/<commit>/` (Crow without
`ARTIFACT_ROOT`: `$CARGO_HOME/ccid-artifacts/<commit>/`): the crate at the top,
`web/`, `jsr/` and `python/` below it, each with its receipt, `SOURCE_COMMIT`
and `SHA256SUMS`.

| Selector | Coverage |
| --- | --- |
| `rust` | Formatting, locked tests and Clippy with all features |
| `core` | Default-free tests and the Wasm feature check |
| `licenses` | Cargo dependency license inventory |
| `release-config` | Release adapter contract (tag trigger, phases, job permissions, OIDC routes, publisher pin) and Python/shell syntax; required provisioned Actionlint |
| `rust-package` | Crate archive, license notices and an independent consumer |
| `js-package` | npm Wasm package with corresponding source, offline rebuild and consumer |
| `jsr-package` | JSR package generated from the published npm archive pinned in `jsr-input.json` |
| `python-prepare` | Resolve `py/Cargo.lock` against the published crate |
| `python-package` | Native wheel and sdist with vendored source, offline rebuild and consumers |
| `jsr-registry`, `python-registry` | Consumers of the exact version on the live registry |

## Publishing prepared artifacts

`.ci/publish.py` runs the exact shared publisher resource from the release
workflow's verified ccid archive
([import and publication contract](https://github.com/corbet-libs/ccid/blob/3175c51005006f6050033a55b3da033333ecf84d/adapters/registry-publish.md)).
A pushed `vX.Y.Z` tag runs `.github/workflows/release.yml`; see
[releasing](../docs/releasing.md) for its two phases. The manual Crow `release`
workflow remains the fallback route for the same bundle: supply
`RELEASE_BUNDLE`, `RELEASE_BUNDLE_SHA256` and one `RELEASE_CHANNELS` value with
`RELEASE_OPERATION=publish`. Select `release-config` to check adapter
identities without compiling products or accessing registries.
