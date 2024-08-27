use std::{env, fs::File, io::Write};

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
    println!("Creating new project: {}", opts.name.green().bold());
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("-q").arg("new").arg(&opts.name);
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
    let locking_features = vec![opts.locking.as_feature_str()];
    let mut robo_features: Vec<&str> = locking_features.clone();
    for feature in &opts.features {
        for feature in feature.split(',') {
            robo_features.push(feature);
        }
    }
    add_dependency(
        "roboplc",
        "0.4",
        &robo_features,
        env::var("ROBOPLC_PATH").ok(),
        true,
    )?;
    add_dependency("tracing", "0.1", &["log"], None, false)?;
    let mut robo_toml = Config {
        remote: config::Remote {
            key: maybe_key,
            url: maybe_url,
            timeout: maybe_timeout,
        },
        build: <_>::default(),
        build_custom: <_>::default(),
    };
    if let Some(docker_arch) = opts.docker {
        robo_toml.build.target = Some(docker_arch.target().to_owned());
        if robo_toml.remote.url.is_none() {
            robo_toml.remote.url = Some(format!("docker://{}", opts.name));
        }
        let mut f = File::create("Dockerfile")?;
        writeln!(f, "FROM {}", docker_arch.docker_image_name())?;
        writeln!(
            f,
            "COPY ./{} /var/roboplc/program/current",
            docker_arch.binary_path_for(&opts.name).display()
        )?;
    }
    std::fs::write(CONFIG_FILE_NAME, toml::to_string_pretty(&robo_toml)?)?;
    std::fs::write("src/main.rs", prepare_main(TPL_DEFAULT_RS, &robo_features))?;
    println!("Project created: {}", opts.name.green().bold());
    Ok(())
}

fn add_dependency(
    name: &str,
    version: &str,
    features: &[&str],
    path: Option<String>,
    disable_defaults: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let dep = if path.is_some() {
        name.to_owned()
    } else {
        format!("{}@{}", name, version)
    };
    println!("Adding dependency: {}", dep.green().bold());
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("-q").arg("add").arg(dep);
    if let Some(path) = path {
        cmd.arg("--path").arg(path);
    }
    for feature in features {
        cmd.arg("--features").arg(feature);
    }
    if disable_defaults {
        cmd.arg("--no-default-features");
    }
    let result = cmd.status()?;
    if !result.success() {
        return Err(format!("Failed to add dependency {}", name).into());
    }
    Ok(())
}

#[allow(clippy::let_and_return)]
fn prepare_main(tpl: &str, features: &[&str]) -> String {
    // METRICS
    let mut out = if features.contains(&"metrics") {
        tpl.replace(
            "    // METRICS",
            r"    roboplc::metrics_exporter_install(
        roboplc::metrics_exporter().set_bucket_duration(Duration::from_secs(600))?,
    )?;",
        )
    } else {
        tpl.replace("    // METRICS\n", "")
    };
    // RVIDEO
    out = if features.contains(&"rvideo") {
        out.replace(
            "// RVIDEO-SERVE",
            r#"#[derive(WorkerOpts)]
#[worker_opts(cpu = 0, priority = 50, scheduling = "fifo", blocking = true)]
struct RvideoSrv {}

impl Worker<Message, Variables> for RvideoSrv {
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        roboplc::serve_rvideo().map_err(Into::into)
    }
}
"#,
        )
        .replace(
            "    // RVIDEO-SPAWN",
            "    controller.spawn_worker(RvideoSrv {})?;",
        )
    } else {
        out.replace("// RVIDEO-SERVE\n", "")
            .replace("    // RVIDEO-SPAWN\n", "")
    };
    // RFLOW
    out = if features.contains(&"rflow") {
        out.replace(
            "// RFLOW-SERVE",
            r#"#[derive(WorkerOpts)]
#[worker_opts(cpu = 0, priority = 50, scheduling = "fifo", blocking = true)]
struct RflowSrv {}

impl Worker<Message, Variables> for RflowSrv {
    fn run(&mut self, _context: &Context<Message, Variables>) -> WResult {
        roboplc::serve_rflow().map_err(Into::into)
    }
}
"#,
        )
        .replace(
            "    // RFLOW-SPAWN",
            "    controller.spawn_worker(RflowSrv {})?;",
        )
    } else {
        out.replace("// RFLOW-SERVE\n", "")
            .replace("    // RFLOW-SPAWN\n", "")
    };
    out
}
