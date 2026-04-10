  Top Level

  This repo is a workspace with two crates:

  - The CLI/binary crate at Cargo.toml
  - The reusable library crate at lib/Cargo.toml

  The unusual part is that the library crate does not have its own independent implementation tree. Instead,
  lib/src/lib.rs:1 re-exports modules from the root src directory via #[path = "../../src/..."]. So the root
  src/ contains almost all real code, and lib/ is mostly a public wrapper.

  Binary Layer

  The CLI entrypoint is src/main.rs:1. It is thin:

  - declares cli_support and cmd
  - builds the Clap CLI
  - dispatches to three subcommands: apply, ast-print, and install-skill

  The binary-only code lives here:

  - src/cmd/apply.rs:1: reads edit programs from args/file/stdin, parses them with
  smartedit_lib::parse_edit_program, then runs them through smartedit_lib::Executor
  - src/cmd/ast_print.rs:1: resolves file/glob inputs, parses Rust files into AST structures, and renders them
  - src/cmd/install_skill.rs:1: installs the repo’s bundled SKILL.md
  - src/cli_support.rs:1: formatting/path helpers for CLI output

  So the CLI layer is mostly orchestration and presentation. The real behavior is in the shared library
  modules.

  Core Library Structure

  The core library is split into a small set of responsibilities:

  - src/edit/mod.rs:1: the domain model for edit operations
  - src/parser.rs:1: parses the DSL text into those edit data structures
  - src/executor.rs:1: plans and applies filesystem/text changes
  - src/file_ast.rs:1: Rust AST parsing/rendering for ast-print
  - src/fs.rs:1: filesystem abstraction used by the executor
  - src/plan.rs:1: execution plan/action types
  - src/error.rs:1: central error enum
  - src/span.rs:1: source spans for parse/errors

  The most important flow is:

  1. parser turns the DSL text into an EditProgram
  2. executor turns that program into an EvaluationPlan
  3. executor optionally applies the resulting PlannedActions through FileSystem

  What Each Core Module Owns

  The edit module is the vocabulary of the DSL. It is split internally into:

  - src/edit/operations.rs:1: Modification, GenericModification, LanguageModification
  - src/edit/program.rs:1: EditProgram, stages, and ProgramMode
  - src/edit/path.rs:1: file/path selectors and destinations
  - src/edit/range.rs:1: text line/range modeling and resolution
  - src/edit/target.rs:1: typed targets like file insertions, range selections, and text patterns

  parser.rs is essentially a chumsky grammar for the smartedit language. It parses statements like ld, li, lm,
  tr, mode, and apply, then builds an EditProgram. It is syntax-focused and does not do filesystem work.

  executor.rs is the main engine. It does the heavy lifting:

  - resolves file matches from exact paths, directories, globs, or regexes
  - accumulates text edits per file
  - uses a snapshot model to reason about staged changes before writing
  - produces PlannedActions like CreateDirectory, WriteFile, DeleteFile
  - can run in snapshot or incremental mode

  fs.rs exists so the executor can be tested against an abstract filesystem instead of always touching the OS
  directly. That is why Executor is generic over F: FileSystem.

  file_ast.rs is largely independent from the edit DSL. It uses tree-sitter Rust to parse .rs files into a
  simplified AST representation (FileAst, AstItem, AstSelector) and render outlines/signatures/bodies. That
  powers the ast-print subcommand.

  Architecture Summary

  The cleanest mental model is:

  - CLI crate: user input/output and subcommand wiring
  - Shared library: edit language, parser, executor, AST tooling
  - edit + parser: define the language
  - executor + fs + plan: execute the language safely
  - file_ast: separate Rust-inspection subsystem for ast-print

  One repo-specific quirk matters when navigating it: the source of truth is mostly the root src tree, even for
  the library crate. lib/src/lib.rs just re-exports that code as smartedit_lib.

  If you want, I can follow this with a second pass that explains the execution path of one concrete command,
  like smartedit apply 'ld ...' or smartedit ast-print ..., end to end.
