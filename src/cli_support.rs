use std::io::{self, Write};
use std::path::{Path, PathBuf};

use smartedit::{ParseError, ProgramMode};
use unicode_width::UnicodeWidthChar;

const DIAGNOSTIC_TAB_STOP: usize = 4;

pub fn write_stdout(output: &str) -> Result<(), String> {
    let stdout = io::stdout();
    write_output(stdout.lock(), output)
        .map_err(|error| format!("failed to write command output: {error}"))
}

fn write_output(mut writer: impl Write, output: &str) -> io::Result<()> {
    match writer
        .write_all(output.as_bytes())
        .and_then(|()| writer.flush())
    {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

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

        let (line_number, line_start, line_end) = line_details(input, error.span.start);
        let line = &input[line_start..line_end];
        let span_start = floor_char_boundary(input, error.span.start.clamp(line_start, line_end));
        let span_end = ceil_char_boundary(input, error.span.end.clamp(span_start, line_end));
        let caret_start = display_width(&input[line_start..span_start]);
        let mut caret_end = display_width(&input[line_start..span_end]);
        if caret_end <= caret_start {
            caret_end = caret_start + 1;
        }
        let column_number = caret_start + 1;

        message.push_str(&format!(
            "{source_name}:{line_number}:{column_number}: {}\n",
            error.message
        ));
        message.push_str(&expand_tabs(line));
        message.push('\n');
        message.push_str(&" ".repeat(caret_start));
        message.push_str(&"^".repeat(caret_end - caret_start));
    }

    message
}

fn line_details(input: &str, offset: usize) -> (usize, usize, usize) {
    let clamped = floor_char_boundary(input, offset.min(input.len()));
    let line_start = input[..clamped].rfind('\n').map_or(0, |index| index + 1);
    let line_end = input[clamped..]
        .find('\n')
        .map_or(input.len(), |index| clamped + index);
    let line_number = input[..clamped]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    (line_number, line_start, line_end)
}

fn floor_char_boundary(input: &str, mut offset: usize) -> usize {
    while !input.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn ceil_char_boundary(input: &str, mut offset: usize) -> usize {
    while offset < input.len() && !input.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

fn display_width(input: &str) -> usize {
    input.chars().fold(0, |column, character| {
        if character == '\t' {
            column + DIAGNOSTIC_TAB_STOP - (column % DIAGNOSTIC_TAB_STOP)
        } else {
            column + character.width().unwrap_or(0)
        }
    })
}

fn expand_tabs(input: &str) -> String {
    let mut expanded = String::with_capacity(input.len());
    let mut column = 0;
    for character in input.chars() {
        if character == '\t' {
            let spaces = DIAGNOSTIC_TAB_STOP - (column % DIAGNOSTIC_TAB_STOP);
            expanded.push_str(&" ".repeat(spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += character.width().unwrap_or(0);
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use smartedit::{ParseError, Span};

    use super::{format_parse_errors, write_output};

    struct FailingWriter(io::ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn diagnostic(input: &str, start: usize, end: usize) -> String {
        format_parse_errors(
            "test.smedit",
            input,
            &[ParseError {
                message: "invalid input".to_owned(),
                span: Span::new(start, end),
            }],
        )
    }

    #[test]
    fn caret_uses_character_width_after_multibyte_text() {
        assert_eq!(
            diagnostic("éx", "é".len(), "éx".len()),
            "test.smedit:1:2: invalid input\néx\n ^"
        );
    }

    #[test]
    fn caret_uses_display_width_after_combining_and_wide_characters() {
        let combining = "e\u{301}x";
        assert_eq!(
            diagnostic(combining, "e\u{301}".len(), combining.len()),
            "test.smedit:1:2: invalid input\ne\u{301}x\n ^"
        );

        assert_eq!(
            diagnostic("界x", "界".len(), "界x".len()),
            "test.smedit:1:3: invalid input\n界x\n  ^"
        );
    }

    #[test]
    fn caret_width_covers_wide_characters() {
        assert_eq!(
            diagnostic("界x", 0, "界".len()),
            "test.smedit:1:1: invalid input\n界x\n^^"
        );
    }

    #[test]
    fn tabs_expand_to_four_column_stops() {
        assert_eq!(
            diagnostic("\tx", 1, 2),
            "test.smedit:1:5: invalid input\n    x\n    ^"
        );
    }

    #[test]
    fn zero_width_eof_span_gets_one_caret() {
        assert_eq!(
            diagnostic("abc", 3, 3),
            "test.smedit:1:4: invalid input\nabc\n   ^"
        );
    }

    #[test]
    fn non_boundary_span_is_clamped_without_panicking() {
        assert_eq!(
            diagnostic("é", 1, 1),
            "test.smedit:1:1: invalid input\né\n^"
        );
    }

    #[test]
    fn broken_pipe_while_writing_stdout_is_successful() {
        assert!(write_output(FailingWriter(io::ErrorKind::BrokenPipe), "output\n").is_ok());
    }

    #[test]
    fn non_pipe_output_errors_are_preserved() {
        let error =
            write_output(FailingWriter(io::ErrorKind::PermissionDenied), "output\n").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
