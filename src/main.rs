mod cli_support;
mod cmd;

use clap::{Parser, Subcommand};

use crate::cmd::{apply::CmdApply, ast_print::CmdAstPrint, install_skill::CmdInstallSkill};

#[derive(Debug, Parser)]
#[command(name = "smartedit")]
#[command(about = "Apply smartedit programs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Apply(CmdApply),
    AstPrint(CmdAstPrint),
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
