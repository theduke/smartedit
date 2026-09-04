use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};

use clap::Args;
use smartedit::{
    EditProgram, EvaluationPlan, ExecutionOptions, Executor, PlannedAction, ProgramMode,
    parse_edit_program,
};

use crate::cli_support::{
    display_path, format_parse_errors, format_program_mode, resolve_root, write_stdout,
};

#[derive(Debug, Args)]
pub struct CmdApply {
    /// Read the edit program from PATH, or from stdin when PATH is `-`.
    #[arg(
        short = 'f',
        long = "file",
        value_name = "PATH",
        conflicts_with = "operations"
    )]
    pub file: Option<PathBuf>,

    /// Inline operation fragments; use semicolons between multiple statements.
    #[arg(value_name = "OPERATION", conflicts_with = "file")]
    pub operations: Vec<String>,

    /// Resolve relative edit paths from DIR. This changes the working directory; it is not a sandbox.
    #[arg(short = 'r', long = "root", value_name = "DIR")]
    pub root: Option<PathBuf>,

    /// Plan and print filesystem actions without applying them.
    #[arg(long)]
    pub dry_run: bool,

    /// Make each modification observe the result of the preceding modification.
    #[arg(long)]
    pub incremental: bool,
}

impl CmdApply {
    pub fn run(&self) -> Result<(), String> {
        let current_dir =
            env::current_dir().map_err(|error| format!("failed to get cwd: {error}"))?;
        let root = resolve_root(self.root.as_deref(), &current_dir);
        let (input, source_name) = read_program_input(self.file.as_deref(), &self.operations)?;

        let mut program = parse_edit_program(&input)
            .map_err(|errors| format_parse_errors(&source_name, &input, &errors))?;
        validate_parsed_program(&program, &source_name)?;
        if self.incremental {
            program = program.with_mode(ProgramMode::Incremental);
        }

        env::set_current_dir(&root).map_err(|error| {
            format!("failed to change directory to {}: {error}", root.display())
        })?;

        let executor = Executor::new();
        let plan = executor
            .run(
                &program,
                ExecutionOptions {
                    dry_run: self.dry_run,
                },
            )
            .map_err(|error| format!("execution failed: {error}"))?;

        let output = if self.dry_run {
            format_dry_run(&program, &plan, &root)
        } else {
            format!(
                "Applied {} modification(s) across {} stage(s) in {} mode.",
                program.modification_count(),
                program.stages().len(),
                format_program_mode(program.mode)
            )
        };

        write_stdout(&format!("{output}\n"))
    }
}

fn read_program_input(
    file: Option<&Path>,
    operations: &[String],
) -> Result<(String, String), String> {
    if file.is_some() && !operations.is_empty() {
        return Err("--file cannot be combined with inline operations".to_owned());
    }

    match file {
        Some(path) => {
            if path != Path::new("-") {
                fs::read_to_string(path)
                    .map_err(|error| format!("failed to read {}: {error}", path.display()))
                    .and_then(|input| validate_program_input(input, path.display().to_string()))
            } else if !io::stdin().is_terminal() {
                let mut input = String::new();
                io::stdin()
                    .read_to_string(&mut input)
                    .map_err(|error| format!("failed to read stdin: {error}"))?;
                validate_program_input(input, "<stdin>".to_owned())
            } else {
                Err("`-f -` was requested but stdin is not piped".to_owned())
            }
        }
        None if !operations.is_empty() => {
            build_inline_program_input(operations).map(|input| (input, "<args>".to_owned()))
        }
        None if !io::stdin().is_terminal() => {
            let mut input = String::new();
            io::stdin()
                .read_to_string(&mut input)
                .map_err(|error| format!("failed to read stdin: {error}"))?;
            validate_program_input(input, "<stdin>".to_owned())
        }
        None => Err("no input file, inline operations, or stdin input provided".to_owned()),
    }
}

fn validate_program_input(input: String, source_name: String) -> Result<(String, String), String> {
    if input.trim().is_empty() {
        return Err(format!("edit program from {source_name} is empty"));
    }
    Ok((input, source_name))
}

fn validate_parsed_program(program: &EditProgram, source_name: &str) -> Result<(), String> {
    if program.modification_count() == 0 {
        return Err(format!(
            "edit program from {source_name} contains no modifications"
        ));
    }
    Ok(())
}

fn build_inline_program_input(operations: &[String]) -> Result<String, String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum LexicalState {
        Normal,
        String,
        Regex,
        Comment,
    }

    let source = operations.join(" ");
    let mut statements = Vec::new();
    let mut statement = String::new();
    let mut state = LexicalState::Normal;
    let mut characters = source.chars().peekable();

    while let Some(character) = characters.next() {
        match state {
            LexicalState::Normal => match character {
                ';' => {
                    if !statement.trim().is_empty() {
                        statements.push(statement.trim().to_owned());
                    }
                    statement.clear();
                }
                '#' => {
                    statement.push(character);
                    state = LexicalState::Comment;
                }
                'r' if characters.peek() == Some(&'"') => {
                    statement.push(character);
                    statement.push(characters.next().expect("peeked quote must be present"));
                    state = LexicalState::Regex;
                }
                '"' => {
                    statement.push(character);
                    state = LexicalState::String;
                }
                _ => statement.push(character),
            },
            LexicalState::String => {
                statement.push(character);
                if character == '\\' {
                    if let Some(escaped) = characters.next() {
                        statement.push(escaped);
                    }
                } else if character == '"' {
                    state = LexicalState::Normal;
                }
            }
            LexicalState::Regex => {
                statement.push(character);
                if character == '\\' && characters.peek() == Some(&'"') {
                    statement.push(characters.next().expect("peeked quote must be present"));
                } else if character == '"' {
                    state = LexicalState::Normal;
                }
            }
            LexicalState::Comment => {
                if character == ';' {
                    if !statement.trim().is_empty() {
                        statements.push(statement.trim().to_owned());
                    }
                    statement.clear();
                    state = LexicalState::Normal;
                } else {
                    statement.push(character);
                    if character == '\n' {
                        state = LexicalState::Normal;
                    }
                }
            }
        }
    }

    if !statement.trim().is_empty() {
        statements.push(statement.trim().to_owned());
    }

    if statements.is_empty() {
        return Err("no inline operations were provided".to_owned());
    }

    Ok(statements.join("\n"))
}

fn format_dry_run(program: &EditProgram, plan: &EvaluationPlan, root: &Path) -> String {
    let mut output = String::new();
    writeln!(output, "Dry run").expect("writing to a string cannot fail");
    writeln!(output, "Mode: {}", format_program_mode(program.mode))
        .expect("writing to a string cannot fail");
    writeln!(output, "Stages: {}", program.stages().len())
        .expect("writing to a string cannot fail");
    writeln!(output, "Modifications: {}", program.modification_count())
        .expect("writing to a string cannot fail");
    write!(output, "Actions: {}", plan.actions().count()).expect("writing to a string cannot fail");

    let mut modification_index = 0usize;
    for (stage_index, stage) in program.stages().iter().enumerate() {
        write!(output, "\n\nStage {}", stage_index + 1).expect("writing to a string cannot fail");

        for _ in stage.modifications() {
            let modification_plan = &plan.modification_plans()[modification_index];
            write!(
                output,
                "\n  Modification {}: {} action(s)",
                modification_index + 1,
                modification_plan.actions().len()
            )
            .expect("writing to a string cannot fail");

            if modification_plan.actions().is_empty() {
                write!(output, "\n  - no filesystem actions")
                    .expect("writing to a string cannot fail");
            } else {
                for action in modification_plan.actions() {
                    write!(output, "\n  - {}", format_action(action, root))
                        .expect("writing to a string cannot fail");
                }
            }

            modification_index += 1;
        }
    }

    output
}

fn format_action(action: &PlannedAction, root: &Path) -> String {
    match action {
        PlannedAction::CreateDirectory { path } => {
            format!("create directory `{}`", display_path(path, root))
        }
        PlannedAction::WriteFile {
            path,
            bytes,
            overwrite,
            ..
        } => format!(
            "{} file `{}` ({} bytes)",
            if *overwrite { "write" } else { "create" },
            display_path(path, root),
            bytes.len()
        ),
        PlannedAction::DeleteFile { path, .. } => {
            format!("delete file `{}`", display_path(path, root))
        }
        PlannedAction::MoveFile {
            source,
            destination,
            ..
        } => format!(
            "move file `{}` to `{}`",
            display_path(source, root),
            display_path(destination, root)
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::{CommandFactory, Parser, error::ErrorKind};

    use super::CmdApply;
    use super::{
        build_inline_program_input, read_program_input, validate_parsed_program,
        validate_program_input,
    };

    #[derive(Debug, Parser)]
    struct ApplyTestCli {
        #[command(flatten)]
        apply: CmdApply,
    }

    #[test]
    fn builds_inline_program_from_positional_arguments() {
        let input = build_inline_program_input(&[
            "ld".to_owned(),
            "a.txt:1-3".to_owned(),
            ";".to_owned(),
            "apply;".to_owned(),
            "li".to_owned(),
            "b.txt:2".to_owned(),
            "\"hello\"".to_owned(),
        ])
        .unwrap();

        assert_eq!(input, "ld a.txt:1-3\napply\nli b.txt:2 \"hello\"");
    }

    #[test]
    fn splits_semicolons_embedded_in_arguments() {
        let input = build_inline_program_input(&[
            "mode".to_owned(),
            "incremental;ld".to_owned(),
            "a.txt:1-3;apply;".to_owned(),
            "r".to_owned(),
            "tmp.txt".to_owned(),
        ])
        .unwrap();

        assert_eq!(input, "mode incremental\nld a.txt:1-3\napply\nr tmp.txt");
    }

    #[test]
    fn preserves_semicolons_inside_strings_and_regexes() {
        let input = build_inline_program_input(&[
            r#"li a.txt:0 "x;y\n"; tr a.txt "x;y" "a;b";"#.to_owned(),
            r#"ldm a.txt r"^x;y$"; r r"src/(a;b)\.rs""#.to_owned(),
        ])
        .unwrap();

        assert_eq!(
            input,
            concat!(
                "li a.txt:0 \"x;y\\n\"\n",
                "tr a.txt \"x;y\" \"a;b\"\n",
                "ldm a.txt r\"^x;y$\"\n",
                "r r\"src/(a;b)\\.rs\"",
            )
        );
    }

    #[test]
    fn preserves_semicolons_around_escaped_quotes() {
        let input =
            build_inline_program_input(&[r#"tr a.txt "before;\"after" "x;\"y"; apply"#.to_owned()])
                .unwrap();

        assert_eq!(input, "tr a.txt \"before;\\\"after\" \"x;\\\"y\"\napply");
    }

    #[test]
    fn semicolon_after_an_inline_comment_starts_the_next_statement() {
        let input =
            build_inline_program_input(&["ld a.txt:0-1 # remove first; r b.txt".to_owned()])
                .unwrap();

        assert_eq!(input, "ld a.txt:0-1 # remove first\nr b.txt");
    }

    #[test]
    fn rejects_empty_inline_programs() {
        let error = build_inline_program_input(&[";".to_owned(), " ; ".to_owned()]).unwrap_err();
        assert_eq!(error, "no inline operations were provided");
    }

    #[test]
    fn rejects_an_empty_file_or_stdin_program() {
        let error =
            validate_program_input(" \r\n\t".to_owned(), "plan.smedit".to_owned()).unwrap_err();

        assert_eq!(error, "edit program from plan.smedit is empty");
    }

    #[test]
    fn rejects_comment_or_directive_only_programs_after_parsing() {
        for input in ["# generated, but empty", "mode incremental", "apply"] {
            let program = smartedit::parse_edit_program(input).unwrap();
            let error = validate_parsed_program(&program, "plan.smedit").unwrap_err();
            assert_eq!(
                error,
                "edit program from plan.smedit contains no modifications"
            );
        }
    }

    #[test]
    fn rejects_file_and_inline_input_together() {
        let error = read_program_input(Some(Path::new("plan.smedit")), &["r a.txt".to_owned()])
            .unwrap_err();

        assert_eq!(error, "--file cannot be combined with inline operations");
    }

    #[test]
    fn clap_rejects_file_and_inline_input_together() {
        let error =
            ApplyTestCli::try_parse_from(["smartedit-apply", "--file", "plan.smedit", "r a.txt"])
                .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn help_explains_apply_input_and_execution_options() {
        let help = ApplyTestCli::command().render_long_help().to_string();

        assert!(help.contains("semicolons between multiple statements"));
        assert!(help.contains("not a sandbox"));
        assert!(help.contains("without applying them"));
        assert!(help.contains("observe the result of the preceding modification"));
    }
}
