use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use colored::Colorize as _;
use serde::Serialize;
use ureq::Agent;
use ureq_multipart::MultipartBuilder;
use which::which;

use crate::{
    arguments::{FlashExec, RollbackCommand},
    common::{report_ok, KernelInfo},
    config,
    ureq_err::PrintErr,
    API_PREFIX,
};

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn flash_file(
    url: &str,
    key: &str,
    agent: Agent,
    file: &Path,
    force: bool,
    run: bool,
    live: bool,
    skip_backup: bool,
    exec_only: bool,
    program_args: Vec<String>,
    program_env: BTreeMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !file.exists() {
        return Err(format!("File not found: {}", file.display()).into());
    }
    if exec_only {
        return crate::exec::exec(url, key, file, force, program_args, program_env);
    }
    if let Some(docker_img) = url.strip_prefix("docker://") {
        if run {
            return Err("Live update for Docker images is not supported".into());
        }
        let tag = std::env::var("ROBOPLC_DOCKER_TAG").unwrap_or_else(|_| {
            crate::TARGET_PACKAGE_VERSION
                .get()
                .cloned()
                .unwrap_or_else(|| "latest".to_owned())
        });
        let img_name = format!("{}:{}", docker_img, tag);
        println!("Building docker image: {}", img_name.yellow());
        let result = std::process::Command::new("docker")
            .args(["build", "-t", &img_name, "."])
            .status()?;
        if !result.success() {
            return Err("Compilation failed".into());
        }
        println!();
        println!("Docker image ready: {}", img_name.green());
        if run {
            println!("Running docker image...");
            let mut args = vec!["run", "--rm", "-it"];
            let port = std::env::var("ROBOPLC_DOCKER_PORT")
                .unwrap_or_else(|_| "127.0.0.1:7700".to_owned());
            let port_mapping = if port.is_empty() {
                None
            } else {
                Some(format!("{}:7700", port))
            };
            if let Some(ref port_mapping) = port_mapping {
                args.push("-p");
                args.push(port_mapping);
                println!(
                    "RoboPLC manager is available at {}",
                    format!("http://{}", port).yellow()
                );
            }
            if force {
                args.push("--privileged");
            }
            args.push(&img_name);
            let result = std::process::Command::new("docker").args(args).status()?;
            if !result.success() {
                return Err("Execution failed".into());
            }
        }
    } else {
        #[derive(Serialize)]
        struct Payload {
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            force: bool,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            run: bool,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            live: bool,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            skip_backup: bool,
        }
        let (content_type, data) = MultipartBuilder::new()
            .add_file("file", file)?
            .add_text(
                "params",
                &serde_json::to_string(&Payload {
                    force,
                    run,
                    live,
                    skip_backup,
                })?,
            )?
            .finish()?;
        agent
            .post(&format!("{}{}/flash", url, API_PREFIX))
            .set("x-auth-key", key)
            .set("content-type", &content_type)
            .send_bytes(&data)
            .process_error()?;
    }
    Ok(())
}

pub fn rollback(
    url: &str,
    key: &str,
    agent: Agent,
    opts: RollbackCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    #[derive(Serialize)]
    struct Payload {
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        force: bool,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        run: bool,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        live: bool,
    }
    agent
        .post(&format!("{}{}/rollback", url, API_PREFIX))
        .set("x-auth-key", key)
        .send_json(&Payload {
            force: opts.force,
            run: opts.run,
            live: opts.live,
        })
        .process_error()?;
    report_ok()?;
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn run_build_custom(
    url: &str,
    key: &str,
    agent: Agent,
    force: bool,
    run: bool,
    live: bool,
    skip_backup: bool,
    cmd: &str,
    file: &Path,
    exec_only: bool,
    program_args: Vec<String>,
    program_env: BTreeMap<String, String>,
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
    flash_file(
        url,
        key,
        agent,
        file,
        force,
        run,
        live,
        skip_backup,
        exec_only,
        program_args,
        program_env,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn flash(
    url: &str,
    key: &str,
    agent: Agent,
    opts: FlashExec,
    build_config: config::Build,
    build_custom: config::BuildCustom,
    exec_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(file) = opts.file {
        flash_file(
            url,
            key,
            agent,
            &file,
            opts.force,
            opts.run,
            opts.live,
            opts.skip_backup,
            exec_only,
            opts.program_args,
            opts.program_env,
        )?;
    } else if let Some(custom_cmd) = build_custom.command {
        run_build_custom(
            url,
            key,
            agent,
            opts.force,
            opts.run,
            opts.live,
            opts.skip_backup,
            &custom_cmd,
            &build_custom
                .file
                .ok_or("Custom build command requires a file")?,
            exec_only,
            opts.program_args,
            opts.program_env,
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
        let binary_name = Path::new(crate::cargo_target_dir())
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
        flash_file(
            url,
            key,
            agent,
            &binary_name,
            opts.force,
            opts.run,
            opts.live,
            opts.skip_backup,
            exec_only,
            opts.program_args,
            opts.program_env,
        )?;
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
