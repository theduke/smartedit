# CLI fixture tests

The `cli-tests` crate runs end-to-end `smartedit` commands declared in fixture
files. Tests are discovered recursively from:

```text
fixtures/<language>/
  src/          # source files shared by that language's tests
  ast-print/    # ast-print specifications (`*.test`)
  apply/        # apply specifications (`*.test`)
```

Each test runs from an isolated copy of `fixtures/<language>`, so an `apply`
test cannot modify the checked-in fixture or affect another test.

## Specification format

The first line is a shell-style command prefixed with `>`. The command may
start with `smartedit`, but normally starts directly with the subcommand. The
following lines are the exact expected stdout, terminated by `===`:

```text
> ast-print --loc ./src/main.rs
[0-1] fn main()
===
```

Successful exit status and empty stderr are always required. Newlines are
normalized to LF before comparison.

An apply test can additionally check one or more files after the command. Put
the relative file path after the separator; terminate the final file with a
bare separator:

```text
> apply 'lr src/main.rs:0-1 "fn changed() {}\n"'
=== src/main.rs
fn changed() {}
===
```

Only files listed in the specification are compared. Use `.test` for fixture
specification filenames; other files below `ast-print` and `apply` are ignored.
