use std::path::{Path, PathBuf};

use smartedit::{ParseError, ProgramMode};

pub fn resolve_root(root: Option<&Path>, current_dir: &Path) -> PathBuf {
    match root {
        Some(root) if root.is_absolute() => root.to_path_buf(),
        Some(root) => current_dir.join(root),
        None => current_dir.to_path_buf(),
    }
}

pub fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub fn format_program_mode(mode: ProgramMode) -> &'static str {
    match mode {
        ProgramMode::Snapshot => "snapshot",
        ProgramMode::Incremental => "incremental",
    }
}

pub fn format_parse_errors(source_name: &str, input: &str, errors: &[ParseError]) -> String {
    let mut message = String::new();

    for (index, error) in errors.iter().enumerate() {
        if index > 0 {
            message.push('\n');
            message.push('\n');
        }

        let (line_number, column_number, line_start, line_end) =
            line_details(input, error.span.start);
        let line = &input[line_start..line_end];
        let caret_start = error.span.start.saturating_sub(line_start);
        let mut caret_end = error.span.end.min(line_end).saturating_sub(line_start);
        if caret_end <= caret_start {
            caret_end = caret_start + 1;
        }

        message.push_str(&format!(
            "{source_name}:{line_number}:{column_number}: {}\n",
            error.message
        ));
        message.push_str(line);
        message.push('\n');
        message.push_str(&" ".repeat(caret_start));
        message.push_str(&"^".repeat(caret_end - caret_start));
    }

    message
}

fn line_details(input: &str, offset: usize) -> (usize, usize, usize, usize) {
    let clamped = offset.min(input.len());
    let line_start = input[..clamped].rfind('\n').map_or(0, |index| index + 1);
    let line_end = input[clamped..]
        .find('\n')
        .map_or(input.len(), |index| clamped + index);
    let line_number = input[..clamped]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let column_number = input[line_start..clamped].chars().count() + 1;
    (line_number, column_number, line_start, line_end)
}
