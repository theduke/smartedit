# smartedit

smartedit is a tool for enabling simple language-aware exploration and modifications of source code.

The main goal is to support token-efficient and language-aware modifications of files for AI agents to reduce token usage and cost.

It is primarily used as a CLI, with either inline instructions or .smedit files.

## Develop

### Release Automation

GitHub Actions uses `release-plz` via `.github/workflows/release-plz.yml`.
Published CLI releases then trigger `.github/workflows/release-binaries.yml` to build and attach `smartedit` binaries for Linux, Windows, and macOS.

- On pushes to `main`, it opens or updates a release PR with version/changelog changes.
- If `CARGO_REGISTRY_TOKEN` is configured in repository secrets, it also runs publish/release.
- The binary upload workflow listens for published releases, so `RELEASE_PLZ_TOKEN` must remain a PAT instead of the default `GITHUB_TOKEN`.
