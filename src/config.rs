use std::{collections::HashMap, net::SocketAddr, path::Path};

use color_eyre::eyre::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub endpoints: HashMap<String, Endpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    /// Interval in seconds to reload config file (default: 60, 0 to disable)
    #[serde(default = "default_reload_config_interval")]
    pub reload_config_interval: u64,
}

const fn default_reload_config_interval() -> u64 {
    60
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Endpoint {
    pub addr: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Check interval in seconds (default: 60)
    #[serde(default = "default_interval")]
    pub interval: u64,
    /// Request timeout in seconds (default: 10)
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    /// Expected HTTP status code (default: 200)
    #[serde(default = "default_expected_status")]
    pub expected_status: u16,
    /// Skip TLS certificate verification (default: false)
    #[serde(default)]
    pub skip_tls_verification: bool,
}

const fn default_interval() -> u64 {
    60
}

const fn default_timeout() -> u64 {
    10
}

const fn default_expected_status() -> u16 {
    200
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read config file: {}", path.display()))?;

        toml::from_str(&content)
            .wrap_err_with(|| format!("failed to parse config file: {}", path.display()))
    }
}
