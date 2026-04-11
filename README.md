# smartedit

`smartedit` is a small CLI for language-aware code exploration and compact source edits.

It is built primarily for AI-agent workflows: inspect code structure without reading full files, identify narrow edit targets, and apply small deterministic changes with less token usage than whole-file reads and rewrites. It is also useful for humans who want scriptable, narrow edits.

`smartedit` currently centers on three capabilities:

- `smartedit ast-print` for fast structural exploration of Rust, Python, JavaScript, and TypeScript
- `smartedit apply` for compact line-, text-, and file-oriented edits
- `smartedit install-skill` for installing the bundled agent skill

For command syntax and detailed manual usage, see `docs/ast-print.md`, `docs/text-format.md`, `skill/SKILL.md`, and `smartedit --help`.

## Why Use It

Use `smartedit` when an agent or operator should:

- inspect file structure before reading full source
- locate exact edit spans with `--loc`
- scan signatures or docs across many files
- apply narrow edits instead of rewriting entire files
- bias an agent toward token-efficient exploration and editing in a repo

## Install The CLI

### Cargo

Install from a checkout:

```bash
cargo install --path .
```

Install directly from Git:

```bash
cargo install --git https://github.com/theduke/smartedit --locked smartedit-cli
```

### Nix Flake

Install the default package from the flake:

```bash
nix profile install github:theduke/smartedit
```

Run it without installing:

```bash
nix run github:theduke/smartedit
```

## Install The Skill

The repo ships a bundled `smartedit` skill for agent environments that support `.agents/skills`.

Install it for the current repository:

```bash
smartedit install-skill --repo
```

Install it into a specific directory:

```bash
smartedit install-skill --dir path/to/project
```

Install it for your user home:

```bash
smartedit install-skill --user
```

This writes `SKILL.md` to `.agents/skills/smartedit` under the chosen root.

## Agent-Driven Usage

The intended loop is:

1. Use `smartedit ast-print` to get structure, signatures, docs, or exact locations.
2. Decide on a small target span instead of editing a whole file.
3. Use `smartedit apply` with an inline edit program or `.smedit` file.

Typical agent scenarios:

- A coding agent needs the outline of a Rust or TypeScript file before deciding what to read next.
- An agent finds a function with `--loc`, then replaces only that function body.
- An agent scans signatures across a glob to understand a subsystem without paying to read every file in full.
- A repo installs the bundled skill so agents default to `smartedit` first and fall back only when needed.

## Develop

### Release Automation

GitHub Actions uses `release-plz` via `.github/workflows/release-plz.yml`.
Published CLI releases then trigger `.github/workflows/release-binaries.yml` to build and attach `smartedit` binaries for Linux, Windows, and macOS.

- On pushes to `main`, it opens or updates a release PR with version/changelog changes.
- If `CARGO_REGISTRY_TOKEN` is configured in repository secrets, it also runs publish/release.
- The binary upload workflow listens for published releases, so `RELEASE_PLZ_TOKEN` must remain a PAT instead of the default `GITHUB_TOKEN`.
