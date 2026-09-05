mod cli_support;
mod cmd;

use clap::{Parser, Subcommand};

use crate::cmd::{apply::CmdApply, ast_print::CmdAstPrint, install_skill::CmdInstallSkill};

#[derive(Debug, Parser)]
#[command(name = "smartedit")]
#[command(version)]
#[command(about = "Inspect source structure and apply compact, deterministic edits")]
#[command(
    long_about = "Inspect structure for 19 languages including Rust, Python, JS/TS, Go, Java, C/C++, C#, Ruby, PHP; apply compact text and file edit programs; or install the bundled agent skill.\n\nRun `smartedit <COMMAND> --help` for command-specific options and examples.",
    after_help = "Examples:\n  smartedit ast-print --loc src/main.rs\n  smartedit apply 'li notes.txt:0 \"heading\\n\"'\n  smartedit install-skill --repo"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply a compact edit program to files.
    #[command(
        long_about = "Apply a compact edit program supplied as inline operations, a .smedit file, or standard input.\n\nLine ranges are zero-based and half-open. By default a stage reads from one snapshot; use --incremental when later operations should observe earlier ones."
    )]
    Apply(CmdApply),

    /// Print a structural outline of supported source files.
    #[command(
        long_about = "Print a structural outline of source files in 19 supported languages. Inputs may be paths or globs; selectors narrow output to matching items."
    )]
    AstPrint(CmdAstPrint),

    /// Install the bundled smartedit agent skill.
    #[command(
        long_about = "Install the bundled smartedit agent skill below a repository, a directory, or your user home.\n\nThe skill is written to .agents/skills/smartedit/SKILL.md below the selected root. The installed skill invokes `smartedit`, so the executable must remain available on PATH."
    )]
    InstallSkill(CmdInstallSkill),
}

impl Command {
    fn run(&self) -> Result<(), String> {
        match self {
            Self::Apply(cmd) => cmd.run(),
            Self::AstPrint(cmd) => cmd.run(),
            Self::InstallSkill(cmd) => cmd.run(),
        }
    }
}

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    cli.command.run()
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser, error::ErrorKind};

    use super::Cli;

    #[test]
    fn top_level_help_describes_each_command() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("Inspect structure for 19 languages"));
        assert!(help.contains("Apply a compact edit program to files"));
        assert!(help.contains("Print a structural outline"));
        assert!(help.contains("Install the bundled smartedit agent skill"));
        assert!(help.contains("smartedit ast-print --loc src/main.rs"));
    }

    #[test]
    fn install_skill_help_explains_targets_layout_and_examples() {
        let error = Cli::try_parse_from(["smartedit", "install-skill", "--help"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        let help = error.to_string();

        assert!(help.contains(".agents/skills/smartedit/SKILL.md"));
        assert!(help.contains("--repo"));
        assert!(help.contains("nearest Git repository"));
        assert!(help.contains("--user"));
        assert!(help.contains("cross-platform home directory"));
        assert!(help.contains("--dir"));
        assert!(help.contains("[default: current directory]"));
        assert!(help.contains("must remain available on PATH"));
        assert!(help.contains("smartedit install-skill --repo path/to/checkout"));
    }

    #[test]
    fn ast_print_help_warns_about_shared_line_ranges() {
        let error = Cli::try_parse_from(["smartedit", "ast-print", "--help"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        let help = error.to_string();

        assert!(help.contains("zero-based, half-open line ranges"));
        assert!(help.contains("shared lines are marked unsafe"));
        assert!(help.contains("source files in 19 supported languages"));
    }
}
