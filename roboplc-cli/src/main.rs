use core::fmt;
use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Parser;
use colored::Colorize as _;
use roboplc::thread_rt::Scheduling;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ureq::Agent;
use ureq_multipart::MultipartBuilder;
use which::which;

const API_PREFIX: &str = "/roboplc/api";

#[derive(Debug, Serialize, Deserialize)]
pub struct State {
    pid: Option<u32>,
    mode: Mode,
    memory_used: Option<u64>,
    run_time: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Run,
    Config,
    Unknown,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Mode::Run => write!(f, "RUN"),
            Mode::Config => write!(f, "CONFIG"),
            Mode::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Task {
    name: String,
    pid: u32,
    cpu: i32,
    cpu_usage: f32,
    sched: Scheduling,
    priority: i32,
}

#[derive(Parser)]
struct Args {
    #[clap(short = 'T', long, default_value = "60", help = "Manager API timeout")]
    timeout: f64,
    #[clap(short = 'U', long, env = "ROBOPLC_URL", help = "Manager URL")]
    url: String,
    #[clap(short = 'k', long, env = "ROBOPLC_KEY", help = "Management key")]
    key: String,
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Parser)]
enum SubCommand {
    #[clap(name = "stat", about = "Get program status")]
    Stat,
    #[clap(name = "config", about = "Switch remote into CONFIG mode")]
    Config,
    #[clap(name = "run", about = "Switch remote into RUN mode")]
    Run,
    #[clap(name = "flash", about = "Flash program")]
    Flash(FlashCommand),
    #[clap(name = "purge", about = "Purge var directory")]
    Purge,
}

#[derive(Parser)]
struct FlashCommand {
    #[clap(long, env = "CARGO", help = "cargo/cross binary path")]
    cargo: Option<PathBuf>,
    #[clap(long, help = "Override remote cargo target")]
    cargo_target: Option<String>,
    #[clap(long, help = "Do not compile a Rust project, use a file instead")]
    file: Option<PathBuf>,
    #[clap(long, help = "Force flash (automatically put remote in CONFIG mode)")]
    force: bool,
    #[clap(long, help = "Put remote in RUN mode after flashing")]
    run: bool,
}

fn stat_command(url: &str, key: &str, agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    let resp = agent
        .post(&format!("{}{}/query.stats.program", url, API_PREFIX))
        .set("x-auth-key", key)
        .call()
        .process_error()?;
    let stats: State = resp.into_json()?;
    let mode_colored = match stats.mode {
        Mode::Run => format!("{}", stats.mode).green(),
        Mode::Config => format!("{}", stats.mode).yellow(),
        Mode::Unknown => format!("{}", stats.mode).red(),
    };
    println!("Mode {}", mode_colored);
    if let Some(pid) = stats.pid {
        println!("PID  {}", pid);
    }
    if let Some(memory) = stats.memory_used {
        println!("Mem  {}", memory);
    }
    if let Some(run_time) = stats.run_time {
        println!("Up   {}", run_time);
    }
    Ok(())
}

macro_rules! ok {
    () => {
        println!("{}", "OK".green());
    };
}

fn set_mode_command(
    url: &str,
    key: &str,
    agent: Agent,
    mode: Mode,
) -> Result<(), Box<dyn std::error::Error>> {
    agent
        .post(&format!("{}{}/set.program.mode", url, API_PREFIX))
        .set("x-auth-key", key)
        .send_json(ureq::json!({
             "mode": mode,
        }))
        .process_error()?;
    ok!();
    Ok(())
}

fn purge_command(url: &str, key: &str, agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    agent
        .post(&format!("{}{}/purge.var", url, API_PREFIX))
        .set("x-auth-key", key)
        .call()
        .process_error()?;
    ok!();
    Ok(())
}

#[derive(Deserialize)]
struct KernelInfo {
    machine: String,
}

trait PrintErr<T> {
    fn process_error(self) -> Result<T, Box<dyn std::error::Error>>;
}

impl<T> PrintErr<T> for Result<T, ureq::Error> {
    fn process_error(self) -> Result<T, Box<dyn std::error::Error>> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => match e.kind() {
                ureq::ErrorKind::HTTP => {
                    let response = e.into_response().unwrap();
                    let status = response.status();
                    let msg = format!(
                        "{} ({})",
                        response.into_string().unwrap_or_default(),
                        status
                    );
                    eprintln!("{}: {}", "Error".red(), msg);
                    Err("Remote".into())
                }
                _ => Err(e.into()),
            },
        }
    }
}

fn flash_file(
    url: &str,
    key: &str,
    agent: Agent,
    file: PathBuf,
    force: bool,
    run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (content_type, data) = MultipartBuilder::new()
        .add_file("file", file)?
        .add_text(
            "params",
            &serde_json::to_string(&json! {
                {
                    "force": force,
                    "run": run,
                }

            })?,
        )?
        .finish()?;
    agent
        .post(&format!("{}{}/flash", url, API_PREFIX))
        .set("x-auth-key", key)
        .set("content-type", &content_type)
        .send_bytes(&data)
        .process_error()?;
    Ok(())
}

fn flash(
    url: &str,
    key: &str,
    agent: Agent,
    opts: FlashCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(file) = opts.file {
        flash_file(url, key, agent, file, opts.force, opts.run)?;
    } else {
        let cargo_target = if let Some(cargo_target) = opts.cargo_target {
            cargo_target
        } else {
            let resp = agent
                .post(&format!("{}{}/query.info.kernel", url, API_PREFIX))
                .set("x-auth-key", key)
                .call()?;
            let info: KernelInfo = resp.into_json()?;
            format!("{}-unknown-linux-gnu", info.machine)
        };
        let cargo = opts
            .cargo
            .unwrap_or_else(|| which("cross").unwrap_or_else(|_| Path::new("cargo").to_owned()));
        let Some(name) = find_name_and_chdir() else {
            return Err("Could not find Cross.toml/binary name".into());
        };
        let binary_name = Path::new("target")
            .join(&cargo_target)
            .join("release")
            .join(name);
        println!("Machine: {}", url.yellow());
        println!("Cargo: {}", cargo.display().to_string().yellow());
        println!("Cargo target: {}", cargo_target.yellow());
        println!("Binary: {}", binary_name.display().to_string().yellow());
        println!("Compiling...");
        let result = std::process::Command::new(cargo)
            .arg("build")
            .arg("--release")
            .arg("--target")
            .arg(cargo_target)
            .status()?;
        if !result.success() {
            return Err("Compilation failed".into());
        }
        println!("Flashing...");
        flash_file(url, key, agent, binary_name, opts.force, opts.run)?;
    }
    ok!();
    Ok(())
}

fn find_name_and_chdir() -> Option<String> {
    let mut current_dir = env::current_dir().ok()?;
    loop {
        let mut cargo_toml_path = current_dir.clone();
        cargo_toml_path.push("Cargo.toml");
        if cargo_toml_path.exists() {
            let contents = std::fs::read_to_string(cargo_toml_path).ok()?;
            let value = contents.parse::<toml::Value>().ok()?;
            env::set_current_dir(current_dir).ok()?;
            return value["package"]["name"].as_str().map(String::from);
        } else if !current_dir.pop() {
            break;
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let agent: Agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs_f64(args.timeout))
        .timeout_write(Duration::from_secs_f64(args.timeout))
        .build();
    match args.subcmd {
        SubCommand::Stat => {
            stat_command(&args.url, &args.key, agent)?;
        }
        SubCommand::Config => {
            set_mode_command(&args.url, &args.key, agent, Mode::Config)?;
        }
        SubCommand::Run => {
            set_mode_command(&args.url, &args.key, agent, Mode::Run)?;
        }
        SubCommand::Flash(opts) => {
            flash(&args.url, &args.key, agent, opts)?;
        }
        SubCommand::Purge => {
            purge_command(&args.url, &args.key, agent)?;
        }
    }
    Ok(())
}
