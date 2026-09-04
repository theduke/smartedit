# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

### Changed

- `ast-print --loc` now reports zero-based, half-open whole-line ranges. Ranges whose
  boundary lines contain other source are annotated `shared-line` and must not be
  passed directly to line-edit operations.
- `ast-print` rejects syntax-error recovery trees instead of emitting partial output,
  and selectors now aggregate matches across all input files.
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
