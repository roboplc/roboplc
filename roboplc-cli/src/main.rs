use std::{fs, time::Duration};

use arguments::{Args, SubCommand};
use clap::Parser;
use common::{find_robo_toml, Mode};
use once_cell::sync::OnceCell;
use ureq::Agent;

use crate::config::Config;

const API_PREFIX: &str = "/roboplc/api";
const DEFAULT_TIMEOUT: u64 = 60;
const TPL_DEFAULT_RS: &str = include_str!("../tpl/default.rs");

// filled by find_robo_toml if Cargo.toml is found
static TARGET_PACKAGE_NAME: OnceCell<String> = OnceCell::new();
static TARGET_PACKAGE_VERSION: OnceCell<String> = OnceCell::new();

static CARGO_TARGET_DIR: OnceCell<String> = OnceCell::new();

/// # Panics
///
/// Panics if `CARGO_TARGET_DIR` is not set
pub fn cargo_target_dir() -> &'static str {
    CARGO_TARGET_DIR.get().expect("CARGO_TARGET_DIR not set")
}

mod arguments;
mod common;
mod config;
mod exec;
mod flashing;
mod project;
mod remote;
mod ureq_err;

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    let _ansi_enabled = ansi_term::enable_ansi_support();
    let args = Args::parse();
    CARGO_TARGET_DIR
        .set(std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_owned()))
        .expect("unable to set CARGO_TARGET_DIR");
    let mut maybe_url = args.url;
    let mut maybe_key = args.key;
    if let Some(ref u) = maybe_url {
        if !u.starts_with("http://") && !u.starts_with("https://") {
            // try to get url from global config
            if let Some(remote) = config::get_global_remote(u) {
                if let Some(url) = remote.url {
                    maybe_url = Some(url);
                }
                if let Some(key) = remote.key {
                    maybe_key = Some(key);
                }
            }
        }
    }
    let mut maybe_timeout = args.timeout;
    let mut build_config = None;
    let mut build_custom = None;
    if let SubCommand::New(_) = args.subcmd {
        // do not parse robo.toml for `new` command
    } else if let Some(robo_toml_path) = find_robo_toml() {
        let contents = fs::read_to_string(robo_toml_path)?;
        let robo_toml: Config = toml::from_str(&contents)?;
        if maybe_url.is_none() {
            maybe_url = robo_toml.remote.url;
        }
        if maybe_key.is_none() {
            maybe_key = robo_toml.remote.key;
        }
        if maybe_timeout.is_none() {
            maybe_timeout = robo_toml.remote.timeout;
        }
        build_config = Some(robo_toml.build);
        build_custom = Some(robo_toml.build_custom);
    }
    maybe_url = maybe_url.map(|v| {
        let mut u = v.trim_end_matches('/').to_owned();
        if !u.starts_with("http://") && !u.starts_with("https://") && !u.starts_with("docker://") {
            u = format!("http://{}", u);
        }
        u
    });
    if let SubCommand::New(opts) = args.subcmd {
        project::create(maybe_url, maybe_key, maybe_timeout, &opts)?;
        return Ok(());
    }
    let url = maybe_url.ok_or("URL not specified")?;
    let key = if let Some(k) = maybe_key {
        k
    } else if url.starts_with("docker://") {
        String::new()
    } else {
        return Err("Key not specified".into());
    };
    let timeout = maybe_timeout.unwrap_or(DEFAULT_TIMEOUT);
    let agent: Agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(timeout))
        .timeout_write(Duration::from_secs(timeout))
        .build();
    match args.subcmd {
        SubCommand::New(_) => {
            panic!("BUG");
        }
        SubCommand::Stat(opts) => {
            remote::stat(&url, &key, agent, opts.show_versions)?;
        }
        SubCommand::Config => {
            remote::set_mode(&url, &key, &agent, Mode::Config, true)?;
        }
        SubCommand::Run => {
            remote::set_mode(&url, &key, &agent, Mode::Run, true)?;
        }
        SubCommand::Restart => {
            remote::set_mode(&url, &key, &agent, Mode::Config, false)?;
            remote::set_mode(&url, &key, &agent, Mode::Run, true)?;
        }
        SubCommand::Flash(opts) => {
            flashing::flash(
                &url,
                &key,
                agent,
                opts.into(),
                build_config.unwrap_or_default(),
                build_custom.unwrap_or_default(),
                false,
            )?;
        }
        SubCommand::Rollback(opts) => {
            flashing::rollback(&url, &key, agent, opts)?;
        }
        SubCommand::Exec(opts) => {
            flashing::flash(
                &url,
                &key,
                agent,
                opts.into(),
                build_config.unwrap_or_default(),
                build_custom.unwrap_or_default(),
                true,
            )?;
        }
        SubCommand::Purge => {
            remote::purge(&url, &key, agent)?;
        }
    }
    Ok(())
}
