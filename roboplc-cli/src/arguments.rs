use std::path::PathBuf;

use clap::Parser;

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
    #[clap(name = "flash", about = "Flash program")]
    Flash(FlashCommand),
    #[clap(name = "purge", about = "Purge program data directory")]
    Purge,
}

#[derive(Parser)]
pub struct NewCommand {
    #[clap(help = "Project name")]
    pub name: String,
    #[clap(long, help = "RoboPLC crate features")]
    pub features: Vec<String>,
    #[clap(last(true), help = "extra cargo arguments")]
    pub extras: Vec<String>,
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
