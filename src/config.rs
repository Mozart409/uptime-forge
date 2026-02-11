use std::{collections::HashMap, net::SocketAddr, path::Path};

use color_eyre::eyre::{Context, Result, bail};
use regex::Regex;
use serde::Deserialize;
use url::Url;

/// Type of health check to perform
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CheckType {
    #[default]
    Http,
    Tcp,
    Dns,
}

/// HTTP method for health checks
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl HttpMethod {
    pub fn as_reqwest_method(&self) -> reqwest::Method {
        match self {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Delete => reqwest::Method::DELETE,
            HttpMethod::Head => reqwest::Method::HEAD,
            HttpMethod::Options => reqwest::Method::OPTIONS,
        }
    }
}

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
    /// URL or address to check (required)
    pub addr: String,
    /// Type of check: http (default), tcp, dns
    #[serde(default, rename = "type")]
    pub check_type: CheckType,
    /// Optional description
    #[serde(default)]
    pub description: Option<String>,
    /// Optional group for organizing endpoints
    #[serde(default)]
    pub group: Option<String>,
    /// Optional tags for filtering
    #[serde(default)]
    pub tags: Vec<String>,
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
    /// HTTP method (default: GET)
    #[serde(default)]
    pub method: HttpMethod,
    /// Custom headers (supports `${ENV_VAR}` substitution)
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Request body for POST/PUT
    #[serde(default)]
    pub body: Option<String>,
    /// Number of retries before marking as failed (default: 0)
    #[serde(default)]
    pub retries: u32,
    /// Delay between retries in seconds (default: 5)
    #[serde(default = "default_retry_delay")]
    pub retry_delay: u64,
    /// Alert after N consecutive failures (default: 3)
    #[serde(default = "default_alert_after_failures")]
    pub alert_after_failures: u32,
    /// Alert channels to notify (e.g., `["webhook"]`)
    #[serde(default)]
    pub alert_channels: Vec<String>,
    /// Expected DNS records (for DNS check type)
    #[serde(default)]
    pub expected_records: Vec<String>,
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

const fn default_retry_delay() -> u64 {
    5
}

const fn default_alert_after_failures() -> u32 {
    3
}

/// Regex pattern for environment variable substitution: `${VAR_NAME}`
fn env_var_pattern() -> Regex {
    Regex::new(r"\$\{([A-Z_][A-Z0-9_]*)\}").expect("invalid regex pattern")
}

/// Substitute environment variables in a string
/// Supports `${VAR_NAME}` syntax
pub fn substitute_env_vars(input: &str) -> String {
    let pattern = env_var_pattern();
    pattern
        .replace_all(input, |caps: &regex::Captures| {
            let var_name = &caps[1];
            std::env::var(var_name).unwrap_or_else(|_| {
                tracing::warn!(var = %var_name, "environment variable not found, using empty string");
                String::new()
            })
        })
        .to_string()
}

impl Endpoint {
    /// Get headers with environment variables substituted
    pub fn resolved_headers(&self) -> HashMap<String, String> {
        self.headers
            .iter()
            .map(|(k, v)| (k.clone(), substitute_env_vars(v)))
            .collect()
    }

    /// Get body with environment variables substituted
    pub fn resolved_body(&self) -> Option<String> {
        self.body.as_ref().map(|b| substitute_env_vars(b))
    }

    /// Get addr with environment variables substituted
    pub fn resolved_addr(&self) -> String {
        substitute_env_vars(&self.addr)
    }
}

/// Configuration validation errors
#[derive(Debug)]
pub struct ValidationWarning {
    pub endpoint: String,
    pub message: String,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read config file: {}", path.display()))?;

        let config: Config = toml::from_str(&content)
            .wrap_err_with(|| format!("failed to parse config file: {}", path.display()))?;

        // Validate and report warnings
        let (errors, warnings) = config.validate();

        for warning in warnings {
            tracing::warn!(
                endpoint = %warning.endpoint,
                "config warning: {}",
                warning.message
            );
        }

        if !errors.is_empty() {
            let error_messages: Vec<_> = errors
                .iter()
                .map(|e| format!("[{}] {}", e.endpoint, e.message))
                .collect();
            bail!("configuration errors:\n  {}", error_messages.join("\n  "));
        }

        Ok(config)
    }

    /// Validate the configuration and return (errors, warnings)
    pub fn validate(&self) -> (Vec<ValidationWarning>, Vec<ValidationWarning>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        for (name, endpoint) in &self.endpoints {
            // Validate timeout < interval
            if endpoint.timeout >= endpoint.interval {
                errors.push(ValidationWarning {
                    endpoint: name.clone(),
                    message: format!(
                        "timeout ({}) must be less than interval ({})",
                        endpoint.timeout, endpoint.interval
                    ),
                });
            }

            // Validate URL format based on check type
            match endpoint.check_type {
                CheckType::Http => {
                    let resolved_addr = endpoint.resolved_addr();
                    if let Err(e) = Url::parse(&resolved_addr) {
                        errors.push(ValidationWarning {
                            endpoint: name.clone(),
                            message: format!("invalid URL '{resolved_addr}': {e}"),
                        });
                    }
                }
                CheckType::Tcp => {
                    // TCP addresses should be in format "host:port" or "tcp://host:port"
                    let addr = endpoint
                        .addr
                        .strip_prefix("tcp://")
                        .unwrap_or(&endpoint.addr);
                    if !addr.contains(':') {
                        errors.push(ValidationWarning {
                            endpoint: name.clone(),
                            message: format!(
                                "TCP address '{}' must include port (e.g., 'host:port')",
                                endpoint.addr
                            ),
                        });
                    }
                }
                CheckType::Dns => {
                    // DNS addresses should be valid hostnames
                    let addr = endpoint
                        .addr
                        .strip_prefix("dns://")
                        .unwrap_or(&endpoint.addr);
                    if addr.contains("://") {
                        errors.push(ValidationWarning {
                            endpoint: name.clone(),
                            message: format!(
                                "DNS address '{}' should be a hostname, not a URL",
                                endpoint.addr
                            ),
                        });
                    }
                }
            }

            // Warn if interval is too aggressive
            if endpoint.interval < 10 {
                warnings.push(ValidationWarning {
                    endpoint: name.clone(),
                    message: format!(
                        "interval ({}) is very aggressive, consider >= 10 seconds",
                        endpoint.interval
                    ),
                });
            }

            // Warn if retries configured but no retry delay
            if endpoint.retries > 0 && endpoint.retry_delay == 0 {
                warnings.push(ValidationWarning {
                    endpoint: name.clone(),
                    message: "retries configured but retry_delay is 0".to_string(),
                });
            }
        }

        (errors, warnings)
    }
}
