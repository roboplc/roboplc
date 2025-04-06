use std::{collections::BTreeMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::common::{print_err, GLOBAL_CONFIG_FILE_NAME};

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub remote: Remote,
    #[serde(default)]
    pub build: Build,
    #[serde(default)]
    pub x: X,
    #[serde(default, rename = "build-custom")]
    pub build_custom: BuildCustom,
}

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct Remote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct Build {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo_args: Option<String>,
}

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct X {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct BuildCustom {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
}

#[derive(Deserialize, Debug)]
struct GlobalConfig {
    remote: BTreeMap<String, Remote>,
}

pub fn get_global_remote(url: &str) -> Option<Remote> {
    let Some(home) = dirs::home_dir() else {
        print_err("Cannot get home directory");
        return None;
    };
    let path = home.join(GLOBAL_CONFIG_FILE_NAME);
    if !path.exists() {
        return None;
    }
    match fs::read_to_string(&path) {
        Ok(contents) => match toml::from_str::<GlobalConfig>(&contents) {
            Ok(mut config) => {
                if let Some(remote) = config.remote.remove(url) {
                    return Some(remote);
                }
                None
            }
            Err(e) => {
                print_err(&format!("Cannot parse {}: {}", path.display(), e));
                None
            }
        },
        Err(e) => {
            print_err(&format!("Cannot read {}: {}", path.display(), e));
            None
        }
    }
}

#[derive(Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct ServerConfig {
    pub http: ServerHttpConfig,
    pub aaa: ServerAaaConfig,
}

impl ServerConfig {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let s = fs::read_to_string("/etc/roboplc/manager.toml")?;
        let config: ServerConfig = toml::from_str(&s)?;
        Ok(config)
    }
}

#[derive(Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct ServerHttpConfig {
    pub bind: String,
}

#[derive(Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct ServerAaaConfig {
    pub management_key: Option<String>,
}
