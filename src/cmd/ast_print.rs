use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use globset::Glob;
use ignore::WalkBuilder;
use smartedit::{AstLanguage, AstRenderOptions, AstSelector, parse_file_ast};

use crate::cli_support::{display_path, resolve_root, write_stdout};

#[derive(Debug, Args)]
pub struct CmdAstPrint {
    /// Select item paths with glob syntax (for example `module.Type.method`).
    #[arg(short = 's', long = "select", value_name = "ITEM_GLOB")]
    pub selectors: Vec<String>,

    /// Select types and their associated items; qualified paths are accepted.
    #[arg(short = 'S', long = "type-select", value_name = "TYPE_GLOB")]
    pub type_selectors: Vec<String>,

    /// Render declaration signatures; type bodies are omitted.
    #[arg(long)]
    pub signatures: bool,

    /// Render complete type, trait, impl, module, macro, and foreign-block bodies.
    #[arg(long = "type-bodies")]
    pub type_bodies: bool,

    /// Render complete function bodies.
    #[arg(long = "function-bodies")]
    pub function_bodies: bool,

    /// Include documentation comments, leading comments, or docstrings.
    #[arg(long)]
    pub doc: bool,

    /// Prefix items with zero-based, half-open line ranges; shared lines are marked unsafe.
    #[arg(short = 'l', long = "loc")]
    pub loc: bool,

    /// Include files normally excluded by ignore rules when expanding globs.
    #[arg(long = "no-ignore")]
    pub no_ignore: bool,

    #[arg(value_name = "PATH_OR_GLOB", required = true)]
    pub inputs: Vec<String>,
}

#[derive(Debug, Default)]
struct ResolvedAstInputs {
    supported_files: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
struct RenderedAstFile {
    path: PathBuf,
    rendered: String,
}

impl CmdAstPrint {
    pub fn run(&self) -> Result<(), String> {
        let current_dir =
            env::current_dir().map_err(|error| format!("failed to get cwd: {error}"))?;
        let resolved = resolve_ast_inputs(&self.inputs, &current_dir, self.no_ignore)?;
        if resolved.supported_files.is_empty() {
            return Err("no supported AST files matched the provided inputs".to_owned());
        }

        let options = AstRenderOptions {
            include_signatures: self.signatures,
            include_type_bodies: self.type_bodies,
            include_function_bodies: self.function_bodies,
            include_docs: self.doc,
            include_locations: self.loc,
        };
        let selector = AstSelector {
            item_patterns: self.selectors.clone(),
            type_patterns: self.type_selectors.clone(),
        };

        let rendered_files =
            render_ast_files(&resolved.supported_files, &current_dir, &selector, options)?;
        let show_file_markers = resolved.supported_files.len() > 1;
        let output = format_ast_output(&rendered_files, &current_dir, show_file_markers);
        write_stdout(&output)
    }
}

fn format_ast_output(
    rendered_files: &[RenderedAstFile],
    current_dir: &Path,
    show_file_markers: bool,
) -> String {
    let mut output = String::new();
    for (index, file) in rendered_files.iter().enumerate() {
        if show_file_markers {
            if index > 0 {
                output.push('\n');
            }
            writeln!(
                output,
                "{}",
                format_file_marker(&file.path, current_dir, false)
            )
            .expect("writing to a string cannot fail");
        }
        writeln!(output, "{}", file.rendered).expect("writing to a string cannot fail");
        if show_file_markers {
            writeln!(
                output,
                "{}",
                format_file_marker(&file.path, current_dir, true)
            )
            .expect("writing to a string cannot fail");
        }
    }
    output
}

fn render_ast_files(
    paths: &[PathBuf],
    current_dir: &Path,
    selector: &AstSelector,
    options: AstRenderOptions,
) -> Result<Vec<RenderedAstFile>, String> {
    let mut rendered_files = Vec::new();
    for path in paths {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let ast = parse_file_ast(path, &source).map_err(|error| error.to_string())?;
        let render_options = AstRenderOptions {
            include_locations: options.include_locations && !ast.has_errors,
            ..options
        };
        let rendered = if selector.is_empty() {
            ast.render(render_options)
        } else {
            let selected = ast
                .select_items(selector)
                .map_err(|error| error.to_string())?;
            if selected.is_empty() {
                continue;
            }
            ast.render_with_selector(selector, render_options)
                .map_err(|error| format!("{}: {error}", display_path(path, current_dir)))?
        };
        rendered_files.push(RenderedAstFile {
            path: path.clone(),
            rendered,
        });
    }

    if !selector.is_empty() && rendered_files.is_empty() {
        return Err(format!(
            "no AST items matched selector {} across all input files",
            selector.display()
        ));
    }

    Ok(rendered_files)
}

fn format_file_marker(path: &Path, current_dir: &Path, is_end: bool) -> String {
    let path = display_path(path, current_dir);
    if is_end {
        format!("== end {path} ==")
    } else {
        format!("== {path} ==")
    }
}

fn resolve_ast_inputs(
    inputs: &[String],
    current_dir: &Path,
    no_ignore: bool,
) -> Result<ResolvedAstInputs, String> {
    let mut supported_files = BTreeSet::new();

    for input in inputs {
        if looks_like_glob(input) {
            let matched = resolve_glob_input(input, current_dir, no_ignore)?;
            if matched.is_empty() {
                return Err(format!("no files matched glob `{input}`"));
            }
            for path in matched {
                if AstLanguage::from_path(&path).is_some() {
                    supported_files.insert(path);
                }
            }
            continue;
        }

        let path = resolve_root(Some(Path::new(input)), current_dir);
        if !path.exists() {
            return Err(format!("file does not exist: {}", path.display()));
        }
        if path.is_dir() {
            return Err(format!(
                "directories are not supported by `ast-print`: {}",
                path.display()
            ));
        }
        if AstLanguage::from_path(&path).is_some() {
            supported_files.insert(path);
        }
    }

    Ok(ResolvedAstInputs {
        supported_files: supported_files.into_iter().collect(),
    })
}

fn resolve_glob_input(
    input: &str,
    current_dir: &Path,
    no_ignore: bool,
) -> Result<Vec<PathBuf>, String> {
    let (root, pattern) = split_glob_root(input);
    let root = if root.is_absolute() {
        root
    } else {
        current_dir.join(root)
    };
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        return Err(format!(
            "glob root is not a directory for `{input}`: {}",
            root.display()
        ));
    }

    let matcher = Glob::new(&pattern)
        .map_err(|error| format!("invalid glob pattern `{input}`: {error}"))?
        .compile_matcher();
    let mut files = Vec::new();

    let mut walker = WalkBuilder::new(&root);
    walker.require_git(false);
    if no_ignore {
        walker
            .hidden(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false);
    } else {
        walker.standard_filters(true);
    }

    for entry in walker.build() {
        let entry = entry.map_err(|error| format!("failed to walk {}: {error}", root.display()))?;
        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let path = entry.into_path();
        let relative_path = path
            .strip_prefix(&root)
            .map_err(|error| format!("failed to normalize {}: {error}", path.display()))?;
        if matcher.is_match(normalize_path_for_glob(relative_path)) {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

fn looks_like_glob(input: &str) -> bool {
    input.contains('*') || input.contains('?') || input.contains('[')
}

fn split_glob_root(input: &str) -> (PathBuf, String) {
    let normalized = input.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').collect();
    let wildcard_index = segments
        .iter()
        .position(|segment| segment.contains('*') || segment.contains('?') || segment.contains('['))
        .unwrap_or(segments.len());

    let root = if wildcard_index == 0 {
        PathBuf::from(".")
    } else {
        PathBuf::from(segments[..wildcard_index].join("/"))
    };
    let pattern = segments[wildcard_index..].join("/");

    (normalize_empty_path(root), pattern)
}

fn normalize_empty_path(path: PathBuf) -> PathBuf {
    if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path
    }
}

fn normalize_path_for_glob(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use smartedit::{AstRenderOptions, AstSelector};

    use super::{
        format_ast_output, format_file_marker, render_ast_files, resolve_ast_inputs,
        resolve_glob_input, split_glob_root,
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "smartedit-ast-print-{name}-{}-{unique}",
                process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn glob_input_respects_gitignore_filters() {
        let dir = TestDir::new("gitignore");
        fs::create_dir_all(dir.path().join("src/ignored")).unwrap();
        fs::write(dir.path().join(".gitignore"), "src/ignored/\n").unwrap();
        fs::write(dir.path().join("src/keep.rs"), "fn keep() {}\n").unwrap();
        fs::write(
            dir.path().join("src/ignored/skip.rs"),
            "fn skipped_by_gitignore() {}\n",
        )
        .unwrap();

        let matches = resolve_glob_input("src/**/*", dir.path(), false).unwrap();

        assert_eq!(matches, vec![dir.path().join("src/keep.rs")]);
    }

    #[test]
    fn glob_input_can_disable_ignore_filters() {
        let dir = TestDir::new("no-ignore");
        fs::create_dir_all(dir.path().join("src/ignored")).unwrap();
        fs::write(dir.path().join(".gitignore"), "src/ignored/\n").unwrap();
        fs::write(dir.path().join("src/keep.rs"), "fn keep() {}\n").unwrap();
        fs::write(
            dir.path().join("src/ignored/skip.rs"),
            "fn included_when_no_ignore() {}\n",
        )
        .unwrap();

        let matches = resolve_glob_input("src/**/*", dir.path(), true).unwrap();

        assert_eq!(
            matches,
            vec![
                dir.path().join("src/ignored/skip.rs"),
                dir.path().join("src/keep.rs")
            ]
        );
    }

    #[test]
    fn resolve_inputs_skips_unsupported_glob_matches_silently() {
        let dir = TestDir::new("unsupported-glob");
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/keep.rs"), "fn keep() {}\n").unwrap();
        fs::write(dir.path().join("src/notes.txt"), "plain text\n").unwrap();

        let resolved = resolve_ast_inputs(&["src/**/*".to_owned()], dir.path(), false).unwrap();

        assert_eq!(
            resolved.supported_files,
            vec![dir.path().join("src/keep.rs")]
        );
    }

    #[test]
    fn resolve_inputs_skips_unsupported_direct_paths_silently() {
        let dir = TestDir::new("unsupported-direct");
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/keep.rs"), "fn keep() {}\n").unwrap();
        fs::write(dir.path().join("notes.txt"), "plain text\n").unwrap();

        let resolved = resolve_ast_inputs(
            &["notes.txt".to_owned(), "src/keep.rs".to_owned()],
            dir.path(),
            false,
        )
        .unwrap();

        assert_eq!(
            resolved.supported_files,
            vec![dir.path().join("src/keep.rs")]
        );
    }

    #[test]
    fn resolve_inputs_include_supported_non_rust_files() {
        let dir = TestDir::new("multi-language-direct");
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/keep.rs"), "fn keep() {}\n").unwrap();
        fs::write(dir.path().join("src/tool.py"), "def run():\n    pass\n").unwrap();
        fs::write(
            dir.path().join("src/api.pyi"),
            "def request(value: str) -> str: ...\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/tool.js"), "export function run() {}\n").unwrap();
        fs::write(
            dir.path().join("src/tool.ts"),
            "export function run(): void {}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/tool.go"),
            "package tool\n\nfunc Run() {}\n",
        )
        .unwrap();

        let resolved = resolve_ast_inputs(&["src/**/*".to_owned()], dir.path(), false).unwrap();

        assert_eq!(
            resolved.supported_files,
            vec![
                dir.path().join("src/api.pyi"),
                dir.path().join("src/keep.rs"),
                dir.path().join("src/tool.go"),
                dir.path().join("src/tool.js"),
                dir.path().join("src/tool.py"),
                dir.path().join("src/tool.ts")
            ]
        );

        let direct = resolve_ast_inputs(&["src/api.pyi".to_owned()], dir.path(), false).unwrap();
        assert_eq!(direct.supported_files, vec![dir.path().join("src/api.pyi")]);
    }

    #[test]
    fn formats_matching_start_and_end_file_markers() {
        let dir = TestDir::new("markers");
        let path = dir.path().join("src/main.rs");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "fn main() {}\n").unwrap();

        assert_eq!(
            format_file_marker(&path, dir.path(), false),
            "== src/main.rs =="
        );
        assert_eq!(
            format_file_marker(&path, dir.path(), true),
            "== end src/main.rs =="
        );
    }

    #[test]
    fn formats_ast_output_before_writing_it() {
        let dir = TestDir::new("formatted-output");
        let files = vec![
            super::RenderedAstFile {
                path: dir.path().join("first.rs"),
                rendered: "fn first".to_owned(),
            },
            super::RenderedAstFile {
                path: dir.path().join("second.rs"),
                rendered: "fn second".to_owned(),
            },
        ];

        assert_eq!(
            format_ast_output(&files, dir.path(), true),
            concat!(
                "== first.rs ==\n",
                "fn first\n",
                "== end first.rs ==\n",
                "\n",
                "== second.rs ==\n",
                "fn second\n",
                "== end second.rs ==\n",
            )
        );
    }

    #[test]
    fn glob_root_accepts_windows_path_separators() {
        assert_eq!(
            split_glob_root(r"C:\work\src\*.rs"),
            (PathBuf::from("C:/work/src"), "*.rs".to_owned())
        );
        assert_eq!(
            split_glob_root(r"C:\work\src\**\*.ts"),
            (PathBuf::from("C:/work/src"), "**/*.ts".to_owned())
        );
        assert_eq!(
            split_glob_root(r"src\generated\*.py"),
            (PathBuf::from("src/generated"), "*.py".to_owned())
        );
    }

    #[test]
    fn selectors_match_across_files_instead_of_requiring_every_file() {
        let dir = TestDir::new("aggregate-selectors");
        let first = dir.path().join("first.rs");
        let middle = dir.path().join("middle.rs");
        let last = dir.path().join("last.rs");
        fs::write(&first, "fn first() {}\n").unwrap();
        fs::write(&middle, "fn wanted() {}\n").unwrap();
        fs::write(&last, "fn last() {}\n").unwrap();

        let paths = [first.clone(), middle.clone(), last.clone()];
        for (selector, expected_path) in [("first", first), ("wanted", middle), ("last", last)] {
            let rendered = render_ast_files(
                &paths,
                dir.path(),
                &AstSelector {
                    item_patterns: vec![selector.to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap();

            assert_eq!(rendered.len(), 1);
            assert_eq!(rendered[0].path, expected_path);
            assert_eq!(rendered[0].rendered, format!("fn {selector}"));
        }
    }

    #[test]
    fn selectors_fail_only_after_no_match_across_all_files() {
        let dir = TestDir::new("aggregate-no-match");
        let first = dir.path().join("first.rs");
        let last = dir.path().join("last.rs");
        fs::write(&first, "fn first() {}\n").unwrap();
        fs::write(&last, "fn last() {}\n").unwrap();

        let error = render_ast_files(
            &[first, last],
            dir.path(),
            &AstSelector {
                item_patterns: vec!["missing".to_owned()],
                type_patterns: Vec::new(),
            },
            AstRenderOptions::default(),
        )
        .unwrap_err();

        assert_eq!(
            error,
            "no AST items matched selector -s missing across all input files"
        );
    }

    #[test]
    fn python_selectors_and_stub_globs_work_across_files() {
        let dir = TestDir::new("python-stub-selectors");
        let ordinary = dir.path().join("ordinary.py");
        let stub = dir.path().join("service.pyi");
        fs::write(&ordinary, "class Ordinary: pass\n").unwrap();
        fs::write(
            &stub,
            "class Service:\n    def request(self, value: str) -> str: ...\n",
        )
        .unwrap();

        let resolved = resolve_ast_inputs(&["*.py*".to_owned()], dir.path(), false).unwrap();
        assert_eq!(
            resolved.supported_files,
            vec![ordinary.clone(), stub.clone()]
        );

        let rendered = render_ast_files(
            &resolved.supported_files,
            dir.path(),
            &AstSelector {
                item_patterns: Vec::new(),
                type_patterns: vec!["Service".to_owned()],
            },
            AstRenderOptions {
                include_signatures: true,
                ..AstRenderOptions::default()
            },
        )
        .unwrap();
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].path, stub);
        assert_eq!(
            rendered[0].rendered,
            "class Service:\n> def request(self, value: str) -> str:"
        );
    }

    #[test]
    fn syntax_errors_render_recovered_items_without_locations() {
        let dir = TestDir::new("syntax-errors");
        let fixtures = [
            (
                "broken.rs",
                "fn recovered() {}\nfn broken(\n",
                "fn recovered",
            ),
            (
                "broken.py",
                "def recovered():\n    pass\n\ndef broken(\n",
                "def recovered",
            ),
            (
                "broken.js",
                "function recovered() {}\nfunction broken( {\n",
                "function recovered",
            ),
            (
                "broken.ts",
                "interface Recovered {}\ninterface Broken<T {\n",
                "interface Recovered",
            ),
            (
                "broken.tsx",
                "const recovered = () => <div />;\nconst broken = <div>;\n",
                "function recovered",
            ),
            (
                "broken.go",
                "package broken\n\nfunc Recovered() {}\nfunc Broken( {\n",
                "func Recovered",
            ),
        ];

        for (name, source, expected) in fixtures {
            let path = dir.path().join(name);
            fs::write(&path, source).unwrap();
            let rendered = render_ast_files(
                std::slice::from_ref(&path),
                dir.path(),
                &AstSelector::default(),
                AstRenderOptions {
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
            assert_eq!(rendered.len(), 1);
            assert!(rendered[0].rendered.contains(expected), "{name}");
            assert!(
                rendered[0]
                    .rendered
                    .lines()
                    .all(|line| !line.trim_start().starts_with('[')),
                "locations must be suppressed for recovered {name}: {}",
                rendered[0].rendered
            );
        }
    }

    #[test]
    fn incomplete_or_statement_shaped_go_inputs_are_still_renderable() {
        let dir = TestDir::new("go-source-shape");
        for (name, source) in [
            ("missing-package.go", "func MissingPackage() {}\n"),
            ("duplicate-package.go", "package first\npackage second\n"),
            ("misordered-package.go", "var Before int\npackage sample\n"),
            ("short-var.go", "package sample\nvalue := 1\n"),
            ("return.go", "package sample\nreturn\n"),
        ] {
            let path = dir.path().join(name);
            fs::write(&path, source).unwrap();
            let rendered = render_ast_files(
                std::slice::from_ref(&path),
                dir.path(),
                &AstSelector::default(),
                AstRenderOptions {
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
            assert_eq!(rendered.len(), 1, "{name}");
            assert!(
                rendered[0]
                    .rendered
                    .lines()
                    .all(|line| !line.trim_start().starts_with('[')),
                "locations must be suppressed for recovered {name}: {}",
                rendered[0].rendered
            );
        }
    }

    #[test]
    fn multi_file_rendering_keeps_locations_only_for_valid_files() {
        let dir = TestDir::new("syntax-error-batch");
        let valid = dir.path().join("valid.rs");
        let broken = dir.path().join("broken.rs");
        fs::write(&valid, "fn valid() {}\n").unwrap();
        fs::write(&broken, "fn recovered() {}\nfn broken(\n").unwrap();

        let rendered = render_ast_files(
            &[valid, broken],
            dir.path(),
            &AstSelector::default(),
            AstRenderOptions {
                include_locations: true,
                ..AstRenderOptions::default()
            },
        )
        .unwrap();

        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].rendered.starts_with('['));
        assert!(rendered[1].rendered.contains("fn recovered"));
        assert!(!rendered[1].rendered.starts_with('['));
    }
}
