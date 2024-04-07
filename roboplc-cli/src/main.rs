use core::fmt;
use std::{
    env, fs,
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

const DEFAULT_TIMEOUT: u64 = 60;

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
#[clap(author = "Bohemia Automation (https://bma.ai)",
    version = env!("CARGO_PKG_VERSION"),
    about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[clap(short = 'T', long, help = "Manager API timeout")]
    timeout: Option<u64>,
    #[clap(short = 'U', long, env = "ROBOPLC_URL", help = "Manager URL")]
    url: Option<String>,
    #[clap(short = 'k', long, env = "ROBOPLC_KEY", help = "Management key")]
    key: Option<String>,
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
    #[clap(name = "purge", about = "Purge program data directory")]
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
        .post(&format!("{}{}/purge.program.data", url, API_PREFIX))
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
    robo_toml: Option<toml::Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(file) = opts.file {
        flash_file(url, key, agent, file, opts.force, opts.run)?;
    } else {
        let mut cargo_target: Option<String> = None;
        if let Some(c) = opts.cargo_target {
            cargo_target.replace(c);
        }
        if cargo_target.is_none() {
            if let Some(ref value) = robo_toml {
                cargo_target = value
                    .get("build")
                    .and_then(|v| v.get("target"))
                    .map(|v| v.as_str().unwrap().to_owned());
            }
        }
        if cargo_target.is_none() {
            let resp = agent
                .post(&format!("{}{}/query.info.kernel", url, API_PREFIX))
                .set("x-auth-key", key)
                .call()?;
            let info: KernelInfo = resp.into_json()?;
            cargo_target.replace(format!("{}-unknown-linux-gnu", info.machine));
        }
        let mut cargo: Option<PathBuf> = None;
        if let Some(c) = opts.cargo {
            cargo.replace(c);
        }
        if cargo.is_none() {
            if let Some(ref value) = robo_toml {
                cargo = value
                    .get("build")
                    .and_then(|v| v.get("cargo"))
                    .map(|v| v.as_str().unwrap().into());
            }
        }
        if cargo.is_none() {
            cargo = which("cross").ok();
        }
        let cargo_target = cargo_target.unwrap();
        let cargo = cargo.unwrap_or_else(|| "cargo".into());
        let Some(name) = find_name_and_chdir() else {
            return Err("Could not find Cargo.toml/binary name".into());
        };
        let binary_name = Path::new("target")
            .join(&cargo_target)
            .join("release")
            .join(name);
        println!("Remote: {}", url.yellow());
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
            let contents = fs::read_to_string(cargo_toml_path).ok()?;
            let value = contents.parse::<toml::Value>().ok()?;
            env::set_current_dir(current_dir).ok()?;
            return value["package"]["name"].as_str().map(String::from);
        }
        if !current_dir.pop() {
            break;
        }
    }
    None
}

fn find_roboplc_toml() -> Option<PathBuf> {
    let mut current_dir = env::current_dir().ok()?;
    loop {
        let mut cargo_toml_path = current_dir.clone();
        cargo_toml_path.push("Cargo.toml");
        if cargo_toml_path.exists() {
            let mut roboplc_toml_path = current_dir.clone();
            roboplc_toml_path.push("robo.toml");
            if roboplc_toml_path.exists() {
                return Some(roboplc_toml_path);
            }
        }
        if !current_dir.pop() {
            break;
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut url = args.url;
    let mut key = args.key;
    let mut timeout = args.timeout;
    let mut robo_toml: Option<toml::Value> = None;
    if let Some(roboplc_toml_path) = find_roboplc_toml() {
        let contents = fs::read_to_string(roboplc_toml_path)?;
        let value = contents.parse::<toml::Value>()?;
        if url.is_none() {
            url = value
                .get("remote")
                .and_then(|v| v.get("url"))
                .and_then(|v| v.as_str())
                .map(String::from);
        }
        if key.is_none() {
            key = value
                .get("remote")
                .and_then(|v| v.get("key"))
                .and_then(|v| v.as_str())
                .map(String::from);
        }
        if timeout.is_none() {
            let toml_timeout = value.get("remote").and_then(|v| v.get("timeout"));
            if let Some(t) = toml_timeout {
                timeout = Some(u64::try_from(
                    t.as_integer().ok_or("Invalid timeout (must be integer)")?,
                )?);
            }
        }
        robo_toml.replace(value);
    }
    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let url_s = url.ok_or("URL not specified")?;
    let url = url_s.trim_end_matches('/');
    let key = key.ok_or("Key not specified")?;
    let agent: Agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(timeout))
        .timeout_write(Duration::from_secs(timeout))
        .build();
    match args.subcmd {
        SubCommand::Stat => {
            stat_command(url, &key, agent)?;
        }
        SubCommand::Config => {
            set_mode_command(url, &key, agent, Mode::Config)?;
        }
        SubCommand::Run => {
            set_mode_command(url, &key, agent, Mode::Run)?;
        }
        SubCommand::Flash(opts) => {
            flash(url, &key, agent, opts, robo_toml)?;
        }
        SubCommand::Purge => {
            purge_command(url, &key, agent)?;
        }
    }
    Ok(())
}
