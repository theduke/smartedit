# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

## [0.0.2](https://github.com/theduke/smartedit/compare/smartedit-v0.0.1...smartedit-v0.0.2) - 2026-09-05

### Added

- Add additional languages
- Add Go (golang) support

### Fixed

- harden editing and language analysis
- Fix some panics for Rust code

### Other

- Fix clippy lints
- Some fixes and more tests
- Update docs
- Add tests
- Add more parsers
- README tweaks
- Improve README
- Add stub changelogs

### Breaking API changes

- `AstLocationRange` uses zero-based, half-open line coordinates and adds
  `is_edit_ready` to distinguish safe whole-line edit ranges.
- `FileAst` adds `first_error`, and the new `AstSyntaxErrorLocation` reports the first
  recovery node using zero-based line and byte-column coordinates.
- `AstItem` adds `inner_docs`, `attributes`, and `source_preamble`; `AstItemKind` adds
  variants for newly represented source constructs.
- The misleading `ExecutionMode` API was removed. `ExecutionOptions` now contains only
  execution controls; snapshot versus incremental evaluation is selected with
  `ProgramMode`.
- `PlannedAction::WriteFile` carries overwrite intent and `PlannedAction::MoveFile` is
  a first-class action.
- `FileSystem` implementations must support exclusive file creation and metadata-aware
  file moves.

These changes require a version bump before publication. Downstream code constructing
public structs or exhaustively matching public enums will need corresponding updates.

### Fixed

- Path identity, snapshot merging, destructive-overlap validation, target-topology
  checks, CRLF matching, and move semantics now preserve documented edit behavior.
- AST parsing and selection cover the audited Rust, Python, JavaScript, TypeScript, TSX,
  and Go constructs while retaining recovery metadata for malformed source.
