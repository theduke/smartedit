use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};

use clap::Args;
use smartedit::{
    EditProgram, EvaluationPlan, ExecutionOptions, Executor, PlannedAction, ProgramMode,
    parse_edit_program,
};

use crate::cli_support::{display_path, format_parse_errors, format_program_mode, resolve_root};

#[derive(Debug, Args)]
pub struct CmdApply {
    #[arg(short = 'f', long = "file")]
    pub file: Option<PathBuf>,

    #[arg(value_name = "OPERATION")]
    pub operations: Vec<String>,

    #[arg(short = 'r', long = "root")]
    pub root: Option<PathBuf>,

    #[arg(long)]
    pub dry_run: bool,

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
                    ..ExecutionOptions::default()
                },
            )
            .map_err(|error| format!("execution failed: {error}"))?;

        if self.dry_run {
            print_dry_run(&program, &plan, &root);
        } else {
            println!(
                "Applied {} modification(s) across {} stage(s) in {} mode.",
                program.modification_count(),
                program.stages().len(),
                format_program_mode(program.mode)
            );
        }

        Ok(())
    }
}

fn read_program_input(
    file: Option<&Path>,
    operations: &[String],
) -> Result<(String, String), String> {
    match file {
        Some(path) => {
            if path != Path::new("-") {
                fs::read_to_string(path)
                    .map(|input| (input, path.display().to_string()))
                    .map_err(|error| format!("failed to read {}: {error}", path.display()))
            } else if !io::stdin().is_terminal() {
                let mut input = String::new();
                io::stdin()
                    .read_to_string(&mut input)
                    .map_err(|error| format!("failed to read stdin: {error}"))?;
                Ok((input, "<stdin>".to_owned()))
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
            Ok((input, "<stdin>".to_owned()))
        }
        None => Err("no input file, inline operations, or stdin input provided".to_owned()),
    }
}

fn build_inline_program_input(operations: &[String]) -> Result<String, String> {
    let statements: Vec<String> = operations
        .join(" ")
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(str::to_owned)
        .collect();

    if statements.is_empty() {
        return Err("no inline operations were provided".to_owned());
    }

    Ok(statements.join("\n"))
}

fn print_dry_run(program: &EditProgram, plan: &EvaluationPlan, root: &Path) {
    println!("Dry run");
    println!("Mode: {}", format_program_mode(program.mode));
    println!("Stages: {}", program.stages().len());
    println!("Modifications: {}", program.modification_count());
    println!("Actions: {}", plan.actions().count());

    let mut modification_index = 0usize;
    for (stage_index, stage) in program.stages().iter().enumerate() {
        println!();
        println!("Stage {}", stage_index + 1);

        for _ in stage.modifications() {
            let modification_plan = &plan.modification_plans()[modification_index];
            println!(
                "  Modification {}: {} action(s)",
                modification_index + 1,
                modification_plan.actions().len()
            );

            if modification_plan.actions().is_empty() {
                println!("  - no filesystem actions");
            } else {
                for action in modification_plan.actions() {
                    println!("  - {}", format_action(action, root));
                }
            }

            modification_index += 1;
        }
    }
}

fn format_action(action: &PlannedAction, root: &Path) -> String {
    match action {
        PlannedAction::CreateDirectory { path } => {
            format!("create directory `{}`", display_path(path, root))
        }
        PlannedAction::WriteFile { path, bytes } => format!(
            "write file `{}` ({} bytes)",
            display_path(path, root),
            bytes.len()
        ),
        PlannedAction::DeleteFile { path, .. } => {
            format!("delete file `{}`", display_path(path, root))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_inline_program_input;

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
    fn rejects_empty_inline_programs() {
        let error = build_inline_program_input(&[";".to_owned(), " ; ".to_owned()]).unwrap_err();
        assert_eq!(error, "no inline operations were provided");
    }
}
