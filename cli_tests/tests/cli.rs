use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{self, Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use libtest_mimic::{Arguments, Failed, Trial};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

// Keep this list in sync with `smartedit::file_ast::AstLanguage`.
const SUPPORTED_LANGUAGE_FIXTURES: &[&str] = &[
    "bash",
    "c",
    "cpp",
    "csharp",
    "go",
    "java",
    "javascript",
    "json",
    "kotlin",
    "lua",
    "php",
    "python",
    "ruby",
    "rust",
    "scala",
    "toml",
    "tsx",
    "typescript",
    "yaml",
];

#[derive(Debug)]
struct FixtureTest {
    name: String,
    language_dir: PathBuf,
    spec_path: PathBuf,
}

#[derive(Debug)]
struct Spec {
    command: Vec<String>,
    expected_stdout: String,
    expected_files: Vec<(PathBuf, String)>,
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(test_name: &str) -> Result<Self, String> {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let safe_name: String = test_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect();
        let path = std::env::temp_dir().join(format!(
            "smartedit-cli-test-{}-{sequence}-{safe_name}",
            process::id()
        ));
        fs::create_dir(&path)
            .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() {
    let args = Arguments::from_args();
    let binary = build_smartedit().unwrap_or_else(|error| panic!("{error}"));
    let fixtures = discover_fixture_tests().unwrap_or_else(|error| panic!("{error}"));

    let mut tests: Vec<_> = fixtures
        .into_iter()
        .map(|fixture| {
            let binary = binary.clone();
            Trial::test(fixture.name.clone(), move || {
                run_fixture_test(&binary, &fixture).map_err(Failed::from)
            })
        })
        .collect();
    tests.push(Trial::test("harness/normalizes-crlf-sources", || {
        verify_crlf_source_normalization().map_err(Failed::from)
    }));

    libtest_mimic::run(&args, tests).exit();
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cli_tests must be directly inside the workspace")
        .to_path_buf()
}

fn build_smartedit() -> Result<PathBuf, String> {
    let workspace = workspace_root();
    let test_executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate cli test executable: {error}"))?;
    let profile_dir = test_executable
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            format!(
                "unexpected cli test executable path: {}",
                test_executable.display()
            )
        })?;
    let profile_dir_name = file_name(profile_dir)?;
    let cargo_profile = if profile_dir_name == "debug" {
        "dev"
    } else {
        &profile_dir_name
    };
    let output = Command::new(env!("CARGO"))
        .args([
            "build",
            "--quiet",
            "--package",
            "smartedit-cli",
            "--profile",
            cargo_profile,
        ])
        .current_dir(&workspace)
        .output()
        .map_err(|error| format!("failed to invoke cargo to build smartedit: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "failed to build smartedit:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let binary = profile_dir.join(format!("smartedit{}", std::env::consts::EXE_SUFFIX));

    if !binary.is_file() {
        return Err(format!(
            "cargo built smartedit, but {} was not found",
            binary.display()
        ));
    }

    Ok(binary)
}

fn discover_fixture_tests() -> Result<Vec<FixtureTest>, String> {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let language_dirs = read_sorted_dirs(&fixtures_dir)?;
    let language_names = language_dirs
        .iter()
        .map(|path| file_name(path))
        .collect::<Result<Vec<_>, _>>()?;

    for language in &language_names {
        if !SUPPORTED_LANGUAGE_FIXTURES.contains(&language.as_str()) {
            return Err(format!(
                "unsupported language fixture directory: {}",
                fixtures_dir.join(language).display()
            ));
        }
    }

    let mut fixtures = Vec::new();

    for language in SUPPORTED_LANGUAGE_FIXTURES {
        let language_dir = fixtures_dir.join(language);
        if !language_dir.is_dir() {
            return Err(format!(
                "missing fixture directory for supported language `{language}`: {}",
                language_dir.display()
            ));
        }

        let source_dir = language_dir.join("src");
        if !source_dir.is_dir() {
            return Err(format!(
                "missing source fixture directory for supported language `{language}`: {}",
                source_dir.display()
            ));
        }
        if !directory_contains_file(&source_dir)? {
            return Err(format!(
                "source fixture directory for supported language `{language}` is empty: {}",
                source_dir.display()
            ));
        }

        for suite in ["ast-print", "apply"] {
            let suite_dir = language_dir.join(suite);
            if !suite_dir.is_dir() {
                return Err(format!(
                    "missing `{suite}` fixture suite for supported language `{language}`: {}",
                    suite_dir.display()
                ));
            }

            let mut specs = Vec::new();
            collect_specs(&suite_dir, &mut specs)?;
            specs.sort();
            if specs.is_empty() {
                return Err(format!(
                    "`{suite}` fixture suite for supported language `{language}` has no .test specifications: {}",
                    suite_dir.display()
                ));
            }
            for spec_path in specs {
                let relative = spec_path.strip_prefix(&suite_dir).map_err(|error| {
                    format!("failed to name fixture {}: {error}", spec_path.display())
                })?;
                let case = relative
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/");
                fixtures.push(FixtureTest {
                    name: format!("{language}/{suite}/{case}"),
                    language_dir: language_dir.clone(),
                    spec_path,
                });
            }
        }
    }

    if fixtures.is_empty() {
        return Err(format!(
            "no .test fixture specifications found under {}",
            fixtures_dir.display()
        ));
    }

    Ok(fixtures)
}

fn directory_contains_file(path: &Path) -> Result<bool, String> {
    let entries = fs::read_dir(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_file() || (file_type.is_dir() && directory_contains_file(&entry.path())?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn read_sorted_dirs(path: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn collect_specs(path: &Path, specs: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            collect_specs(&entry.path(), specs)?;
        } else if file_type.is_file() && entry.path().extension() == Some(OsStr::new("test")) {
            specs.push(entry.path());
        }
    }
    Ok(())
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("fixture path has no UTF-8 file name: {}", path.display()))
}

fn run_fixture_test(binary: &Path, fixture: &FixtureTest) -> Result<(), String> {
    let spec = parse_spec(&fixture.spec_path)?;
    let temp = TempDir::new(&fixture.name)?;
    copy_dir(&fixture.language_dir, temp.path())?;
    normalize_fixture_sources(&temp.path().join("src"))?;

    let output = Command::new(binary)
        .args(&spec.command)
        .current_dir(temp.path())
        .output()
        .map_err(|error| format!("failed to execute `{}`: {error}", spec.command.join(" ")))?;

    assert_success(&fixture.spec_path, &spec, &output)?;

    let actual_stdout = normalize_newlines(&String::from_utf8_lossy(&output.stdout));
    if actual_stdout != spec.expected_stdout {
        return Err(mismatch(
            &fixture.spec_path,
            "stdout",
            &spec.expected_stdout,
            &actual_stdout,
        ));
    }

    for (relative_path, expected) in &spec.expected_files {
        let actual_path = temp.path().join(relative_path);
        let actual = fs::read_to_string(&actual_path).map_err(|error| {
            format!(
                "{}: failed to read expected output file {}: {error}",
                fixture.spec_path.display(),
                relative_path.display()
            )
        })?;
        let actual = normalize_newlines(&actual);
        if actual != *expected {
            return Err(mismatch(
                &fixture.spec_path,
                &format!("file {}", relative_path.display()),
                expected,
                &actual,
            ));
        }
    }

    Ok(())
}

fn assert_success(spec_path: &Path, spec: &Spec, output: &Output) -> Result<(), String> {
    if !output.status.success() {
        return Err(format!(
            "{}: `{}` exited with {}\nstdout:\n{}\nstderr:\n{}",
            spec_path.display(),
            spec.command.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    if !output.stderr.is_empty() {
        return Err(format!(
            "{}: `{}` wrote unexpected stderr:\n{}",
            spec_path.display(),
            spec.command.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn mismatch(spec_path: &Path, subject: &str, expected: &str, actual: &str) -> String {
    format!(
        "{}: {subject} mismatch\nexpected:\n---\n{expected}---\nactual:\n---\n{actual}---",
        spec_path.display()
    )
}

fn parse_spec(path: &Path) -> Result<Spec, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let normalized = normalize_newlines(&text);
    let lines: Vec<&str> = normalized.lines().collect();
    let command_line = lines
        .first()
        .and_then(|line| line.strip_prefix("> "))
        .ok_or_else(|| format!("{}: first line must start with `> `", path.display()))?;
    let mut command = shell_words::split(command_line)
        .map_err(|error| format!("{}: invalid command line: {error}", path.display()))?;
    if command
        .first()
        .is_some_and(|argument| argument == "smartedit")
    {
        command.remove(0);
    }
    if command.is_empty() {
        return Err(format!("{}: command must not be empty", path.display()));
    }

    let separator = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.starts_with("==="))
        .map(|(index, _)| index)
        .ok_or_else(|| format!("{}: missing `===` separator", path.display()))?;
    let expected_stdout = joined_fixture_lines(&lines[1..separator]);
    let mut expected_files = Vec::new();
    let mut index = separator;

    loop {
        let boundary = lines[index];
        if boundary == "===" {
            if lines[index + 1..].iter().any(|line| !line.is_empty()) {
                return Err(format!(
                    "{}: unexpected content after final `===`",
                    path.display()
                ));
            }
            break;
        }

        let relative = boundary.strip_prefix("=== ").ok_or_else(|| {
            format!(
                "{}: separators must be `===` or `=== relative/path`",
                path.display()
            )
        })?;
        let relative = validate_relative_path(path, relative)?;
        let next = lines
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, line)| line.starts_with("==="))
            .map(|(line_index, _)| line_index)
            .ok_or_else(|| {
                format!(
                    "{}: expected file section `{}` is missing a closing `===`",
                    path.display(),
                    relative.display()
                )
            })?;
        expected_files.push((relative, joined_fixture_lines(&lines[index + 1..next])));
        index = next;
    }

    Ok(Spec {
        command,
        expected_stdout,
        expected_files,
    })
}

fn validate_relative_path(spec_path: &Path, value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(format!(
            "{}: expected file path must stay inside the fixture: {value:?}",
            spec_path.display()
        ));
    }
    Ok(path)
}

fn joined_fixture_lines(lines: &[&str]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n")
}

fn normalize_fixture_sources(path: &Path) -> Result<(), String> {
    for entry in
        fs::read_dir(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry_path.display()))?;

        if file_type.is_dir() {
            normalize_fixture_sources(&entry_path)?;
        } else if file_type.is_file() {
            let bytes = fs::read(&entry_path)
                .map_err(|error| format!("failed to read {}: {error}", entry_path.display()))?;
            if bytes.windows(2).any(|window| window == b"\r\n") {
                let Ok(text) = String::from_utf8(bytes) else {
                    continue;
                };
                fs::write(&entry_path, text.replace("\r\n", "\n")).map_err(|error| {
                    format!("failed to normalize {}: {error}", entry_path.display())
                })?;
            }
        }
    }

    Ok(())
}

fn verify_crlf_source_normalization() -> Result<(), String> {
    let temp = TempDir::new("crlf-normalization")?;
    let text_path = temp.path().join("source.txt");
    let binary_path = temp.path().join("source.bin");
    let binary = b"binary:\xff\r\n";

    fs::write(&text_path, b"first\r\nsecond\r\n")
        .map_err(|error| format!("failed to write {}: {error}", text_path.display()))?;
    fs::write(&binary_path, binary)
        .map_err(|error| format!("failed to write {}: {error}", binary_path.display()))?;

    normalize_fixture_sources(temp.path())?;

    let text = fs::read(&text_path)
        .map_err(|error| format!("failed to read {}: {error}", text_path.display()))?;
    if text != b"first\nsecond\n" {
        return Err("CRLF fixture source was not normalized to LF".into());
    }

    let unchanged_binary = fs::read(&binary_path)
        .map_err(|error| format!("failed to read {}: {error}", binary_path.display()))?;
    if unchanged_binary != binary {
        return Err("binary fixture source was modified during normalization".into());
    }

    Ok(())
}

fn copy_dir(source: &Path, destination: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            fs::create_dir(&destination_path).map_err(|error| {
                format!("failed to create {}: {error}", destination_path.display())
            })?;
            copy_dir(&entry.path(), &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &destination_path).map_err(|error| {
                format!(
                    "failed to copy {} to {}: {error}",
                    entry.path().display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}
