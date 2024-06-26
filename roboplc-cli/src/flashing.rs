use std::{
    env, fs,
    path::{Path, PathBuf},
};

use colored::Colorize as _;
use serde_json::json;
use ureq::Agent;
use ureq_multipart::MultipartBuilder;
use which::which;

use crate::{
    arguments::FlashCommand,
    common::{report_ok, KernelInfo},
    config,
    ureq_err::PrintErr,
    API_PREFIX,
};

fn flash_file(
    url: &str,
    key: &str,
    agent: Agent,
    file: &Path,
    force: bool,
    run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !file.exists() {
        return Err(format!("File not found: {}", file.display()).into());
    }
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

fn run_build_custom(
    url: &str,
    key: &str,
    agent: Agent,
    force: bool,
    run: bool,
    cmd: &str,
    file: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Remote: {}", url.yellow());
    println!("Build command line: {}", cmd.yellow());
    println!("Binary: {}", file.display().to_string().yellow());
    println!("Compiling...");
    let result = std::process::Command::new("sh")
        .args(["-c", cmd])
        .status()?;
    if !result.success() {
        return Err("Compilation failed".into());
    }
    println!("Flashing...");
    if !file.exists() {
        return Err(format!("File not found: {}", file.display()).into());
    }
    flash_file(url, key, agent, file, force, run)?;
    Ok(())
}

pub fn flash(
    url: &str,
    key: &str,
    agent: Agent,
    opts: FlashCommand,
    build_config: config::Build,
    build_custom: config::BuildCustom,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(file) = opts.file {
        flash_file(url, key, agent, &file, opts.force, opts.run)?;
    } else if let Some(custom_cmd) = build_custom.command {
        run_build_custom(
            url,
            key,
            agent,
            opts.force,
            opts.run,
            &custom_cmd,
            &build_custom
                .file
                .ok_or("Custom build command requires a file")?,
        )?;
    } else {
        let mut cargo_target: Option<String> = None;
        if let Some(c) = opts.cargo_target {
            cargo_target.replace(c);
        }
        if cargo_target.is_none() {
            cargo_target = build_config.target;
        }
        if cargo_target.is_none() {
            let resp = agent
                .post(&format!("{}{}/query.info.kernel", url, API_PREFIX))
                .set("x-auth-key", key)
                .call()?;
            let info: KernelInfo = resp.into_json()?;
            cargo_target.replace(info.to_machine_cargo_target());
        }
        let mut cargo: Option<PathBuf> = None;
        if let Some(c) = opts.cargo {
            cargo.replace(c);
        }
        if cargo.is_none() {
            cargo = build_config.cargo;
        }
        if cargo.is_none() {
            cargo = which("cross").ok();
        }
        let cargo_target = cargo_target.unwrap();
        let cargo = cargo.unwrap_or_else(|| "cargo".into());
        let Some(name) = find_name_and_chdir() else {
            return Err("Could not find Cargo.toml/binary name".into());
        };
        let mut cargo_args = None;
        if let Some(args) = opts.cargo_args {
            cargo_args.replace(args);
        } else {
            cargo_args = build_config.cargo_args;
        }
        let binary_name = Path::new("target")
            .join(&cargo_target)
            .join("release")
            .join(name);
        let mut args: Vec<String> = vec![
            "build".into(),
            "--release".into(),
            "--target".into(),
            cargo_target.clone(),
        ];
        if let Some(extra) = cargo_args {
            args.extend(shlex::split(&extra).expect("Invalid cargo args"));
        }
        println!("Remote: {}", url.yellow());
        println!(
            "Cargo command line: {} {}",
            cargo.display().to_string().yellow(),
            args.join(" ").yellow()
        );
        println!("Cargo target: {}", cargo_target.yellow());
        println!("Binary: {}", binary_name.display().to_string().yellow());
        println!("Compiling...");
        let result = std::process::Command::new(cargo).args(args).status()?;
        if !result.success() {
            return Err("Compilation failed".into());
        }
        println!("Flashing...");
        flash_file(url, key, agent, &binary_name, opts.force, opts.run)?;
    }
    report_ok()
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
