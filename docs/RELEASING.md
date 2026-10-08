# CI and release process

The repository is prepared locally. No GitHub repository, Actions run or published
release is created by this source delivery. Choose your own repository owner/name;
there is no fabricated remote URL or upload credential.

## First push

Create an empty repository under the intended account, then push the prepared git
repository. `gh` is optional; the same can be done through GitHub's web interface.
For example, after choosing the real `OWNER/REPO`:

```sh
git add .
git commit -m "Add ShinyHarbor native server and compatibility tests"
git remote add origin https://github.com/OWNER/REPO.git
git push -u origin main
```

On main pushes, pull requests and manual runs, `ci.yml` resolves one dependency
lockfile for all targets, runs the eight-target build/test/package matrix, and runs
Linux, Windows and macOS browser checks against current CRAN Shiny. Every matrix
job runs executable gateway tests without needing R. Browser jobs separately
exercise real Shiny, runtime callbacks, request-time discovery, static files,
bookmark restoration and draining restarts.
The weekly canary repeats these checks against the current CRAN package. It is a
canary, not a commitment that every future upstream release already works.

The initial source has no generated `Cargo.lock` because dependency resolution
could not be executed during preparation. The workflow generates one when absent
and distributes the exact same lockfile to all matrix jobs. Retrieve the
`resolved-lockfile` artifact, review it, and commit it before treating builds as
reproducible. Builds thereafter use `--locked`. Pin an exact Rust toolchain and
reviewed GitHub Action commit SHAs before a production/stable release.

Rustfmt is applied during CI as a parse/format step; it does not currently enforce
a clean formatting diff. Clippy checks async lock misuse, then unit tests and
integration scripts execute. No matrix failure is suppressed. The published
archive includes the actual lockfile, a conservative all-resolved-crates SBOM,
Cargo metadata and retained license/copyright files. Missing crate license texts
stop packaging for manual review. The SBOM conservatively includes development and
build dependencies and is not a binary-reachability analysis.

## Prerelease

The tag must exactly match `Cargo.toml`, for example:

```sh
git tag v0.1.0-alpha.1
git push origin v0.1.0-alpha.1
```

The release workflow performs policy checks, invokes the full reusable CI, requires
all eight archives, checks each sidecar digest, creates `SHA256SUMS`, and then
publishes a GitHub prerelease. A failed platform, browser job or packaging step
prevents publication. Only the publish job receives `contents: write`; build jobs
have read-only repository permissions. No `pull_request_target` workflow is used.
Forked PRs do not receive a publish token.

Artifact digests detect corruption; there is no signing/notarization/attestation
workflow in this alpha. Generated source archives from GitHub are separate from
binary distributions and may not contain the resolved lockfile if it was not committed.

## Stable release

`tools/release_gate.py` blocks a stable tag while mandatory compatibility work is
unimplemented, certification is unverified, a reviewed Cargo.lock is missing, or
source-bound evidence is absent. Do not bypass it by changing the version or writing
made-up success JSON. A proper stable candidate needs the actual complete test
suite, retained results, review, security testing and platform evidence. Existing
smoke checks are intentionally insufficient.

A repository administrator can modify workflow policy; the gate does not make
malicious maintainers unable to mislabel software. Independent review remains part
of the acceptance process. Required conformance artifacts must be explicitly added
to the tagged source if the gate is to read them on a clean runner.
