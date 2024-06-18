use std::{collections::BTreeMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::common::{print_err, GLOBAL_CONFIG_FILE_NAME};

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub remote: Remote,
    #[serde(default)]
    pub build: Build,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo_args: Option<String>,
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
