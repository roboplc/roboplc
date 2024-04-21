use std::env;

use colored::Colorize as _;

use crate::{
    arguments::NewCommand,
    common::CONFIG_FILE_NAME,
    config::{self, Config},
    TPL_DEFAULT_RS,
};

pub fn create(
    maybe_url: Option<String>,
    maybe_key: Option<String>,
    maybe_timeout: Option<u64>,
    opts: &NewCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating new project {}", opts.name.green());
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("new").arg(&opts.name);
    if !opts.extras.is_empty() {
        cmd.args(&opts.extras);
    }
    let result = cmd.status()?;
    if !result.success() {
        return Err("Failed to create new project with cargo".into());
    }
    let mut current_dir = env::current_dir()?;
    current_dir.push(&opts.name);
    env::set_current_dir(&current_dir)?;
    let mut robo_features: Vec<&str> = Vec::new();
    for feature in &opts.features {
        for feature in feature.split(',') {
            robo_features.push(feature);
        }
    }
    add_dependency("roboplc", &robo_features)?;
    add_dependency("tracing", &["log"])?;
    let robo_toml = Config {
        remote: config::Remote {
            key: maybe_key,
            url: maybe_url,
            timeout: maybe_timeout,
        },
        build: <_>::default(),
    };
    std::fs::write(CONFIG_FILE_NAME, toml::to_string_pretty(&robo_toml)?)?;
    std::fs::write("src/main.rs", prepare_main(TPL_DEFAULT_RS, &robo_features))?;
    println!("Project created: {}", opts.name.green());
    Ok(())
}

fn add_dependency(name: &str, features: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    println!("Adding dependency {}", name.green());
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("add").arg(name);
    for feature in features {
        cmd.arg("--features").arg(feature);
    }
    let result = cmd.status()?;
    if !result.success() {
        return Err(format!("Failed to add dependency {}", name).into());
    }
    Ok(())
}

#[allow(clippy::let_and_return)]
fn prepare_main(tpl: &str, features: &[&str]) -> String {
    let out = if features.contains(&"metrics") {
        tpl.replace(
            "    // METRICS",
            r"    roboplc::metrics_exporter()
        .set_bucket_duration(Duration::from_secs(600))?
        .install()?;",
        )
    } else {
        tpl.replace("    // METRICS\n", "")
    };
    out
}
