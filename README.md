# smartedit

**Give coding agents a map of the codebase—not a dump of every file.**

`smartedit` turns Rust, Python, JavaScript, TypeScript, TSX, and Go source into compact,
language-aware outlines. Agents can find the relevant type or function before reading its body,
keeping exploration fast, focused, and token-efficient.

```bash
smartedit ast-print --signatures --loc 'src/**/*.rs'
```

Alongside structural printing, `smartedit` provides compact deterministic edits with
`smartedit apply` and a bundled agent skill that teaches this inspect-then-edit workflow.

## See It in Action

The Rust examples below run against this repository. In the output, `>` shows nesting and
`--loc` adds zero-based, half-open line ranges.

<details>
<summary><strong>Rust: map the CLI entry point</strong></summary>

```console
$ smartedit ast-print src/main.rs
mod cli_support
mod cmd
use clap::{Parser, Subcommand};
use crate::cmd::{apply::CmdApply, ast_print::CmdAstPrint, install_skill::CmdInstallSkill};
struct Cli
enum Command
impl Command
> fn run
fn main
fn run
mod tests
> use clap::{CommandFactory, Parser, error::ErrorKind};
> use super::Cli;
> fn top_level_help_describes_each_command
> fn install_skill_help_explains_targets_layout_and_examples
> fn ast_print_help_warns_about_shared_line_ranges
```

</details>

<details>
<summary><strong>Rust: select one type and its implementation</strong></summary>

```console
$ smartedit ast-print -S AstSelector --signatures --loc lib/src/file_ast.rs
#[derive(Debug, Clone, PartialEq, Eq, Default)]
[242-247] pub struct AstSelector
[248-293] impl AstSelector
> [249-252] pub fn is_empty(&self) -> bool
> [253-267] pub fn display(&self) -> String
> [268-292] fn compile(&self) -> Result<CompiledAstSelector>
```

The selector removes everything unrelated to `AstSelector`; `--signatures` keeps bodies out of
the output, while `--loc` identifies the exact source spans to inspect next.

</details>

<details>
<summary><strong>JavaScript: print signatures and documentation</strong></summary>

Given `example.js`:

```javascript
/** module docs */

/** class docs */
class Greeter {
    /** method docs */
    greet(name) {
        function normalize(value) {
            return value.trim();
        }

        return normalize(name);
    }
}

export const run = async (task) => {
    return task();
};
```

The structural view is:

```console
$ smartedit ast-print --signatures --doc example.js
/** module docs */
/** class docs */
class Greeter
> /** method docs */
> greet(name)
>> function normalize(value)
export const run = async (task) =>
```

</details>

## Install

### Release binary

Download a binary for Linux, macOS, or Windows from the
[latest GitHub release](https://github.com/theduke/smartedit/releases/latest).

### Cargo

Install directly from Git:

```bash
cargo install --git https://github.com/theduke/smartedit --locked smartedit-cli
```

Or install from a checkout:

```bash
cargo install --path .
```

### Nix

Install the default flake package:

```bash
nix profile install github:theduke/smartedit
```

Or run it without installing:

```bash
nix run github:theduke/smartedit
```

## Print Source Structure

Start broad, then narrow the output only when needed:

```bash
# Outline one file.
smartedit ast-print src/main.rs

# Include signatures and source locations.
smartedit ast-print --signatures --loc src/main.rs

# Scan a repository while respecting ignore files.
smartedit ast-print --signatures 'src/**/*.{rs,ts,tsx}'

# Select an item subtree or a type and its associated implementation.
smartedit ast-print -s 'parser.*' --function-bodies src/parser.rs
smartedit ast-print -S AstSelector --signatures --loc lib/src/file_ast.rs

# Include Rust docs, Python docstrings, or leading JS/TS/Go comments.
smartedit ast-print --doc src/example.js
```

Supported output controls include:

- `--signatures` for declarations without full bodies
- `--doc` for documentation comments, leading comments, and docstrings
- `--loc` for whole-line source ranges; unsafe shared boundaries are marked `shared-line`
- `--type-bodies` and `--function-bodies` when the implementation is actually needed
- `-s` / `--select` for item-path globs and `-S` / `--type-select` for a type plus associated items

Globs respect `.gitignore` and `.ignore` by default. Pass `--no-ignore` to include ignored files.

## Install the Agent Skill

Install the bundled skill for your user account:

```bash
smartedit install-skill --user
```

Or install it for the current repository or a specific directory:

```bash
smartedit install-skill --repo
smartedit install-skill --dir path/to/project
```

Repository installation writes the skill to `.agents/skills/smartedit/SKILL.md`. It guides agents
to print structure first, target the smallest useful span, and then use `smartedit apply` for a
compact change when appropriate.

## Compact Edits

`smartedit apply` supports line insertion, deletion, replacement and movement; literal or regex
text replacement; and file movement or removal. Edit programs can be passed inline or stored in
`.smedit` files.

```bash
smartedit apply 'lr src/lib.rs:5-7 "mod cli_support;\nmod cmd;\n"'
smartedit apply --dry-run 'tr src/**/*.rs "TODO" "DONE"'
```

See the [edit format reference](docs/text-format.md) for operation syntax, snapshot versus
incremental execution, and safety behavior.

## Documentation

- [AST printing](docs/ast-print.md)
- [Edit program format](docs/text-format.md)
- [Bundled agent skill](skill/SKILL.md)
- `smartedit --help` and `smartedit <COMMAND> --help`

## Release Automation

GitHub Actions uses `release-plz` via `.github/workflows/release-plz.yml`. Pushes to `main` open
or update a release PR, and published releases trigger `.github/workflows/release-binaries.yml`
to attach binaries for Linux, Windows, and macOS.

If `CARGO_REGISTRY_TOKEN` is configured, the release workflow also publishes the crates. Keep
`RELEASE_PLZ_TOKEN` configured as a PAT because the binary workflow listens for release events
that the default `GITHUB_TOKEN` cannot trigger.
