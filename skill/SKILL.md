---
name: smartedit
description: Prefer `smartedit ast-print` for Rust, Python, JavaScript, TypeScript, or Go exploration and `smartedit apply` for compact edits when working in this repo for token-efficient inspection/modification. Use to inspect outlines, signatures, doc comments/docstrings, item subtrees, type/function bodies, and to apply inline edit programs instead of rewriting whole files, with smarter more token-efficient editing commands.
---

# Smartedit First

## Default

- Do not spend much time choosing tools.
- If `smartedit` can plausibly do the job, try it first.
- Prefer `smartedit` CLI subcommands over generic reads/writes when they fit.
- Prefer `smartedit ast-print` before reading whole Rust, Python, JavaScript, TypeScript, or Go files.
- Prefer `smartedit apply` with inline args before writing full-file patches.
- Try `smartedit` once, then fall back if it is unsupported, awkward, or less clear.

## `smartedit ast-print`

- Subcommand: `smartedit ast-print [options] <path-or-glob>...`
- Supported languages: Rust, Python, JavaScript (including JSX), TypeScript (including TSX), and Go.
- Use for: outlines, signatures, doc comments/docstrings, type bodies, function bodies, locations, nested item selection, type inspection, multi-file/glob scans.
- Default to `--loc` when the result may drive an edit.
- `--loc` is the most important exploration flag for editing: it prints zero-based, half-open line ranges that use the same coordinates as `smartedit apply`.
- Treat `--loc` output as the fast path from exploration to editing. A plain `[start-end]` span can be copied directly into `ld`, `lr`, or `lm` as `start-end`.
- Never apply a `[start-end shared-line]` span directly: another item or source fragment occupies a boundary line, and line-level edits cannot isolate it. Split/reformat onto separate lines or inspect and construct a safe line edit; `smartedit apply` has no byte-range mode.
- `--doc` prints Rust doc comments, Python docstrings, leading JavaScript/TypeScript comments, and Go package/item comments and directives. For Rust, it also prints root module `//!` docs. For Python, it prints module/class/function docstrings. For Go, comments before the `package` clause are file docs and adjacent comments after it belong to the following declaration.
- Flags: `-l/--loc`, `--signatures`, `--doc`, `--type-bodies`, `--function-bodies`, `-s/--select <item-glob>`, `-S/--type-select <type-glob>`, `--no-ignore`
- Prefer `-s` when you know an item path or subtree. Prefer `-S` when you want a type plus associated `impl` items in Rust, a class plus nested methods in Python/JavaScript, a class/interface plus nested members in TypeScript, or a Go type plus its members and receiver methods.
- Selector paths are AST item paths, not filenames. For a top-level `fn f1()` in `fun.rs`, prefer `-s f1`, not `-s fun.f1`.
- Selectors over multiple files are aggregate queries: files without a match are skipped, and the command fails only if no input file matches.
- `ast-print` rejects syntax-error recovery trees, reports the first known zero-based error line and byte column, and prints no partial outline. Fix malformed input before relying on its structure or locations.
- Rust item locations include owned docs, outer attributes, and standalone/intervening comments, but not a comment trailing earlier same-line code. Rust signatures and full bodies preserve the non-doc parts of that preamble.
- Rust `--doc` recognizes doc comments and prose-bearing outer/inner `doc = ...` attributes; Rustdoc control attributes such as `doc(hidden)` remain in signature/full-body preambles.
- Rust `-S` resolves impl targets using module-relative `self::`, repeated `super::`, `crate::`, and plain nested paths. Bare type names still match by basename.
- With `--doc`, Rust inline-module `//!` docs appear as an indented child in outline/signature output and remain inside a requested full module body without duplication.
- Go methods use receiver-qualified paths such as `Box.Get`; a unique bare method name remains a convenient `-s` selector. Grouped type/const/var specs and multi-name declarations are independently selectable, while shared source spans are marked `shared-line`. Group-level comments render once as context without being claimed by a spec location. Full-body output preserves adjacent compiler directives even without `--doc`.
- Go inputs must be complete source files with exactly one leading `package` clause. File-scope statements and recoverable snippets without the required package shape fail closed.

```bash
# quick outline
smartedit ast-print src/main.rs

# locate a function or item before editing it
smartedit ast-print -s 'resolve_ast_inputs' --loc src/cmd/ast_print.rs

# signatures across multiple Rust files
smartedit ast-print --signatures --loc 'src/**/*.rs'

# quick Python outline
smartedit ast-print src/example.py

# Python class plus nested methods
smartedit ast-print -S Greeter --signatures --doc src/example.py

# quick JavaScript outline
smartedit ast-print src/example.js

# JavaScript class plus nested methods
smartedit ast-print -S Greeter --signatures --doc src/example.js

# quick TypeScript outline
smartedit ast-print src/example.ts

# TypeScript interface/class plus nested members
smartedit ast-print -S Greeter --signatures --doc src/example.ts

# Go type, fields, interface members, and receiver methods
smartedit ast-print -S Box --signatures --doc src/example.go

# signatures plus doc comments for one item subtree
smartedit ast-print -s 'outer.inner.*' --doc --signatures src/file_ast.rs

# nested selection
smartedit ast-print -s 'outer.inner.*' --loc src/file_ast.rs

# print a type definition and associated impl bodies
smartedit ast-print -S AstSelector --type-bodies --loc src/file_ast.rs

# print one function body
smartedit ast-print -s 'resolve_ast_inputs' --function-bodies src/cmd/ast_print.rs

# print one top-level function by name
smartedit ast-print -s 'parse_edit_program' --function-bodies --loc src/parser.rs
```

## `smartedit apply`

- Subcommand: `smartedit apply [options] <inline program...>`
- Default: pass inline operations as shell-quoted args and separate statements with `;`.
- Prefer inline args. Use `-f/--file` or stdin only when the program is long enough that inline quoting is annoying.
- Flags: `--dry-run`, `--incremental`, `-r/--root <path>`, `-f/--file <path>`
- Directives: `mode incremental`, `apply`
- Syntax notes:
  - line ranges are zero-based and half-open: `10-12` means lines `10` and `11`
  - strings are quoted: `"one\n"`
  - regex uses raw strings: `r"^use "`
  - source specs can be exact files, `dir/`, globs, or regex specs

- Operations:
  - `m`, `move <source-spec> <dest-dir>`
  - `r`, `remove <source-spec>`
  - `lm`, `linemove <src:ranges> <dst:offset>`
  - `ld`, `linedelete <file:ranges>`
  - `li`, `lineinsert <file:offset> <string>`
  - `lr`, `linereplace <file:ranges> <string>`
  - `ldm`, `linedeletematch <file> <regex>`
  - `tr`, `textreplace <source-spec> <match> <replacement>`

```bash
# delete lines
smartedit apply 'ld src/lib.rs:0-2'

# insert literal lines
smartedit apply 'li src/lib.rs:2 "use std::fmt;\n"'

# replace a line range
smartedit apply 'lr src/lib.rs:5-7 "mod cli_support;\nmod cmd;\n"'

# delete lines by regex
smartedit apply 'ldm src/lib.rs r"^use "'

# text replacement with a literal match
smartedit apply 'tr README.md "ast-print" "smartedit ast-print"'

# text replacement with regex captures
smartedit apply 'tr Cargo.toml r"^(name = )\"([^\"]+)\"" "$1\"smartedit\""'

# move lines between files
smartedit apply 'lm docs/ast-print.md:0-5 tmp.txt:0'

# move files
smartedit apply 'm src/*.rs extracted/'

# remove files
smartedit apply 'r extracted/*.rs'
```

```bash
# multiple inline statements
smartedit apply 'ld a.txt:1-3; li a.txt:1 "replacement\n"'

# explicit stage boundary
smartedit apply 'lm source.txt:0-1 tmp.txt:0; apply; lm tmp.txt:0-1 final.txt:0'

# incremental execution via inline directive
smartedit apply 'mode incremental; lm source.txt:0-1 out.txt:0; lm source.txt:1-2 out.txt:1'

# incremental execution via flag
smartedit apply --incremental 'lm source.txt:0-1 out.txt:0; lm source.txt:1-2 out.txt:1'

# dry run before writing
smartedit apply --dry-run 'tr src/**/*.rs "TODO" "DONE"'

# alternate root
smartedit apply -r docs 'tr **/*.md "smartedit" "smartedit-cli"'

# file input: only when inline args are too awkward
smartedit apply -f edits.smedit

# stdin input: only when generating the program on the fly
printf '%s\n' 'ld notes.txt:0-1' | smartedit apply
```

## Editing Workflows

- Default loop: locate narrowly with `smartedit ast-print --loc`, then edit narrowly with inline `smartedit apply`.
- Prefer `--dry-run` first for risky multi-file edits.

```bash
# move a function from one file to another
# 1. find it and print its location
smartedit ast-print -s 'extract_tokens' --function-bodies --loc src/old_module.rs

# 2. move the reported line range into the destination file at the desired offset
smartedit apply 'lm src/old_module.rs:<start>-<end> src/new_module.rs:<insert-at>'
```

```bash
# replace one function after locating it
# 1. locate the function body
smartedit ast-print -s 'resolve_ast_inputs' --function-bodies --loc src/cmd/ast_print.rs

# 2. replace only that range
smartedit apply 'lr src/cmd/ast_print.rs:<start>-<end> "fn resolve_ast_inputs(...) {\n    // new body\n}\n"'
```

```bash
# inspect one type and then edit associated code nearby
# 1. find the type and impl locations
smartedit ast-print -S AstSelector --signatures --loc src/file_ast.rs

# 2. make a narrow insertion or replacement near the reported span
smartedit apply 'li src/file_ast.rs:<offset> "\nimpl AstSelector {\n    // ...\n}\n"'
```
