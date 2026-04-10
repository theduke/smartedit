use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use globset::Glob;
use ignore::WalkBuilder;
use smartedit::{AstLanguage, AstRenderOptions, AstSelector, parse_file_ast};

use crate::cli_support::{display_path, resolve_root};

#[derive(Debug, Args)]
pub struct CmdAstPrint {
    #[arg(short = 's', long = "select")]
    pub selectors: Vec<String>,

    #[arg(short = 'S', long = "type-select")]
    pub type_selectors: Vec<String>,

    #[arg(long)]
    pub signatures: bool,

    #[arg(long = "type-bodies")]
    pub type_bodies: bool,

    #[arg(long = "function-bodies")]
    pub function_bodies: bool,

    #[arg(long)]
    pub doc: bool,

    #[arg(short = 'l', long = "loc")]
    pub loc: bool,

    #[arg(long = "no-ignore")]
    pub no_ignore: bool,

    #[arg(value_name = "PATH_OR_GLOB", required = true)]
    pub inputs: Vec<String>,
}

#[derive(Debug, Default)]
struct ResolvedAstInputs {
    supported_files: Vec<PathBuf>,
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

        for (index, path) in resolved.supported_files.iter().enumerate() {
            let source = fs::read_to_string(path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            let ast = parse_file_ast(path, &source).map_err(|error| error.to_string())?;
            let rendered = if selector.is_empty() {
                ast.render(options)
            } else {
                ast.render_with_selector(&selector, options)
                    .map_err(|error| format!("{}: {error}", display_path(path, &current_dir)))?
            };

            if resolved.supported_files.len() > 1 {
                if index > 0 {
                    println!();
                }
                println!("{}", format_file_marker(path, &current_dir, false));
            }
            println!("{rendered}");
            if resolved.supported_files.len() > 1 {
                println!("{}", format_file_marker(path, &current_dir, true));
            }
        }

        Ok(())
    }
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
    let segments: Vec<&str> = input.split('/').collect();
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

    use super::{format_file_marker, resolve_ast_inputs, resolve_glob_input};

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
}
