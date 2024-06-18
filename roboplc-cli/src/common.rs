use core::fmt;
use std::{env, path::PathBuf};

use colored::Colorize;
use serde::{Deserialize, Serialize};

pub const CONFIG_FILE_NAME: &str = "robo.toml";
pub const GLOBAL_CONFIG_FILE_NAME: &str = ".robo-global.toml";

pub fn print_err(msg: &str) {
    eprintln!("{}", msg.red());
}

#[derive(Debug, Serialize, Deserialize)]
pub struct State {
    pid: Option<u32>,
    mode: Mode,
    memory_used: Option<u64>,
    run_time: Option<u64>,
}

impl State {
    pub fn print_std(&self) {
        let mode_colored = match self.mode {
            Mode::Run => format!("{}", self.mode).green(),
            Mode::Config => format!("{}", self.mode).yellow(),
            Mode::Unknown => format!("{}", self.mode).red(),
        };
        println!("Mode {}", mode_colored);
        if let Some(pid) = self.pid {
            println!("PID  {}", pid);
        }
        if let Some(memory) = self.memory_used {
            println!("Mem  {}", memory);
        }
        if let Some(run_time) = self.run_time {
            println!("Up   {}", run_time);
        }
    }
}

#[derive(Deserialize)]
pub struct KernelInfo {
    machine: String,
}

impl KernelInfo {
    pub fn to_machine_cargo_target(&self) -> String {
        format!("{}-unknown-linux-gnu", self.machine)
    }
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

pub fn find_robo_toml() -> Option<PathBuf> {
    let mut current_dir = env::current_dir().ok()?;
    loop {
        let mut cargo_toml_path = current_dir.clone();
        cargo_toml_path.push("Cargo.toml");
        if cargo_toml_path.exists() {
            let mut roboplc_toml_path = current_dir.clone();
            roboplc_toml_path.push(CONFIG_FILE_NAME);
            if roboplc_toml_path.exists() {
                return Some(roboplc_toml_path);
            }
        }
        if !current_dir.pop() {
            let local_path = PathBuf::from(CONFIG_FILE_NAME);
            if local_path.exists() {
                return Some(local_path);
            }
            break;
        }
    }
    None
}

#[allow(clippy::unnecessary_wraps)]
pub fn report_ok() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "OK".green());
    Ok(())
}
