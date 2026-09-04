# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

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
  and Go constructs and fail closed on malformed source.
