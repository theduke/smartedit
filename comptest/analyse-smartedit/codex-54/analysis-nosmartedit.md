
• High Level

  This repo is a small Rust workspace with two crates:

  - The root crate smartedit is the CLI binary, defined in Cargo.toml and entered through src/main.rs:1.
  - The lib/ crate smartedit_lib is the reusable API surface, defined in lib/Cargo.toml and re-exported from
    lib/src/lib.rs:1.

  The unusual part is that the library crate does not have its own separate implementation tree. Instead, lib/
  src/lib.rs:1 uses #[path = "../../src/..."] to compile the same source files from the root src/ directory. So
  most real logic lives once in src/, and both the binary and library use it.

  How The Code Is Organized

  - src/main.rs:8 is just Clap wiring. It exposes three subcommands: apply, ast-print, and install-skill.
  - src/cmd/apply.rs:32 is the main execution path: read input, parse the edit DSL, optionally force
    incremental mode, then call Executor::run.
  - src/cmd/ast_print.rs:45 is a separate AST exploration command: collect matching files, parse them with
    tree-sitter, render a structured outline.
  - src/cmd/install_skill.rs:32 is standalone utility code that writes the bundled skill/SKILL.md into a
    target .agents/skills/smartedit directory.

  The core model for edits lives under src/edit/mod.rs:1:

  - src/edit/program.rs:4 defines EditProgram, EditStage, and ProgramMode. A program is a sequence of stages,
    and apply inserts a stage boundary.
  - src/edit/operations.rs:8 defines Modification and GenericModification. This is the main operation enum for
    file moves, deletes, line edits, and text replacement.
  - src/edit/path.rs:6, src/edit/target.rs:7, and src/edit/range.rs:7 define the typed building blocks used by
    operations: path selectors, insertion points, file/range targets, text patterns, and line-range resolution.

  Execution Flow

  The main pipeline is:

  1. Parse text into an EditProgram in src/parser.rs:23.
  2. The parser recognizes directives and operations in src/parser.rs:85, building typed Modification values
     such as MoveFiles, DeleteRanges, InsertLines, and TextReplace.
  3. Plan and execute in src/executor.rs:122. Executor::evaluate turns a program into an src/plan.rs:3
     EvaluationPlan, and run optionally applies it.
  4. Filesystem effects are represented as PlannedAction in src/plan.rs:47, then performed through the
     FileSystem trait in src/fs.rs:7.

  A key design point is in src/executor.rs:127: snapshot mode evaluates a whole stage against one filesystem
  snapshot, while incremental mode re-evaluates after each modification. That matches the behavior described in
  docs/text-format.md.

  AST Side

  The AST functionality is separate from edit execution:

  - src/file_ast.rs:47 defines FileAst and AstItem.
  - src/file_ast.rs:205 parses a file based on extension.
  - src/file_ast.rs:213 currently supports only Rust via tree-sitter-rust.
  - src/file_ast.rs:163 defines selectors for filtering the rendered outline.

  One notable signal from the structure: LanguageModification exists in src/edit/operations.rs:84 but is still
  empty. So today the project is mostly a generic text/file edit engine plus a Rust AST inspection feature,
  with room for future language-specific edit operations.

  If you want, I can also turn this into a “read this repo in order” guide with the 6-8 files to open first.
