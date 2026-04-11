# AST Print

Supported languages:

- Rust
- Python
- JavaScript (including JSX)
- TypeScript (including TSX)

`smartedit ast-print` prints a structured outline of source files. It is meant for quickly understanding a file without reading it top to bottom.

For Rust, the output can include items such as:

- functions
- structs
- enums
- traits
- modules
- `impl` blocks and their methods

For Python, the output can include items such as:

- classes
- functions and `async` functions
- nested methods, classes, and functions
- module, class, and function docstrings with `--doc`

For JavaScript and TypeScript, the output can include items such as:

- classes and methods
- functions, async functions, and generator functions
- functions assigned to variables such as `const run = () => {}`
- TypeScript interfaces, enums, and type aliases
- leading file/item comments with `--doc`

You can use it to:

- get a high-level overview of a file
- inspect function signatures without full bodies
- include full type or function bodies when needed
- focus on a subset of items with selectors
- print locations to jump to the relevant lines quickly

## Basic Usage

Print a simple outline:

```bash
smartedit ast-print src/main.rs
```

Include function and type signatures:

```bash
smartedit ast-print --signatures src/main.rs
```

Include full type bodies:

```bash
smartedit ast-print --type-bodies src/file_ast.rs
```

Include full function bodies:

```bash
smartedit ast-print --function-bodies src/file_ast.rs
```

Include both:

```bash
smartedit ast-print --type-bodies --function-bodies src/file_ast.rs
```

Show line ranges:

```bash
smartedit ast-print --loc src/file_ast.rs
smartedit ast-print -l src/file_ast.rs
```

Show doc comments or docstrings:

```bash
smartedit ast-print --doc src/example.py
smartedit ast-print --doc src/example.ts
```

## Multiple Files And Globs

`ast-print` accepts file paths and glob patterns.

Examples:

```bash
smartedit ast-print src/main.rs src/lib.rs
smartedit ast-print 'src/**/*.rs'
smartedit ast-print 'src/**/*.{py,js,jsx,ts,tsx}'
smartedit ast-print '**/*'
```

When you pass glob patterns, matched files respect ignore rules from files such as `.gitignore` and `.ignore` by default.

Disable ignore filtering with:

```bash
smartedit ast-print --no-ignore 'src/**/*'
```

If a glob matches files for unsupported languages or formats, they are reported as ignored and skipped.

## Selectors

Use selectors to print only part of a file.

Item selectors with `-s` or `--select` match item paths using glob patterns.

Example: print everything inside an inline module `xyz`:

```bash
smartedit ast-print -s 'xyz.*' src/file_ast.rs
```

Type selectors with `-S` or `--type-select` match a type and its associated items, such as `impl` methods.

Example: print the definition of `S1` and methods associated with it:

```bash
smartedit ast-print -S S1 src/file_ast.rs
```

Type selectors also work for Python classes and TypeScript interfaces/classes.

Selectors can be combined with the other formatting flags:

```bash
smartedit ast-print -S S1 --signatures --loc src/file_ast.rs
smartedit ast-print -s 'parser.*' --function-bodies src/parser.rs
```

## Common Workflows

Quick overview of a Rust file:

```bash
smartedit ast-print src/lib.rs
```

Quick overview of a Python or JavaScript file:

```bash
smartedit ast-print src/example.py
smartedit ast-print src/example.js
```

Review public APIs and signatures across a directory:

```bash
smartedit ast-print --signatures 'src/**/*.rs'
smartedit ast-print --signatures 'src/**/*.{ts,tsx}'
```

Inspect one type and its methods with line locations:

```bash
smartedit ast-print -S AstSelector --signatures --loc src/file_ast.rs
```

Inspect the full implementation of a specific module subtree:

```bash
smartedit ast-print -s 'cmd.ast_print.*' --function-bodies src/main.rs src/cmd/ast_print.rs
```

Inspect one JavaScript or TypeScript type with nested methods:

```bash
smartedit ast-print -S Greeter --signatures --doc src/example.ts
smartedit ast-print -S Greeter --signatures src/example.js
```
