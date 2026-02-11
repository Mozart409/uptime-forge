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

#[cfg(test)]
mod tests {
    use super::*;

    // ============ Environment Variable Substitution Tests ============

    #[test]
    fn substitute_env_vars_replaces_single_variable() {
        // SAFETY: Tests are run single-threaded with --test-threads=1 or are isolated
        unsafe {
            std::env::set_var("TEST_VAR_SINGLE", "test_value");
        }
        let result = substitute_env_vars("Bearer ${TEST_VAR_SINGLE}");
        assert_eq!(result, "Bearer test_value");
        unsafe {
            std::env::remove_var("TEST_VAR_SINGLE");
        }
    }

    #[test]
    fn substitute_env_vars_replaces_multiple_variables() {
        // SAFETY: Tests are run single-threaded with --test-threads=1 or are isolated
        unsafe {
            std::env::set_var("TEST_HOST", "example.com");
            std::env::set_var("TEST_PORT", "8080");
        }
        let result = substitute_env_vars("https://${TEST_HOST}:${TEST_PORT}/api");
        assert_eq!(result, "https://example.com:8080/api");
        unsafe {
            std::env::remove_var("TEST_HOST");
            std::env::remove_var("TEST_PORT");
        }
    }

    #[test]
    fn substitute_env_vars_returns_empty_for_missing_variable() {
        // Make sure the variable doesn't exist
        // SAFETY: Tests are run single-threaded with --test-threads=1 or are isolated
        unsafe {
            std::env::remove_var("NONEXISTENT_VAR_12345");
        }
        let result = substitute_env_vars("prefix_${NONEXISTENT_VAR_12345}_suffix");
        assert_eq!(result, "prefix__suffix");
    }

    #[test]
    fn substitute_env_vars_preserves_string_without_variables() {
        let input = "just a normal string";
        let result = substitute_env_vars(input);
        assert_eq!(result, input);
    }

    #[test]
    fn substitute_env_vars_handles_adjacent_variables() {
        // SAFETY: Tests are run single-threaded with --test-threads=1 or are isolated
        unsafe {
            std::env::set_var("TEST_A", "Hello");
            std::env::set_var("TEST_B", "World");
        }
        let result = substitute_env_vars("${TEST_A}${TEST_B}");
        assert_eq!(result, "HelloWorld");
        unsafe {
            std::env::remove_var("TEST_A");
            std::env::remove_var("TEST_B");
        }
    }

    #[test]
    fn substitute_env_vars_ignores_invalid_syntax() {
        // These should NOT be substituted
        let result = substitute_env_vars("$VAR ${} ${lowercase} ${123}");
        assert_eq!(result, "$VAR ${} ${lowercase} ${123}");
    }

    // ============ HttpMethod Tests ============

    #[test]
    fn http_method_converts_to_reqwest_correctly() {
        assert_eq!(HttpMethod::Get.as_reqwest_method(), reqwest::Method::GET);
        assert_eq!(HttpMethod::Post.as_reqwest_method(), reqwest::Method::POST);
        assert_eq!(HttpMethod::Put.as_reqwest_method(), reqwest::Method::PUT);
        assert_eq!(
            HttpMethod::Patch.as_reqwest_method(),
            reqwest::Method::PATCH
        );
        assert_eq!(
            HttpMethod::Delete.as_reqwest_method(),
            reqwest::Method::DELETE
        );
        assert_eq!(HttpMethod::Head.as_reqwest_method(), reqwest::Method::HEAD);
        assert_eq!(
            HttpMethod::Options.as_reqwest_method(),
            reqwest::Method::OPTIONS
        );
    }

    #[test]
    fn http_method_default_is_get() {
        assert_eq!(HttpMethod::default(), HttpMethod::Get);
    }

    // ============ CheckType Tests ============

    #[test]
    fn check_type_default_is_http() {
        assert_eq!(CheckType::default(), CheckType::Http);
    }

    // ============ Endpoint Tests ============

    fn make_test_endpoint(addr: &str) -> Endpoint {
        Endpoint {
            addr: addr.to_string(),
            check_type: CheckType::Http,
            description: None,
            group: None,
            tags: vec![],
            interval: 60,
            timeout: 10,
            expected_status: 200,
            skip_tls_verification: false,
            method: HttpMethod::Get,
            headers: HashMap::new(),
            body: None,
            retries: 0,
            retry_delay: 5,
            alert_after_failures: 3,
            alert_channels: vec![],
            expected_records: vec![],
        }
    }

    #[test]
    fn endpoint_resolved_headers_substitutes_env_vars() {
        // SAFETY: Tests are run single-threaded with --test-threads=1 or are isolated
        unsafe {
            std::env::set_var("TEST_TOKEN", "secret123");
        }
        let mut endpoint = make_test_endpoint("https://example.com");
        endpoint.headers.insert(
            "Authorization".to_string(),
            "Bearer ${TEST_TOKEN}".to_string(),
        );

        let resolved = endpoint.resolved_headers();
        assert_eq!(resolved.get("Authorization").unwrap(), "Bearer secret123");

        unsafe {
            std::env::remove_var("TEST_TOKEN");
        }
    }

    #[test]
    fn endpoint_resolved_body_substitutes_env_vars() {
        // SAFETY: Tests are run single-threaded with --test-threads=1 or are isolated
        unsafe {
            std::env::set_var("TEST_USER", "admin");
        }
        let mut endpoint = make_test_endpoint("https://example.com");
        endpoint.body = Some(r#"{"user": "${TEST_USER}"}"#.to_string());

        let resolved = endpoint.resolved_body();
        assert_eq!(resolved.unwrap(), r#"{"user": "admin"}"#);

        unsafe {
            std::env::remove_var("TEST_USER");
        }
    }

    #[test]
    fn endpoint_resolved_body_returns_none_when_no_body() {
        let endpoint = make_test_endpoint("https://example.com");
        assert!(endpoint.resolved_body().is_none());
    }

    #[test]
    fn endpoint_resolved_addr_substitutes_env_vars() {
        // SAFETY: Tests are run single-threaded with --test-threads=1 or are isolated
        unsafe {
            std::env::set_var("TEST_DOMAIN", "api.example.com");
        }
        let endpoint = make_test_endpoint("https://${TEST_DOMAIN}/health");

        let resolved = endpoint.resolved_addr();
        assert_eq!(resolved, "https://api.example.com/health");

        unsafe {
            std::env::remove_var("TEST_DOMAIN");
        }
    }

    // ============ Config Validation Tests ============

    fn make_test_config(endpoints: HashMap<String, Endpoint>) -> Config {
        Config {
            server: ServerConfig {
                addr: "127.0.0.1:3000".parse().unwrap(),
                reload_config_interval: 60,
            },
            endpoints,
        }
    }

    #[test]
    fn validation_errors_when_timeout_exceeds_interval() {
        let mut endpoint = make_test_endpoint("https://example.com");
        endpoint.timeout = 60;
        endpoint.interval = 30;

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("timeout"));
        assert!(errors[0].message.contains("must be less than interval"));
    }

    #[test]
    fn validation_errors_when_timeout_equals_interval() {
        let mut endpoint = make_test_endpoint("https://example.com");
        endpoint.timeout = 60;
        endpoint.interval = 60;

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("timeout"));
    }

    #[test]
    fn validation_passes_when_timeout_less_than_interval() {
        let endpoint = make_test_endpoint("https://example.com");
        // Default: timeout=10, interval=60

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert!(errors.is_empty());
    }

    #[test]
    fn validation_errors_on_invalid_http_url() {
        let endpoint = make_test_endpoint("not-a-valid-url");

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("invalid URL"));
    }

    #[test]
    fn validation_errors_when_tcp_missing_port() {
        let mut endpoint = make_test_endpoint("tcp://example.com");
        endpoint.check_type = CheckType::Tcp;
        endpoint.addr = "example.com".to_string(); // No port!

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("must include port"));
    }

    #[test]
    fn validation_passes_for_tcp_with_port() {
        let mut endpoint = make_test_endpoint("tcp://example.com:5432");
        endpoint.check_type = CheckType::Tcp;
        endpoint.addr = "tcp://example.com:5432".to_string();

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert!(errors.is_empty());
    }

    #[test]
    fn validation_passes_for_tcp_with_host_port_format() {
        let mut endpoint = make_test_endpoint("localhost:5432");
        endpoint.check_type = CheckType::Tcp;
        endpoint.addr = "localhost:5432".to_string();

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert!(errors.is_empty());
    }

    #[test]
    fn validation_errors_when_dns_is_url() {
        let mut endpoint = make_test_endpoint("dns://https://example.com");
        endpoint.check_type = CheckType::Dns;
        endpoint.addr = "https://example.com".to_string();

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("should be a hostname"));
    }

    #[test]
    fn validation_passes_for_dns_hostname() {
        let mut endpoint = make_test_endpoint("example.com");
        endpoint.check_type = CheckType::Dns;
        endpoint.addr = "example.com".to_string();

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert!(errors.is_empty());
    }

    #[test]
    fn validation_passes_for_dns_with_prefix() {
        let mut endpoint = make_test_endpoint("dns://example.com");
        endpoint.check_type = CheckType::Dns;
        endpoint.addr = "dns://example.com".to_string();

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, _warnings) = config.validate();

        assert!(errors.is_empty());
    }

    #[test]
    fn validation_warns_on_aggressive_interval() {
        let mut endpoint = make_test_endpoint("https://example.com");
        endpoint.interval = 5; // Very aggressive

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (errors, warnings) = config.validate();

        // Should error because timeout (10) >= interval (5)
        assert!(!errors.is_empty());
        // Also should warn about aggressive interval
        assert!(warnings.iter().any(|w| w.message.contains("aggressive")));
    }

    #[test]
    fn validation_warns_when_retries_without_delay() {
        let mut endpoint = make_test_endpoint("https://example.com");
        endpoint.retries = 3;
        endpoint.retry_delay = 0;

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (_errors, warnings) = config.validate();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("retry_delay is 0"));
    }

    #[test]
    fn validation_no_warning_when_retries_with_delay() {
        let mut endpoint = make_test_endpoint("https://example.com");
        endpoint.retries = 3;
        endpoint.retry_delay = 5;

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (_errors, warnings) = config.validate();

        assert!(warnings.is_empty());
    }

    #[test]
    fn validation_no_warning_when_no_retries() {
        let mut endpoint = make_test_endpoint("https://example.com");
        endpoint.retries = 0;
        endpoint.retry_delay = 0;

        let mut endpoints = HashMap::new();
        endpoints.insert("test".to_string(), endpoint);
        let config = make_test_config(endpoints);

        let (_errors, warnings) = config.validate();

        assert!(warnings.is_empty());
    }

    // ============ Config Loading Tests ============

    #[test]
    fn config_load_parses_valid_toml() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("forge.toml");

        let toml_content = r#"
[server]
addr = "0.0.0.0:3003"

[endpoints.example]
addr = "https://example.com"
description = "Example Site"
interval = 60
timeout = 10
"#;

        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = Config::load(&config_path).unwrap();

        assert_eq!(config.server.addr.to_string(), "0.0.0.0:3003");
        assert!(config.endpoints.contains_key("example"));

        let endpoint = config.endpoints.get("example").unwrap();
        assert_eq!(endpoint.addr, "https://example.com");
        assert_eq!(endpoint.description, Some("Example Site".to_string()));
        assert_eq!(endpoint.interval, 60);
        assert_eq!(endpoint.timeout, 10);
    }

    #[test]
    fn config_load_uses_defaults() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("forge.toml");

        let toml_content = r#"
[server]
addr = "0.0.0.0:3003"

[endpoints.minimal]
addr = "https://example.com"
"#;

        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = Config::load(&config_path).unwrap();
        let endpoint = config.endpoints.get("minimal").unwrap();

        // Check defaults
        assert_eq!(endpoint.interval, 60);
        assert_eq!(endpoint.timeout, 10);
        assert_eq!(endpoint.expected_status, 200);
        assert_eq!(endpoint.retries, 0);
        assert_eq!(endpoint.retry_delay, 5);
        assert_eq!(endpoint.alert_after_failures, 3);
        assert_eq!(endpoint.check_type, CheckType::Http);
        assert_eq!(endpoint.method, HttpMethod::Get);
        assert!(!endpoint.skip_tls_verification);
    }

    #[test]
    fn config_load_fails_on_missing_file() {
        let result = Config::load("/nonexistent/path/to/config.toml");
        assert!(result.is_err());
    }

    #[test]
    fn config_load_fails_on_invalid_toml() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("invalid.toml");

        let invalid_content = "this is not valid toml {{{";
        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(invalid_content.as_bytes()).unwrap();

        let result = Config::load(&config_path);
        assert!(result.is_err());
    }

    #[test]
    fn config_load_fails_on_validation_error() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("invalid_config.toml");

        // timeout >= interval should fail validation
        let toml_content = r#"
[server]
addr = "0.0.0.0:3003"

[endpoints.bad]
addr = "https://example.com"
interval = 10
timeout = 20
"#;

        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let result = Config::load(&config_path);
        assert!(result.is_err());
    }

    #[test]
    fn config_parses_check_types() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("types.toml");

        let toml_content = r#"
[server]
addr = "0.0.0.0:3003"

[endpoints.http_check]
addr = "https://example.com"
type = "http"

[endpoints.tcp_check]
addr = "tcp://db.example.com:5432"
type = "tcp"

[endpoints.dns_check]
addr = "dns://example.com"
type = "dns"
"#;

        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = Config::load(&config_path).unwrap();

        assert_eq!(
            config.endpoints.get("http_check").unwrap().check_type,
            CheckType::Http
        );
        assert_eq!(
            config.endpoints.get("tcp_check").unwrap().check_type,
            CheckType::Tcp
        );
        assert_eq!(
            config.endpoints.get("dns_check").unwrap().check_type,
            CheckType::Dns
        );
    }

    #[test]
    fn config_parses_http_methods() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("methods.toml");

        let toml_content = r#"
[server]
addr = "0.0.0.0:3003"

[endpoints.get_endpoint]
addr = "https://example.com"
method = "GET"

[endpoints.post_endpoint]
addr = "https://example.com"
method = "POST"
body = '{"test": true}'
"#;

        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = Config::load(&config_path).unwrap();

        assert_eq!(
            config.endpoints.get("get_endpoint").unwrap().method,
            HttpMethod::Get
        );
        assert_eq!(
            config.endpoints.get("post_endpoint").unwrap().method,
            HttpMethod::Post
        );
        assert_eq!(
            config.endpoints.get("post_endpoint").unwrap().body,
            Some(r#"{"test": true}"#.to_string())
        );
    }

    #[test]
    fn config_parses_headers() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("headers.toml");

        let toml_content = r#"
[server]
addr = "0.0.0.0:3003"

[endpoints.api]
addr = "https://api.example.com"
headers = { Authorization = "Bearer token123", "Content-Type" = "application/json" }
"#;

        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = Config::load(&config_path).unwrap();
        let endpoint = config.endpoints.get("api").unwrap();

        assert_eq!(
            endpoint.headers.get("Authorization").unwrap(),
            "Bearer token123"
        );
        assert_eq!(
            endpoint.headers.get("Content-Type").unwrap(),
            "application/json"
        );
    }
}
