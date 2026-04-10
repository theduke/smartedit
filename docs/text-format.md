# Text Format

`smartedit` supports a compact, line-oriented text format for describing edit programs.

Each non-empty line is either:

- a directive such as `mode incremental` or `apply`
- an operation such as `move`, `remove`, `linemove`, `linedelete`, or `textreplace`
- a comment beginning with `#`

The parser is span-aware, so parsed programs retain source spans for operations and nested values.

## Basics

Rules:

- one directive or operation per line
- blank lines are ignored
- `#` starts a comment and runs to the end of the line
- comments may be full-line or inline

Example:

```text
# Move Rust files.
m src/*.rs extracted/

# Move whole lines from one file into another.
lm notes.txt:0-2 archive.txt:0 # prepend to archive
```

## Execution Model

An `.smedit` file executes in one of two modes:

- `snapshot`
- `incremental`

`snapshot` is the default.

### Snapshot Mode

In snapshot mode, each stage is planned against a single static filesystem snapshot.

Within one snapshot stage:

- path specs are resolved once against that snapshot
- line ranges are read from the original contents of the source file in that snapshot
- destination file contents are also taken from that snapshot
- multiple text edits that affect the same file are merged into one final write
- later operations in the same stage do not see earlier operations' results

Example:

```text
lm a.txt:2-5 b.txt:1
lm a.txt:6-8 c.txt:0
```

Both line ranges come from the same original contents of `a.txt`.

Likewise:

```text
lm a.txt:0-1 out.txt:0
lm a.txt:1-2 out.txt:1
```

Both inserts are planned against the same original contents of `out.txt`, then merged into one final write.

When multiple inserts target the same destination offset in the same snapshot stage, they are applied in modification order.

### Incremental Mode

Enable it with:

```text
mode incremental
```

In incremental mode, each modification is planned and applied logically before the next one is planned.

That means:

- later operations see earlier changes
- path matching is re-evaluated after each operation
- line ranges are interpreted against the current file contents at that point

Example:

```text
mode incremental
lm source.txt:0-1 out.txt:0
lm source.txt:1-2 out.txt:1
```

The second `lm` sees the source file after the first one has already removed its lines.

The `mode` directive must appear before any operations.

`mode snapshot` is also accepted, though it is redundant because snapshot mode is the default.

## `apply`

`apply` creates an explicit stage boundary.

In snapshot mode:

- operations before `apply` are planned against one snapshot
- operations after `apply` are planned against the filesystem state produced by the earlier stage

Example:

```text
lm source.txt:0-1 out.txt:0
apply
lm source.txt:1-2 out.txt:1
```

The second `lm` runs against the updated contents produced by the first stage.

In incremental mode, `apply` is allowed but usually redundant, because each modification already forms its own step.

## Available Operations

Currently supported:

- `move` / `m`
- `remove` / `r`
- `linemove` / `lm`
- `linedelete` / `ld`
- `lineinsert` / `li`
- `linereplace` / `lr`
- `linedeletematch` / `ldm`
- `textreplace` / `tr`

### `move`

Moves files selected by a path spec into a destination directory.

Syntax:

```text
move <source-spec> <destination-dir>
m <source-spec> <destination-dir>
```

Examples:

```text
m a.txt out/
m a/b/ c/
m src/*.rs rust/
m r"src/[a-z_]+\.rs" filtered/
```

Semantics:

- the source spec is resolved during planning
- each matched file is copied into the destination directory
- the source file is then deleted
- relative paths below the match root are preserved
- destination parent directories are created as needed
- planning fails if no files match
- planning fails if a destination file already exists

Example:

```text
m a/b/ c/
```

If `a/b/` contains:

```text
a/b/one.txt
a/b/nested/two.txt
```

the result is:

```text
c/one.txt
c/nested/two.txt
```

### `remove`

Deletes files selected by a path spec.

Syntax:

```text
remove <source-spec>
r <source-spec>
```

Examples:

```text
r old.txt
r generated/
r build/*.tmp
r r"cache/.+\.bin"
```

Semantics:

- the source spec is resolved during planning
- each matched file is deleted
- planning fails if no files match

`remove` currently removes files, not whole directories as directory objects.

### `linemove`

Moves one or more whole-line ranges from a source file into a destination file at a line offset.

Syntax:

```text
linemove <source-file>:<ranges> <destination-file>:<offset>
lm <source-file>:<ranges> <destination-file>:<offset>
```

Examples:

```text
lm a.txt:10-20 b.txt:30
lm a.txt:1-3,5-6 out.txt:0
lm src/lib.rs:0-10,20-30 tmp.txt:5
```

Semantics:

- ranges are line ranges
- each range is half-open: `start-end` means line indices `[start, end)`
- line indices are zero-based
- multiple ranges are concatenated in the order written
- each selected line is moved as a whole, including its trailing newline when present
- the selected lines are removed from the source file
- the concatenated lines are inserted into the destination file before the destination line index
- source and destination may be the same file

Validation rules:

- ranges must be valid
- ranges in one `lm` must be sorted and non-overlapping
- ranges must be within bounds
- insertion offsets must be within bounds
- planning fails if an insertion falls inside a deleted region in the same final snapshot result

### `linedelete`

Deletes one or more whole-line ranges from a file.

Syntax:

```text
linedelete <file>:<ranges>
ld <file>:<ranges>
```

Examples:

```text
ld notes.txt:10-20
ld a.txt:1-3,5-6
ld src/lib.rs:0-10,20-30
```

Semantics:

- ranges are line ranges
- each range is half-open: `start-end` means line indices `[start, end)`
- line indices are zero-based
- each selected line is deleted as a whole, including its trailing newline when present
- multiple ranges are deleted in the order written, though the final effect is based on the stage snapshot

Validation rules:

- ranges must be valid
- ranges in one `ld` must be sorted and non-overlapping
- ranges must be within bounds

### `lineinsert`

Inserts literal content at a line offset in a file.

Syntax:

```text
lineinsert <file>:<offset> <string>
li <file>:<offset> <string>
```

Examples:

```text
li notes.txt:0 "header\n"
li src/lib.rs:2 "use std::fmt;\nuse std::io;\n"
```

Semantics:

- the offset is a zero-based line index
- content is inserted before the destination line
- an offset equal to the line count appends at the end of the file
- content is provided as a quoted string literal

Validation rules:

- insertion offsets must be within bounds

### `linereplace`

Replaces one or more whole-line ranges with literal content.

Syntax:

```text
linereplace <file>:<ranges> <string>
lr <file>:<ranges> <string>
```

Examples:

```text
lr notes.txt:10-12 "replacement\n"
lr src/lib.rs:5-8 "mod parser;\nmod plan;\n"
```

Semantics:

- ranges use the same line-range rules as `linedelete`
- the selected lines are removed
- the replacement content is inserted at the start of the first replaced range
- content is provided as a quoted string literal

Validation rules:

- ranges must be valid
- ranges in one `lr` must be sorted and non-overlapping
- ranges must be within bounds

### `linedeletematch`

Deletes whole lines whose text matches a regex.

Syntax:

```text
linedeletematch <file> <regex>
ldm <file> <regex>
```

Examples:

```text
ldm src/lib.rs r"^use "
ldm Cargo.toml r"^#"
```

Semantics:

- the regex is evaluated independently for each line
- matching is performed on line text without the trailing newline
- matching lines are deleted as whole lines, including their trailing newline when present

### `textreplace`

Replaces text matches within one or more files.

Syntax:

```text
textreplace <source-spec> <match-pattern> <replacement>
tr <source-spec> <match-pattern> <replacement>
```

Examples:

```text
tr src/*.rs "TODO" "DONE"
tr Cargo.toml r"^(name = )\"([^\"]+)\"" "$1\"smartedit\""
tr docs/**/*.md "foo" "bar"
```

Semantics:

- the source spec is resolved during planning
- each matched file is treated as UTF-8 text
- the match pattern is either a quoted string literal for exact text matching or an `r"..."` regex literal
- all non-overlapping matches in each file are replaced
- quoted replacements are inserted literally for string matches
- for regex matches, the replacement string may refer to capture groups such as `$1` or `$name`
- if files match but the text pattern matches nothing inside them, the operation is a no-op for those files

Validation rules:

- planning fails if the source spec matches no files
- literal match patterns must not be empty
- regex patterns must compile successfully
- replacements participate in the same snapshot merge rules as other text operations

## Source Specs

File-oriented operations use a `source-spec`.

Supported forms:

- exact file
- recursive directory-all
- glob
- regex

### Exact File

Examples:

```text
a.txt
path/to/file.rs
```

Matches exactly one file path.

### Directory-All

A path ending in `/` means “all files under this directory”, recursively.

Examples:

```text
src/
a/b/
```

### Glob

Any token containing glob metacharacters such as `*`, `?`, or `[` is parsed as a glob spec.

Examples:

```text
src/*.rs
assets/**/*.png
```

Glob matching is evaluated relative to the non-glob prefix:

- `src/*.rs` becomes root `src`, pattern `*.rs`
- `assets/**/*.png` becomes root `assets`, pattern `**/*.png`

### Regex

Regex specs use Rust-style raw string syntax:

```text
r"src/[a-z_]+\.rs"
```

The planner infers a root directory from the literal prefix before the pattern becomes dynamic, then matches the regex against normalized relative paths beneath that root.

Regex matching uses `/` as the normalized separator.

## Line Range Syntax

Line ranges are comma-separated:

```text
10-20
10-20,30-40,50-60
```

Rules:

- `start-end` means `[start, end)`
- offsets are `usize` line indices
- ranges should be sorted and non-overlapping

## String Literals

`lineinsert`, `linereplace`, and `textreplace` use quoted string literals:

```text
"one line\n"
"first line\nsecond line\n"
```

Supported escapes:

- `\n`
- `\r`
- `\t`
- `\"`
- `\\`

## Comments

A `#` starts a comment that runs to the end of the line.

Examples:

```text
# full line comment
m a/*.rs x/ # inline comment
```

There is currently no plain-token escape syntax for `#`, so it should not appear unescaped in non-regex path tokens.

## Planning And Execution

`smartedit` separates:

1. parsing
2. planning
3. execution

Parsing produces a span-aware AST.

Planning:

- resolves path specs into concrete files
- validates that operations are possible
- produces a batch of concrete filesystem actions

Execution applies that planned batch.

This separation makes dry runs possible and allows validation before writing files.

## Informal Grammar

```text
document        := (blank-line | comment-line | statement-line)*
statement-line  := directive | operation

directive       := mode-directive | apply-directive
mode-directive  := "mode" ("snapshot" | "incremental")
apply-directive := "apply"

operation       := move-op | remove-op | linemove-op | linedelete-op | lineinsert-op | linereplace-op | linedeletematch-op | textreplace-op
move-op         := ("move" | "m") <source-spec> <destination-dir>
remove-op       := ("remove" | "r") <source-spec>
linemove-op     := ("linemove" | "lm") <source-file ":" ranges> <destination-file ":" offset>
linedelete-op   := ("linedelete" | "ld") <file ":" ranges>
lineinsert-op   := ("lineinsert" | "li") <file ":" offset> <string>
linereplace-op  := ("linereplace" | "lr") <file ":" ranges> <string>
linedeletematch-op := ("linedeletematch" | "ldm") <file> <regex>
textreplace-op  := ("textreplace" | "tr") <source-spec> <match-pattern> <string>

ranges          := range ("," range)*
range           := <usize> "-" <usize>
offset          := <usize>
match-pattern   := <string> | <regex>
```

## Examples

### Simple Snapshot Program

```text
# default mode is snapshot
m src/*.rs extracted/
lm notes.txt:0-2 archive.txt:0
ld extracted/old.txt:10-12
li archive.txt:0 "header\n"
tr extracted/*.rs "TODO" "DONE"
```

### Snapshot Program With `apply`

```text
lm source.txt:0-1 tmp.txt:0
apply
lm tmp.txt:0-1 final.txt:0
```

### Incremental Program

```text
mode incremental

lm source.txt:0-1 out.txt:0
lm source.txt:1-2 out.txt:1
```

### Mixed File And Text Operations

```text
m templates/*.txt staging/
apply
lm staging/header.txt:0-1 final.txt:0
lr final.txt:0-1 "/* generated */\n"
ldm staging/header.txt r"^//"
tr final.txt r"generated" "rendered"
r staging/
```

## Current Limitations

Not yet supported in the text format:

- create-file syntax
- create-directory syntax
- overwrite flags
- explicit non-recursive directory selection
- language-aware operations such as moving functions or symbols

These can be added later without changing the overall planning model.
