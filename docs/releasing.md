# Releasing ccht

Keep `Cargo.toml`, `Cargo.lock`, `web/package.json`, `web/jsr.json`,
`py/pyproject.toml` and `py/Cargo.toml` versions consistent. Published contents
are immutable: use a new version for corrections.

## Why one version ships in two phases

Two packages are built from other packages of the same version:

- The Python binding (`py/`) depends on the `ccht` crate from crates.io, and its
  committed `py/Cargo.lock` pins that crate's registry checksum. It can only be
  prepared once `ccht X.Y.Z` is on crates.io.
- The JSR package is generated from the published npm archive. The committed
  `.ci/jsr-input.json` pins that archive's commit and SHA-256, so it can only be
  written once `@corbet-labs/ccht X.Y.Z` is on npm.

The tag therefore publishes Cargo and npm; a completion run on the same tag adds
JSR and PyPI. Every channel keeps its own producing commit in the bundle, and the
tag stays the release identity for all four.

## Phase 1: tag

1. Update versions and `CHANGELOG.md` (a `## X.Y.Z - date` section becomes the
   release notes). Commit to `main`.
2. Optionally rehearse: `gh workflow run release.yml --ref main` (no tag). It
   runs the tag phase checks and imports and inspects the bundle, retaining it
   as a workflow artifact. It creates no release and publishes nothing.
3. Push the tag `vX.Y.Z` for the version in `Cargo.toml`. `release.yml` runs:
   - `prepare` (read-only, no credentials) calls `ci.yml` at the tag with
     `release-config,rust,core,licenses,rust-package,js-package`.
   - `bundle` (`contents: write`) requires tag == `v` + version, imports the
     crate and npm archive with `.ci/publish.py bundle` (the
     [shared import contract](https://github.com/corbet-libs/ccid/blob/3175c51005006f6050033a55b3da033333ecf84d/adapters/registry-publish.md)),
     inspects it offline, and creates the GitHub release with
     `publication-bundle.tar`, its `.sha256` and the import receipt
     `publication-bundle.json`.
   - `publish` (`contents: write`, `id-token: write`) runs `status` and uploads
     the missing crate through `rust-lang/crates-io-auth-action`. npm has no
     trusted publisher rule yet; the job reports it as deferred.
4. Upload npm from the release bundle (operator upload below, channel `npm`).

## Phase 2: completion

5. Once crates.io and npm serve `X.Y.Z`, commit to `main` (versions unchanged):
   - `py/Cargo.lock` resolved against the published crate (the `python-prepare`
     selector exports the resolved lock without committing it);
   - `.ci/jsr-input.json` naming `@corbet-labs/ccht` `X.Y.Z`, the tag commit as
     `source_commit`, its source archive SHA-256, the npm tarball URL and the
     tarball SHA-256 (all recorded in the release's `publication-bundle.json`
     for the `npm` channel);
   - README/JSR notes that name the released version, if they change.
6. Dispatch `gh workflow run release.yml --ref main -f tag=vX.Y.Z -f complete=true`.
   `prepare` runs `release-config,jsr-package,python-package` at `main`;
   `bundle` re-imports the tag bundle's Cargo and npm channels unchanged, adds
   JSR and PyPI, refuses any change to the existing channels, keeps the tag
   phase bundle as `tag-phase-publication-bundle.*` on the release, and replaces
   `publication-bundle.*` with the extended bundle. `publish` uploads JSR
   through GitHub OIDC (`RELEASE_JSR_AUTH=trusted`) and reports PyPI as deferred.
7. Upload PyPI from the extended bundle (operator upload below, channel `pypi`).
8. Run the published-consumer checks (`jsr-registry`, `python-registry`) and
   state the status per registry. A successful upload response or version
   collision is insufficient proof.

JSR accepts neither whitespace in package paths nor the
`LGPL-3.0-only WITH LGPL-3.0-linking-exception` expression, so `web/jsr.json`
declares plain `LGPL-3.0-only`; the exception text ships in `LICENSES/`.

## Re-running

A transient failure is retried with "Re-run failed jobs"; earlier jobs'
packages are reused. To reconcile or publish an existing release again (for
example after a registry rule is added), dispatch `release.yml` with
`tag=vX.Y.Z` alone: it skips `prepare` and `bundle` and publishes from the
release's existing bundle. A repeated completion starts again from the kept
`tag-phase-publication-bundle.tar`. Uploads are never repeated after an
uncertain outcome; the release journals block them. If `bundle` fails before
the release exists and nothing was published, fix `main`, then delete and
re-push the tag or release a new patch version.

## Operator upload for npm and PyPI

Until npm and PyPI trust `corbet-foss/ccht:release.yml`, upload them from the
release bundle on a workstation. No build runs; the publisher only verifies and
uploads the reviewed bytes. It needs `GH_TOKEN` with `contents: write` on this
repository (for the publication journals), and `NPM_TOKEN` or `PYPI_TOKEN` for
the selected registry:

```sh
tag=vX.Y.Z repo=corbet-foss/ccht rev=3175c51005006f6050033a55b3da033333ecf84d
work=$(mktemp -d) && cd "$work"
git clone -q --depth 1 --branch "$tag" "https://github.com/$repo" source
git clone -q https://github.com/corbet-libs/ccid publisher && git -C publisher checkout -q "$rev"
git -C publisher archive --format=tar HEAD > publisher.tar
gh release download "$tag" -R "$repo" -p publication-bundle.tar -p publication-bundle.tar.sha256
sha256sum --check --strict publication-bundle.tar.sha256
export CCID_REVISION="$rev" CI_TOOL_ARCHIVE="$PWD/publisher.tar" \
  CI_TOOL_SHA256="$(sha256sum publisher.tar | cut -d ' ' -f 1)" \
  RELEASE_BUNDLE="$PWD/publication-bundle.tar" \
  RELEASE_BUNDLE_SHA256="$(cut -d ' ' -f 1 publication-bundle.tar.sha256)" \
  RELEASE_JOURNAL_ROOT="$PWD/journals" GH_TOKEN="$(gh auth token)"
python3 source/.ci/publish.py status
# Phase 1:
NPM_TOKEN=... python3 source/.ci/publish.py publish --channels npm
# Phase 2:
PYPI_TOKEN=... python3 source/.ci/publish.py publish --channels pypi
```

`rev` is the `CCID_REVISION` pinned in the tag's `release.yml`. Keep the
`journals` directory until every registry reports its package as verified.

Crow keeps a manual `release` route for the same bundle if GitHub Actions is
unavailable. Credentials remain outside repository source and check jobs.
Registry account controls are read as prerequisites; these adapters never
change registry policy or create trusted publisher rules.
