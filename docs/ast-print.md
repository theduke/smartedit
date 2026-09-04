# AST Print

`smartedit ast-print` prints a compact outline of Rust, Python, JavaScript/JSX,
TypeScript/TSX, and Go files.

```bash
smartedit ast-print src/main.rs
smartedit ast-print 'src/**/*.rs'
smartedit ast-print src/main.rs src/lib.rs
```

## Output options

- `--signatures` includes declaration signatures.
- `--type-bodies` includes complete type-like declarations.
- `--function-bodies` includes complete function declarations.
- `--doc` includes owned documentation comments, leading comments, or docstrings.
- `-l`, `--loc` includes source line ranges when they are safe to report.

The outline covers the main declaration forms for each language, including nested and associated
items, Python stubs, JavaScript object callables, TypeScript declaration files, and Go grouped
declarations and receiver methods.

```bash
smartedit ast-print --signatures --doc src/example.py
smartedit ast-print --type-bodies src/example.ts
smartedit ast-print --function-bodies src/example.go
```

## Locations and incomplete files

Locations are zero-based, half-open line ranges compatible with `smartedit apply`. For example,
`[10-13]` covers lines 10 through 12. A `[10-13 shared-line]` range shares a boundary with other
source and must not be passed directly to a line-edit operation.

`ast-print` uses Tree-sitter recovery to produce useful output for incomplete or syntactically
broken files. When a file contains syntax errors, `--loc` is omitted for that file because recovered
node boundaries may not be safe edit targets. Other requested output is still rendered.

## Files and globs

Inputs may be paths or glob patterns:

```bash
smartedit ast-print 'src/**/*.{rs,py,pyi,js,jsx,ts,mts,cts,tsx,go}'
smartedit ast-print --no-ignore 'src/**/*'
```

Glob expansion respects ignore files by default. Unsupported matched files are skipped; the command
fails when no supported files remain.

## Selectors

Use `-s`, `--select` for item paths and `-S`, `--type-select` for a type plus associated items.
Patterns use glob syntax.

```bash
smartedit ast-print -s 'parser.*' --function-bodies src/parser.rs
smartedit ast-print -S AstSelector --signatures --loc src/file_ast.rs
smartedit ast-print -S Box --signatures --doc src/example.go
```

Qualified paths disambiguate duplicate names; bare names retain basename matching. Go receiver
methods use paths such as `Box.Get`. Selectors aggregate across all input files, skipping files
without a match and failing only when nothing matches.

## Notes

- Rust local-item discovery covers declarations directly inside a function body, not declarations
  nested in expression blocks such as `if` or `match`.
- Go grouped specs and multi-name declarations are independently selectable. Shared source spans
  are marked `shared-line`.
- Full-body output preserves source that affects declaration behavior, such as attributes,
  decorators, and Go directives.
