use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub remote: Remote,
    #[serde(default)]
    pub build: Build,
}

#[derive(Deserialize, Serialize, Default)]
pub struct Remote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Deserialize, Serialize, Default)]
pub struct Build {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo_args: Option<String>,
}
