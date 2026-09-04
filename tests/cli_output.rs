use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("smartedit-cli-output-{}-{unique}", process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ast_print_exits_successfully_when_stdout_reader_closes() {
    let dir = TestDir::new();
    let source_path = dir.path().join("large.rs");
    let source: String = (0..20_000)
        .map(|index| format!("fn item_{index}() {{}}\n"))
        .collect();
    fs::write(&source_path, source).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_smartedit"))
        .args(["ast-print", "--loc"])
        .arg(&source_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "smartedit failed after a broken stdout pipe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}

fn quoted_path_operand(path: &Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[test]
fn apply_dry_run_accepts_quoted_paths_with_spaces_hashes_and_semicolons() {
    let dir = TestDir::new();
    let source_dir = dir.path().join("source; # files");
    let source_path = source_dir.join("input file.txt");
    let destination = dir.path().join("destination; # files");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(&source_path, "before\n").unwrap();

    let line_operation = format!("ld {}:0-1", quoted_path_operand(&source_path));
    let line_output = Command::new(env!("CARGO_BIN_EXE_smartedit"))
        .args(["apply", "--dry-run", &line_operation])
        .output()
        .unwrap();
    assert!(
        line_output.status.success(),
        "quoted line target failed: {}",
        String::from_utf8_lossy(&line_output.stderr)
    );

    let move_operation = format!(
        "m {} {}",
        quoted_path_operand(&source_path),
        quoted_path_operand(&destination)
    );
    let move_output = Command::new(env!("CARGO_BIN_EXE_smartedit"))
        .args(["apply", "--dry-run", &move_operation])
        .output()
        .unwrap();
    assert!(
        move_output.status.success(),
        "quoted move operands failed: {}",
        String::from_utf8_lossy(&move_output.stderr)
    );

    let glob = source_dir.join("*.txt");
    let replace_operation = format!("tr {} \"before\" \"after\"", quoted_path_operand(&glob));
    let replace_output = Command::new(env!("CARGO_BIN_EXE_smartedit"))
        .args(["apply", "--dry-run", &replace_operation])
        .output()
        .unwrap();
    assert!(
        replace_output.status.success(),
        "quoted glob source failed: {}",
        String::from_utf8_lossy(&replace_output.stderr)
    );
}
