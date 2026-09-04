# AST Print

Supported languages:

- Rust
- Python
- JavaScript (including JSX)
- TypeScript (including TSX)
- Go

`smartedit ast-print` prints a structured outline of source files. It is meant for quickly understanding a file without reading it top to bottom.

For Rust, the output can include items such as:

- functions
- structs
- enums
- traits
- modules
- `impl` blocks and their methods
- associated types
- public `use` declarations and re-exports
- macro definitions and invocations
- foreign `extern` blocks, functions, and statics
- item declarations nested directly inside functions

Rust local-item discovery is deliberately limited to declarations directly inside a function
body. It does not descend through nested expression blocks such as `if`, `match`, or loops; doing
so would also require a separate policy for expression-level macro invocations and selector scope.

For Python, the output can include items such as:

- classes
- functions and `async` functions
- PEP 695 `type` aliases
- nested methods, classes, functions, and type aliases, including definitions inside compound statements
- module, class, and function docstrings with `--doc`
- Python stub files (`.pyi`)

For JavaScript and TypeScript, the output can include items such as:

- classes, methods, fields, properties, accessors, and function-valued class fields (including
  static, private, readonly, and decorated members)
- functions and methods with syntax-derived `async`, generator, and `static` summaries
- functions assigned to variables such as `const run = () => {}`
- callable object-literal members, including members of nested objects
- anonymous default-exported functions, arrows, and classes under the stable selector `default`
- callable assignments under their qualified paths, such as `module.exports` and `Service.run`
- TypeScript interfaces, enums (including `const enum`), type aliases, function overloads,
  namespaces, legacy modules, ambient modules, and global augmentations
- TypeScript declaration files, including `declare`, `export declare`, `export =`,
  `export as namespace`, and ambient default-binding APIs
- leading file/item comments with `--doc`

For Go, the output can include items such as:

- functions and methods
- structs, interfaces, and type aliases
- const and var declarations
- leading file/item comments with `--doc`

For JavaScript-like files, an initial comment block separated from the first syntax item by a
blank line is file documentation. A comment immediately adjacent to an item belongs only to that
item, including a block comment followed by code on the same line. Comment extraction uses syntax
comment ranges, so code after a block comment is never emitted as documentation.

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

Show edit-ready line ranges:

```bash
smartedit ast-print --loc src/file_ast.rs
smartedit ast-print -l src/file_ast.rs
```

Locations are zero-based, half-open line ranges, like `smartedit apply` ranges. For example,
`[10-13]` owns lines 10, 11, and 12 and can be copied directly into `ld`, `lr`, or `lm`.
`[10-13 shared-line]` means other source also occupies a boundary line, so the line range is
informational and must not be applied directly. First split or reformat the items onto separate
lines, or make a carefully inspected line-level edit; `smartedit apply` does not support byte
ranges.

Rust item locations include owned outer attributes, doc comments, and standalone/intervening
explanatory comments. A comment trailing earlier code on the same line remains with that earlier
code rather than being claimed as the next item's preamble.

Rust documentation includes comment forms such as `///` and `//!` as well as explicit outer and
inner prose attributes such as `#[doc = "..."]` and `#![doc = include_str!(...)]`. Rustdoc control
metadata such as `#[doc(hidden)]` and `#[doc(alias = "...")]` remains an ordinary attribute and is
therefore preserved by signature and full-body modes even when `--doc` is not requested.

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
smartedit ast-print 'src/**/*.{py,pyi,js,jsx,ts,mts,cts,tsx,go}'
smartedit ast-print '**/*'
```

When you pass glob patterns, matched files respect ignore rules from files such as `.gitignore` and `.ignore` by default.

Disable ignore filtering with:

```bash
smartedit ast-print --no-ignore 'src/**/*'
```

If a glob matches files for unsupported languages or formats, those files are silently skipped. The
command fails with `no supported AST files matched` when no supported files remain.

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

Type selectors accept qualified AST paths. `-S nested.Wrapper` distinguishes that type from a
top-level `Wrapper`; `-S Wrapper` retains basename matching for compatibility. Rust impl targets
behind references, lifetimes, `mut`, and generics are associated with their nominal type. Paths
on impl targets are resolved from the impl's module: `self::`, repeated `super::`, `crate::`, and
plain nested relative paths follow Rust path semantics.

Selectors are aggregated across all input files. Files without a match are skipped, and the
command fails only when the selector matches no item in any input file.

Type selectors also work for Python classes and type aliases, including qualified nested paths,
and for TypeScript interfaces/classes/enums/type aliases. Qualified TypeScript namespace paths
such as `-S SDK.Config` disambiguate duplicate local type names; bare names remain supported.

Selectors can be combined with the other formatting flags:

```bash
smartedit ast-print -S S1 --signatures --loc src/file_ast.rs
smartedit ast-print -s 'parser.*' --function-bodies src/parser.rs
```

`--signatures` renders declaration signatures. For Rust it also preserves outer attributes.
`--function-bodies` renders full function items. `--type-bodies` renders full class, interface,
struct, enum, union, type-alias, trait, impl, module, macro-definition, and foreign-block items. A full
body takes precedence over its signature and summarized children. `--doc` is independent and
adds documentation. Python docstrings are not emitted separately when the requested full body
already contains them. Rust inline-module `//!` docs are shown as an indented child beneath the
module declaration in outline/signature modes, which reflects that they document the module from
inside it. A requested full module body already contains those docs, so they are not duplicated.

Each callable in a JavaScript multi-declarator statement is independently selectable and gets a
declaration-qualified signature (for example, `export const second = function ()`). With
`--function-bodies`, each callable renders the complete enclosing declaration. Consequently, a
statement containing multiple callable declarators is repeated when all of them are rendered, but
every body excerpt remains complete JavaScript and preserves its declaration keyword, export
prefix, separators, and terminator.

For TypeScript interfaces and classes, non-method members have stable child selectors: named
properties, fields, accessors, and callable fields use their source name; call, construct, and
index signatures use `call`, `new`, and `index`. Overloads are retained as separate entries and a
selector can return every signature with the same path. String-named ambient modules omit their
quotes in selector paths, for example `-s 'virtual-package.boot'`.
Constructor parameter properties are exposed directly beneath their class, so a declaration such
as `constructor(readonly dependency: Dependency)` is selectable as `ClassName.dependency`.
Declaration-file export bindings use `default`, `export=`, and `export-as-namespace` as stable
selector names; inside an ambient module they are qualified by that module's selector path.

Selected decorated members own their decorator sequence in signature, body, and location output.
Comments between consecutive decorators remain with that declaration, while an adjacent leading
documentation comment is emitted separately by `--doc`.

## Parse Errors

`ast-print` requires a complete parse. If any input contains syntax errors, it exits nonzero,
reports the first known error or missing-syntax location as a zero-based line and byte column, and
does not print partial results. This policy applies to Rust, Python, JavaScript, TypeScript, TSX,
and Go. Fix the syntax error before using the outline as an edit target.

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
smartedit ast-print --signatures 'src/**/*.{py,pyi}'
smartedit ast-print --signatures 'src/**/*.{ts,mts,cts,tsx}'
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
