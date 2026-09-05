# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

## [0.0.2](https://github.com/theduke/smartedit/compare/smartedit-cli-v0.0.1...smartedit-cli-v0.0.2) - 2026-09-05

### Added

- Add additional languages
- Add Go (golang) support

### Fixed

- harden editing and language analysis

### Other

- Add fixture tests for all languages
- Update docs
- Add more parsers
- README tweaks
- *(build)* Use --force in 'just install' command
- Improve README
- Add stub changelogs

### Changed

- `ast-print --loc` now reports zero-based, half-open whole-line ranges. Ranges whose
  boundary lines contain other source are annotated `shared-line` and must not be
  passed directly to line-edit operations.
- `ast-print` renders recoverable structure from incomplete files, suppresses unsafe locations,
  and aggregates selectors across all input files.
- `apply` execution is explicitly best-effort. Snapshot and incremental modes describe
  evaluation visibility, not rollback guarantees.

### Added

- Broader Rust, Python, JavaScript, TypeScript, TSX, Go, and declaration-file AST coverage,
  including Go grouped declarations, members, receiver methods, package docs, and directives.
- Windows CI coverage, native Windows path parsing, CRLF edit programs, Unicode-aware
  diagnostics, and clean handling of closed stdout pipes.
- Quoted path operands for edit targets containing whitespace, `#`, or semicolons.
- Conflict, alias, symlink, overlap, and file-move safety checks in the edit planner.

### Fixed

- Correctly merge text edits made through lexical, absolute, and symlink aliases.
- Preserve quoted semicolons, CRLF input, file metadata during ordinary moves, and
  comments/docstrings/attributes owned by AST items.
- Reject empty edit programs, incompatible targets, overlapping destructive edits,
  and non-overwrite creations whose destination appears after planning.
