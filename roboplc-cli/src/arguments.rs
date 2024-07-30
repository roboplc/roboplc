use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Parser)]
#[clap(author = "Bohemia Automation (https://bma.ai)",
    version = env!("CARGO_PKG_VERSION"),
    about = env!("CARGO_PKG_DESCRIPTION"))]
pub struct Args {
    #[clap(short = 'T', long, help = "Manager API timeout")]
    pub timeout: Option<u64>,
    #[clap(
        short = 'U',
        long,
        env = "ROBOPLC_URL",
        help = "Manager URL or a system"
    )]
    pub url: Option<String>,
    #[clap(
        short = 'k',
        long,
        env = "ROBOPLC_KEY",
        help = "Management key, if required"
    )]
    pub key: Option<String>,
    #[clap(subcommand)]
    pub subcmd: SubCommand,
}

#[derive(Parser)]
pub enum SubCommand {
    #[clap(name = "new", about = "Generate a new project")]
    New(NewCommand),
    #[clap(name = "stat", about = "Get program status")]
    Stat,
    #[clap(name = "config", about = "Switch remote into CONFIG mode")]
    Config,
    #[clap(name = "run", about = "Switch remote into RUN mode")]
    Run,
    #[clap(
        name = "restart",
        about = "Restart program (switch to CONFIG and back to RUN)"
    )]
    Restart,
    #[clap(name = "flash", about = "Flash program")]
    Flash(FlashCommand),
    #[clap(
        name = "x",
        about = "Execute program on the remote host in a virtual terminal"
    )]
    Exec(ExecCommand),
    #[clap(name = "purge", about = "Purge program data directory")]
    Purge,
}

#[derive(Parser)]
pub struct NewCommand {
    #[clap(help = "Project name")]
    pub name: String,
    #[clap(short = 'F', long, help = "RoboPLC crate features")]
    pub features: Vec<String>,
    #[clap(last(true), help = "extra cargo arguments")]
    pub extras: Vec<String>,
    #[clap(short = 'L', long, help = "Locking policy)", default_value = "rt-safe")]
    pub locking: LockingPolicy,
}

#[derive(ValueEnum, Copy, Clone)]
pub enum LockingPolicy {
    #[clap(name = "default", help = "Default locking policy")]
    Default,
    #[clap(name = "rt", help = "Real-time locking policy")]
    Rt,
    #[clap(name = "rt-safe", help = "Real-time safe locking policy")]
    RtSafe,
}

impl LockingPolicy {
    pub fn as_feature_str(self) -> &'static str {
        match self {
            LockingPolicy::Default => "locking-default",
            LockingPolicy::Rt => "locking-rt",
            LockingPolicy::RtSafe => "locking-rt-safe",
        }
    }
}

#[derive(Parser)]
pub struct FlashCommand {
    #[clap(long, env = "CARGO", help = "cargo/cross binary path")]
    pub cargo: Option<PathBuf>,
    #[clap(long, help = "Override remote cargo target")]
    pub cargo_target: Option<String>,
    #[clap(long, help = "Extra cargo arguments")]
    pub cargo_args: Option<String>,
    #[clap(long, help = "Do not compile a Rust project, use a file instead")]
    pub file: Option<PathBuf>,
    #[clap(
        short = 'f',
        long,
        help = "Force flash (automatically put remote in CONFIG mode)"
    )]
    pub force: bool,
    #[clap(short = 'r', long, help = "Put remote in RUN mode after flashing")]
    pub run: bool,
}

#[derive(Parser)]
pub struct ExecCommand {
    #[clap(long, env = "CARGO", help = "cargo/cross binary path")]
    pub cargo: Option<PathBuf>,
    #[clap(long, help = "Override remote cargo target")]
    pub cargo_target: Option<String>,
    #[clap(long, help = "Extra cargo arguments")]
    pub cargo_args: Option<String>,
    #[clap(long, help = "Do not compile a Rust project, use a file instead")]
    pub file: Option<PathBuf>,
    #[clap(
        short = 'f',
        long,
        help = "Force execute (ignore if other program is being executed)"
    )]
    pub force: bool,
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Arguments after -- are passed to the program as-is"
    )]
    pub args: Vec<String>,
}

pub struct FlashExec {
    pub cargo: Option<PathBuf>,
    pub cargo_target: Option<String>,
    pub cargo_args: Option<String>,
    pub file: Option<PathBuf>,
    pub force: bool,
    pub run: bool,
    pub program_args: Vec<String>,
}

impl From<FlashCommand> for FlashExec {
    fn from(cmd: FlashCommand) -> Self {
        Self {
            cargo: cmd.cargo,
            cargo_target: cmd.cargo_target,
            cargo_args: cmd.cargo_args,
            file: cmd.file,
            force: cmd.force,
            run: cmd.run,
            program_args: Vec::new(),
        }
    }
}

impl From<ExecCommand> for FlashExec {
    fn from(cmd: ExecCommand) -> Self {
        Self {
            cargo: cmd.cargo,
            cargo_target: cmd.cargo_target,
            cargo_args: cmd.cargo_args,
            file: cmd.file,
            force: cmd.force,
            run: false,
            program_args: cmd.args,
        }
    }
}
