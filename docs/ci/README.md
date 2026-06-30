# CI / release workflow

[`release.yml`](./release.yml) cross-compiles `voicetools` for macOS (Apple
silicon + Intel), Linux, and Windows and uploads the binaries to a GitHub
Release on `v*` tag pushes.

It lives here rather than under `.github/workflows/` because the bot that
created this branch doesn't have GitHub's `workflow` OAuth scope. To activate
it, move it into place and push from an account/token that has that scope:

```bash
mkdir -p .github/workflows
git mv docs/ci/release.yml .github/workflows/release.yml
git commit -m "ci: add release workflow"
git push
```

Then cut a release by pushing a tag:

```bash
git tag v0.1.0 && git push origin v0.1.0
```
