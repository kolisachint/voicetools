# CI / Release workflows

The GitHub Actions workflows live in `.github/workflows/` (`ci.yml` and
`release.yml`). There is no manual activation step.

## Workflow details

### `ci.yml`

Runs on pushes to `main` and on PRs (installs `libasound2-dev` for the `cpal`
ALSA backend first):
- `cargo fmt --all --check` — formatting
- `cargo clippy --all-targets -- -D warnings` — lints
- `cargo test` — all tests

### `release.yml`

A single workflow triggered when a PR with a `cargo:patch`, `cargo:minor`, or
`cargo:major` label is merged. Runs five jobs:

1. **bump-and-tag** — reads the current version, bumps it based on the label,
   updates `Cargo.toml` + `Cargo.lock`, commits to `main`, pushes, and creates
   an annotated `v*` tag
2. **publish** — publishes `voicetools` to crates.io, skipping a version already
   on the index so a partial run can be retried (needs the `CRATES_IO_TOKEN`
   secret; installs ALSA headers first)
3. **create-release** — creates the GitHub release with auto-generated notes
   (runs in parallel with publish)
4. **build** — builds `voicetools` for four targets (Linux gnu x86_64, macOS
   x86_64 + aarch64, Windows x86_64) and attaches each archive plus a per-asset
   `.sha256`
5. **checksums** — aggregates a combined `SHA256SUMS` manifest for downloaders

## PR-based release flow

The recommended release process uses the `/pr` command (see `.agents/commands/pr.md`):

1. **Agent runs `/pr patch`** (or `minor`/`major`) → Creates PR with `cargo:<bump>` label
2. **PR gets merged** → Triggers `release.yml`
3. **Release workflow** → Bumps version, tags, publishes to crates.io, builds
   cross-platform binaries, and uploads checksums — all in one workflow

This ensures version bumps are reviewable and tied to specific changes.

## Why a single workflow?

Tags pushed by the `GITHUB_TOKEN` do not trigger other workflows (a GitHub
Actions safety measure), so a split bump → tag-push → release design would
require a manual tag re-push every time. Combining version bumping and releasing
into one workflow eliminates that.
