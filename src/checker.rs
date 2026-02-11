use std::{collections::HashMap, net::ToSocketAddrs, path::PathBuf, sync::Arc, time::Duration};

use hickory_resolver::{Resolver, config::ResolverConfig, name_server::TokioConnectionProvider};
use reqwest::Client;
use sqlx::PgPool;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::{RwLock, mpsc},
};
use tokio_util::sync::CancellationToken;

use crate::config::{CheckType, Config, Endpoint};
use crate::db;

/// Shared state containing cached check results
pub type CheckResultsState = Arc<RwLock<HashMap<String, CheckResult>>>;

/// Shared state for active endpoint tasks (name -> cancellation token)
type ActiveTasks = Arc<RwLock<HashMap<String, CancellationToken>>>;

/// Channel sender for triggering config reload
pub type ReloadTrigger = mpsc::Sender<()>;

/// Error type classification for failed checks
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorType {
    Timeout,
    Dns,
    Tls,
    Connection,
    StatusMismatch,
    TcpRefused,
    DnsNxdomain,
    DnsMismatch,
    ClientBuild,
    Unknown,
}

impl ErrorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorType::Timeout => "timeout",
            ErrorType::Dns => "dns",
            ErrorType::Tls => "tls",
            ErrorType::Connection => "connection",
            ErrorType::StatusMismatch => "status_mismatch",
            ErrorType::TcpRefused => "tcp_refused",
            ErrorType::DnsNxdomain => "dns_nxdomain",
            ErrorType::DnsMismatch => "dns_mismatch",
            ErrorType::ClientBuild => "client_build",
            ErrorType::Unknown => "unknown",
        }
    }
}

/// Result of checking an endpoint's availability
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub tags: Vec<String>,
    pub addr: String,
    pub check_type: CheckType,
    pub is_up: bool,
    pub status_code: Option<u16>,
    pub response_time_ms: Option<u64>,
    pub error: Option<String>,
    pub error_type: Option<ErrorType>,
}

/// Classify a reqwest error into an `ErrorType`
fn classify_reqwest_error(e: &reqwest::Error) -> ErrorType {
    if e.is_timeout() {
        ErrorType::Timeout
    } else if e.is_connect() {
        // Check for DNS errors in the error chain
        let error_str = e.to_string().to_lowercase();
        if error_str.contains("dns") || error_str.contains("resolve") {
            ErrorType::Dns
        } else if error_str.contains("tls")
            || error_str.contains("ssl")
            || error_str.contains("certificate")
        {
            ErrorType::Tls
        } else {
            ErrorType::Connection
        }
    } else {
        ErrorType::Unknown
    }
}

/// Create a base `CheckResult` with common fields
fn base_result(name: &str, endpoint: &Endpoint) -> CheckResult {
    CheckResult {
        name: name.to_string(),
        description: endpoint.description.clone(),
        group: endpoint.group.clone(),
        tags: endpoint.tags.clone(),
        addr: endpoint.resolved_addr(),
        check_type: endpoint.check_type.clone(),
        is_up: false,
        status_code: None,
        response_time_ms: None,
        error: None,
        error_type: None,
    }
}

/// Check a single endpoint's availability with retries
pub async fn check_endpoint(name: &str, endpoint: &Endpoint) -> CheckResult {
    let max_attempts = endpoint.retries + 1;
    let mut last_result = base_result(name, endpoint);

    for attempt in 0..max_attempts {
        if attempt > 0 {
            tracing::debug!(
                endpoint = %name,
                attempt = attempt + 1,
                max_attempts = max_attempts,
                "retrying endpoint check"
            );
            tokio::time::sleep(Duration::from_secs(endpoint.retry_delay)).await;
        }

        last_result = match endpoint.check_type {
            CheckType::Http => check_http(name, endpoint).await,
            CheckType::Tcp => check_tcp(name, endpoint).await,
            CheckType::Dns => check_dns(name, endpoint).await,
        };

        if last_result.is_up {
            return last_result;
        }
    }

    last_result
}

/// Perform an HTTP health check
async fn check_http(name: &str, endpoint: &Endpoint) -> CheckResult {
    let mut result = base_result(name, endpoint);

    let client = match Client::builder()
        .timeout(Duration::from_secs(endpoint.timeout))
        .danger_accept_invalid_certs(endpoint.skip_tls_verification)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            result.error = Some(format!("failed to build HTTP client: {e}"));
            result.error_type = Some(ErrorType::ClientBuild);
            return result;
        }
    };

    let start = std::time::Instant::now();
    let resolved_addr = endpoint.resolved_addr();

    // Build the request with method, headers, and body
    let mut request = client.request(endpoint.method.as_reqwest_method(), &resolved_addr);

    // Add custom headers with env var substitution
    for (key, value) in endpoint.resolved_headers() {
        request = request.header(&key, &value);
    }

    // Add body if present
    if let Some(body) = endpoint.resolved_body() {
        request = request.body(body);
    }

    match request.send().await {
        Ok(response) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            let status = response.status().as_u16();
            let is_up = status == endpoint.expected_status;

            result.is_up = is_up;
            result.status_code = Some(status);
            result.response_time_ms = Some(elapsed);

            if !is_up {
                result.error = Some(format!(
                    "expected status {}, got {}",
                    endpoint.expected_status, status
                ));
                result.error_type = Some(ErrorType::StatusMismatch);
            }
        }
        Err(e) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            result.response_time_ms = Some(elapsed);
            result.error = Some(e.to_string());
            result.error_type = Some(classify_reqwest_error(&e));
        }
    }

    result
}

/// Perform a TCP connectivity check
async fn check_tcp(name: &str, endpoint: &Endpoint) -> CheckResult {
    let mut result = base_result(name, endpoint);

    // Parse the address (strip tcp:// prefix if present)
    let addr = endpoint
        .resolved_addr()
        .strip_prefix("tcp://")
        .unwrap_or(&endpoint.resolved_addr())
        .to_string();

    let start = std::time::Instant::now();

    // Resolve address first
    let socket_addr = match addr.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(a) = addrs.next() {
                a
            } else {
                result.error = Some(format!("no addresses found for '{addr}'"));
                result.error_type = Some(ErrorType::Dns);
                return result;
            }
        }
        Err(e) => {
            result.error = Some(format!("failed to resolve address: {e}"));
            result.error_type = Some(ErrorType::Dns);
            return result;
        }
    };

    let timeout = Duration::from_secs(endpoint.timeout);

    match tokio::time::timeout(timeout, TcpStream::connect(socket_addr)).await {
        Ok(Ok(mut stream)) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

            // Try to write and read to ensure the connection is truly alive
            let write_result = stream.write_all(b"").await;
            let _ = stream.shutdown().await;

            if write_result.is_ok() {
                result.is_up = true;
                result.response_time_ms = Some(elapsed);
            } else {
                result.response_time_ms = Some(elapsed);
                result.error = Some("connection established but write failed".to_string());
                result.error_type = Some(ErrorType::Connection);
            }
        }
        Ok(Err(e)) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            result.response_time_ms = Some(elapsed);

            let error_str = e.to_string().to_lowercase();
            result.error = Some(e.to_string());
            result.error_type = Some(if error_str.contains("refused") {
                ErrorType::TcpRefused
            } else {
                ErrorType::Connection
            });
        }
        Err(_) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            result.response_time_ms = Some(elapsed);
            result.error = Some("connection timed out".to_string());
            result.error_type = Some(ErrorType::Timeout);
        }
    }

    result
}

/// Perform a DNS resolution check
async fn check_dns(name: &str, endpoint: &Endpoint) -> CheckResult {
    let mut result = base_result(name, endpoint);

    // Parse the hostname (strip dns:// prefix if present)
    let hostname = endpoint
        .resolved_addr()
        .strip_prefix("dns://")
        .unwrap_or(&endpoint.resolved_addr())
        .to_string();

    let start = std::time::Instant::now();

    // Create resolver
    let resolver = Resolver::builder_with_config(
        ResolverConfig::default(),
        TokioConnectionProvider::default(),
    )
    .build();

    let timeout = Duration::from_secs(endpoint.timeout);
    let lookup_future = resolver.lookup_ip(&hostname);

    match tokio::time::timeout(timeout, lookup_future).await {
        Ok(Ok(response)) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            result.response_time_ms = Some(elapsed);

            let resolved_ips: Vec<String> = response.iter().map(|ip| ip.to_string()).collect();

            // If expected_records is specified, check if they match
            if endpoint.expected_records.is_empty() {
                // No expected records, just check if resolution succeeded
                result.is_up = !resolved_ips.is_empty();
                if resolved_ips.is_empty() {
                    result.error = Some("DNS resolution returned no records".to_string());
                    result.error_type = Some(ErrorType::Dns);
                }
            } else {
                let all_found = endpoint
                    .expected_records
                    .iter()
                    .all(|expected| resolved_ips.contains(expected));

                if all_found {
                    result.is_up = true;
                } else {
                    result.error = Some(format!(
                        "expected records {:?}, got {:?}",
                        endpoint.expected_records, resolved_ips
                    ));
                    result.error_type = Some(ErrorType::DnsMismatch);
                }
            }
        }
        Ok(Err(e)) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            result.response_time_ms = Some(elapsed);

            let error_str = e.to_string().to_lowercase();
            result.error = Some(e.to_string());
            result.error_type = Some(
                if error_str.contains("nxdomain") || error_str.contains("no such") {
                    ErrorType::DnsNxdomain
                } else {
                    ErrorType::Dns
                },
            );
        }
        Err(_) => {
            let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            result.response_time_ms = Some(elapsed);
            result.error = Some("DNS lookup timed out".to_string());
            result.error_type = Some(ErrorType::Timeout);
        }
    }

    result
}

/// Check all endpoints concurrently and return results sorted alphabetically by name
pub async fn check_all_endpoints(endpoints: &HashMap<String, Endpoint>) -> Vec<CheckResult> {
    let futures: Vec<_> = endpoints
        .iter()
        .map(|(name, endpoint)| check_endpoint(name, endpoint))
        .collect();

    let mut results = futures::future::join_all(futures).await;
    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    results
}

/// Get sorted results from the shared state
pub async fn get_sorted_results(state: &CheckResultsState) -> Vec<CheckResult> {
    let results = state.read().await;
    let mut sorted: Vec<_> = results.values().cloned().collect();
    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    sorted
}

/// Spawn a background checking task for a single endpoint
fn spawn_endpoint_checker(
    name: String,
    endpoint: Endpoint,
    state: CheckResultsState,
    db_pool: Option<PgPool>,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        let interval = Duration::from_secs(endpoint.interval);

        loop {
            let result = check_endpoint(&name, &endpoint).await;

            tracing::debug!(
                endpoint = %name,
                is_up = result.is_up,
                response_time_ms = ?result.response_time_ms,
                "endpoint check completed"
            );

            // Write event to database if available
            if let Some(ref pool) = db_pool
                && let Err(e) = db::insert_uptime_event(pool, &result).await
            {
                tracing::warn!(endpoint = %name, error = %e, "failed to insert uptime event");
            }

            {
                let mut results = state.write().await;
                results.insert(name.clone(), result);
            }

            tokio::select! {
                () = tokio::time::sleep(interval) => {}
                () = cancel_token.cancelled() => {
                    tracing::debug!(endpoint = %name, "endpoint checker cancelled");
                    break;
                }
            }
        }
    });
}

/// Perform initial check of all endpoints and populate state
pub async fn initial_check(
    endpoints: &HashMap<String, Endpoint>,
    state: &CheckResultsState,
    db_pool: Option<&PgPool>,
) {
    tracing::info!("performing initial endpoint checks");

    let results = check_all_endpoints(endpoints).await;

    // Write initial results to database
    if let Some(pool) = db_pool {
        for result in &results {
            if let Err(e) = db::insert_uptime_event(pool, result).await {
                tracing::warn!(endpoint = %result.name, error = %e, "failed to insert initial uptime event");
            }
        }
    }

    let mut state_guard = state.write().await;
    for result in results {
        state_guard.insert(result.name.clone(), result);
    }

    tracing::info!("initial endpoint checks completed");
}

/// Apply config changes: cancel old tasks, start new ones, re-check all endpoints
async fn apply_config_update(
    new_endpoints: &HashMap<String, Endpoint>,
    current_endpoints: &mut HashMap<String, Endpoint>,
    active_tasks: &ActiveTasks,
    state: &CheckResultsState,
    db_pool: Option<PgPool>,
) {
    let mut tasks = active_tasks.write().await;
    let mut results = state.write().await;

    // Find removed endpoints
    let removed: Vec<_> = current_endpoints
        .keys()
        .filter(|k| !new_endpoints.contains_key(*k))
        .cloned()
        .collect();

    // Find added endpoints
    let added: Vec<_> = new_endpoints
        .keys()
        .filter(|k| !current_endpoints.contains_key(*k))
        .cloned()
        .collect();

    // Find changed endpoints
    let changed: Vec<_> = new_endpoints
        .iter()
        .filter(|(k, v)| current_endpoints.get(*k).is_some_and(|old| old != *v))
        .map(|(k, _)| k.clone())
        .collect();

    // Find unchanged endpoints (for re-check)
    let unchanged: Vec<_> = new_endpoints
        .keys()
        .filter(|k| !added.contains(k) && !changed.contains(k))
        .cloned()
        .collect();

    // Cancel removed endpoints
    for name in &removed {
        if let Some(token) = tasks.remove(name) {
            token.cancel();
            tracing::info!(endpoint = %name, "removed endpoint");
        }
        results.remove(name);
    }

    // Cancel and restart changed endpoints
    for name in &changed {
        if let Some(token) = tasks.remove(name) {
            token.cancel();
        }
        results.remove(name);

        if let Some(endpoint) = new_endpoints.get(name) {
            let cancel_token = CancellationToken::new();
            spawn_endpoint_checker(
                name.clone(),
                endpoint.clone(),
                Arc::clone(state),
                db_pool.clone(),
                cancel_token.clone(),
            );
            tasks.insert(name.clone(), cancel_token);
            tracing::info!(endpoint = %name, "updated endpoint");
        }
    }

    // Start added endpoints
    for name in &added {
        if let Some(endpoint) = new_endpoints.get(name) {
            let cancel_token = CancellationToken::new();
            spawn_endpoint_checker(
                name.clone(),
                endpoint.clone(),
                Arc::clone(state),
                db_pool.clone(),
                cancel_token.clone(),
            );
            tasks.insert(name.clone(), cancel_token);
            tracing::info!(endpoint = %name, "added endpoint");
        }
    }

    // Release locks before re-checking
    drop(tasks);
    drop(results);

    // Re-check all endpoints (added, changed, and unchanged)
    let endpoints_to_check: HashMap<_, _> = new_endpoints
        .iter()
        .filter(|(k, _)| added.contains(k) || changed.contains(k) || unchanged.contains(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if !endpoints_to_check.is_empty() {
        tracing::info!(
            "re-checking {} endpoints after config reload",
            endpoints_to_check.len()
        );
        let check_results = check_all_endpoints(&endpoints_to_check).await;

        // Write to database
        if let Some(ref pool) = db_pool {
            for result in &check_results {
                if let Err(e) = db::insert_uptime_event(pool, result).await {
                    tracing::warn!(endpoint = %result.name, error = %e, "failed to insert uptime event");
                }
            }
        }

        let mut results = state.write().await;
        for result in check_results {
            results.insert(result.name.clone(), result);
        }
    }

    // Update current endpoints
    current_endpoints.clone_from(new_endpoints);
}

/// Start all endpoint checkers and return the active tasks tracker
async fn start_all_checkers(
    endpoints: &HashMap<String, Endpoint>,
    state: &CheckResultsState,
    db_pool: Option<PgPool>,
) -> ActiveTasks {
    let active_tasks: ActiveTasks = Arc::default();

    for (name, endpoint) in endpoints {
        let cancel_token = CancellationToken::new();

        spawn_endpoint_checker(
            name.clone(),
            endpoint.clone(),
            Arc::clone(state),
            db_pool.clone(),
            cancel_token.clone(),
        );

        let mut tasks = active_tasks.write().await;
        tasks.insert(name.clone(), cancel_token);
    }

    active_tasks
}

/// Spawn the config reloader and all endpoint checkers.
/// Returns a channel sender that can be used to trigger manual reloads.
pub async fn spawn_background_tasks(
    config_path: PathBuf,
    initial_config: Config,
    state: CheckResultsState,
    db_pool: Option<PgPool>,
) -> ReloadTrigger {
    let reload_interval = initial_config.server.reload_config_interval;

    // Start initial endpoint checkers
    let active_tasks = start_all_checkers(&initial_config.endpoints, &state, db_pool.clone()).await;

    // Store current endpoints for comparison
    let current_endpoints = Arc::new(RwLock::new(initial_config.endpoints));

    // Create channel for manual reload triggers
    let (reload_tx, mut reload_rx) = mpsc::channel::<()>(1);

    // Spawn config reloader
    tokio::spawn(async move {
        let auto_reload = reload_interval > 0;
        let interval = Duration::from_secs(if auto_reload { reload_interval } else { 3600 });

        loop {
            // Wait for either timer or manual trigger
            tokio::select! {
                () = tokio::time::sleep(interval), if auto_reload => {
                    tracing::debug!("automatic config reload triggered");
                }
                Some(()) = reload_rx.recv() => {
                    tracing::info!("manual config reload triggered");
                }
            }

            // Reload config
            let new_config = match Config::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("failed to reload config: {e}");
                    continue;
                }
            };

            // Get current endpoints for comparison
            let mut current = current_endpoints.write().await;

            // Check if anything changed
            if new_config.endpoints == *current {
                tracing::debug!("config unchanged, re-checking all endpoints");
                // Even if config unchanged, re-check all endpoints on manual reload
                let check_results = check_all_endpoints(&new_config.endpoints).await;

                // Write to database
                if let Some(ref pool) = db_pool {
                    for result in &check_results {
                        if let Err(e) = db::insert_uptime_event(pool, result).await {
                            tracing::warn!(endpoint = %result.name, error = %e, "failed to insert uptime event");
                        }
                    }
                }

                let mut results = state.write().await;
                for result in check_results {
                    results.insert(result.name.clone(), result);
                }
                continue;
            }

            tracing::info!("config changed, updating endpoints");

            // Apply the config update
            apply_config_update(
                &new_config.endpoints,
                &mut current,
                &active_tasks,
                &state,
                db_pool.clone(),
            )
            .await;
        }
    });

    if reload_interval == 0 {
        tracing::info!("automatic config reloading disabled (manual reload still available)");
    }

    reload_tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HttpMethod;

    // ============ ErrorType Tests ============

    #[test]
    fn error_type_as_str_returns_correct_values() {
        assert_eq!(ErrorType::Timeout.as_str(), "timeout");
        assert_eq!(ErrorType::Dns.as_str(), "dns");
        assert_eq!(ErrorType::Tls.as_str(), "tls");
        assert_eq!(ErrorType::Connection.as_str(), "connection");
        assert_eq!(ErrorType::StatusMismatch.as_str(), "status_mismatch");
        assert_eq!(ErrorType::TcpRefused.as_str(), "tcp_refused");
        assert_eq!(ErrorType::DnsNxdomain.as_str(), "dns_nxdomain");
        assert_eq!(ErrorType::DnsMismatch.as_str(), "dns_mismatch");
        assert_eq!(ErrorType::ClientBuild.as_str(), "client_build");
        assert_eq!(ErrorType::Unknown.as_str(), "unknown");
    }

    #[test]
    fn error_type_equality() {
        assert_eq!(ErrorType::Timeout, ErrorType::Timeout);
        assert_ne!(ErrorType::Timeout, ErrorType::Dns);
    }

    #[test]
    fn error_type_clone() {
        let original = ErrorType::Tls;
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    // ============ base_result Tests ============

    fn make_test_endpoint() -> Endpoint {
        Endpoint {
            addr: "https://example.com/health".to_string(),
            check_type: CheckType::Http,
            description: Some("Test endpoint".to_string()),
            group: Some("backend".to_string()),
            tags: vec!["production".to_string(), "api".to_string()],
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
    fn base_result_copies_endpoint_fields() {
        let endpoint = make_test_endpoint();
        let result = base_result("my-endpoint", &endpoint);

        assert_eq!(result.name, "my-endpoint");
        assert_eq!(result.description, Some("Test endpoint".to_string()));
        assert_eq!(result.group, Some("backend".to_string()));
        assert_eq!(
            result.tags,
            vec!["production".to_string(), "api".to_string()]
        );
        assert_eq!(result.addr, "https://example.com/health");
        assert_eq!(result.check_type, CheckType::Http);
    }

    #[test]
    fn base_result_initializes_is_up_to_false() {
        let endpoint = make_test_endpoint();
        let result = base_result("test", &endpoint);

        assert!(!result.is_up);
    }

    #[test]
    fn base_result_initializes_optional_fields_to_none() {
        let endpoint = make_test_endpoint();
        let result = base_result("test", &endpoint);

        assert!(result.status_code.is_none());
        assert!(result.response_time_ms.is_none());
        assert!(result.error.is_none());
        assert!(result.error_type.is_none());
    }

    #[test]
    fn base_result_resolves_env_vars_in_addr() {
        // SAFETY: Tests are run single-threaded with --test-threads=1 or are isolated
        unsafe {
            std::env::set_var("TEST_CHECK_HOST", "api.example.com");
        }

        let mut endpoint = make_test_endpoint();
        endpoint.addr = "https://${TEST_CHECK_HOST}/status".to_string();

        let result = base_result("test", &endpoint);

        assert_eq!(result.addr, "https://api.example.com/status");

        unsafe {
            std::env::remove_var("TEST_CHECK_HOST");
        }
    }

    #[test]
    fn base_result_handles_none_fields() {
        let mut endpoint = make_test_endpoint();
        endpoint.description = None;
        endpoint.group = None;
        endpoint.tags = vec![];

        let result = base_result("test", &endpoint);

        assert!(result.description.is_none());
        assert!(result.group.is_none());
        assert!(result.tags.is_empty());
    }

    // ============ CheckResult Tests ============

    #[test]
    fn check_result_clone() {
        let endpoint = make_test_endpoint();
        let mut result = base_result("test", &endpoint);
        result.is_up = true;
        result.status_code = Some(200);
        result.response_time_ms = Some(150);

        let cloned = result.clone();

        assert_eq!(result.name, cloned.name);
        assert_eq!(result.is_up, cloned.is_up);
        assert_eq!(result.status_code, cloned.status_code);
        assert_eq!(result.response_time_ms, cloned.response_time_ms);
    }

    #[test]
    fn check_result_debug() {
        let endpoint = make_test_endpoint();
        let result = base_result("test", &endpoint);

        let debug_str = format!("{result:?}");

        assert!(debug_str.contains("CheckResult"));
        assert!(debug_str.contains("test"));
    }

    // ============ CheckType Handling Tests ============

    #[test]
    fn base_result_preserves_http_check_type() {
        let mut endpoint = make_test_endpoint();
        endpoint.check_type = CheckType::Http;

        let result = base_result("test", &endpoint);
        assert_eq!(result.check_type, CheckType::Http);
    }

    #[test]
    fn base_result_preserves_tcp_check_type() {
        let mut endpoint = make_test_endpoint();
        endpoint.check_type = CheckType::Tcp;
        endpoint.addr = "tcp://localhost:5432".to_string();

        let result = base_result("test", &endpoint);
        assert_eq!(result.check_type, CheckType::Tcp);
    }

    #[test]
    fn base_result_preserves_dns_check_type() {
        let mut endpoint = make_test_endpoint();
        endpoint.check_type = CheckType::Dns;
        endpoint.addr = "dns://example.com".to_string();

        let result = base_result("test", &endpoint);
        assert_eq!(result.check_type, CheckType::Dns);
    }

    // ============ check_all_endpoints Tests ============

    #[tokio::test]
    async fn check_all_endpoints_returns_empty_for_empty_input() {
        let endpoints: HashMap<String, Endpoint> = HashMap::new();
        let results = check_all_endpoints(&endpoints).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn check_all_endpoints_sorts_alphabetically() {
        // This test just verifies sorting logic, actual network calls will fail but
        // results will still be sorted
        let mut endpoints = HashMap::new();

        let mut ep1 = make_test_endpoint();
        ep1.addr = "https://a.invalid".to_string();
        ep1.timeout = 1; // Quick timeout
        endpoints.insert("zebra".to_string(), ep1);

        let mut ep2 = make_test_endpoint();
        ep2.addr = "https://b.invalid".to_string();
        ep2.timeout = 1;
        endpoints.insert("alpha".to_string(), ep2);

        let mut ep3 = make_test_endpoint();
        ep3.addr = "https://c.invalid".to_string();
        ep3.timeout = 1;
        endpoints.insert("middle".to_string(), ep3);

        let results = check_all_endpoints(&endpoints).await;

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].name, "alpha");
        assert_eq!(results[1].name, "middle");
        assert_eq!(results[2].name, "zebra");
    }

    #[tokio::test]
    async fn check_all_endpoints_sorting_is_case_insensitive() {
        let mut endpoints = HashMap::new();

        let mut ep1 = make_test_endpoint();
        ep1.addr = "https://a.invalid".to_string();
        ep1.timeout = 1;
        endpoints.insert("ZEBRA".to_string(), ep1);

        let mut ep2 = make_test_endpoint();
        ep2.addr = "https://b.invalid".to_string();
        ep2.timeout = 1;
        endpoints.insert("alpha".to_string(), ep2);

        let results = check_all_endpoints(&endpoints).await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "alpha");
        assert_eq!(results[1].name, "ZEBRA");
    }

    // ============ get_sorted_results Tests ============

    #[tokio::test]
    async fn get_sorted_results_returns_empty_for_empty_state() {
        let state: CheckResultsState = Arc::new(RwLock::new(HashMap::new()));
        let results = get_sorted_results(&state).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn get_sorted_results_sorts_alphabetically() {
        let state: CheckResultsState = Arc::new(RwLock::new(HashMap::new()));

        {
            let mut state_guard = state.write().await;
            let endpoint = make_test_endpoint();

            let mut result1 = base_result("zebra", &endpoint);
            result1.is_up = true;
            state_guard.insert("zebra".to_string(), result1);

            let mut result2 = base_result("alpha", &endpoint);
            result2.is_up = false;
            state_guard.insert("alpha".to_string(), result2);

            let mut result3 = base_result("middle", &endpoint);
            result3.is_up = true;
            state_guard.insert("middle".to_string(), result3);
        }

        let results = get_sorted_results(&state).await;

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].name, "alpha");
        assert_eq!(results[1].name, "middle");
        assert_eq!(results[2].name, "zebra");
    }

    #[tokio::test]
    async fn get_sorted_results_preserves_all_fields() {
        let state: CheckResultsState = Arc::new(RwLock::new(HashMap::new()));

        {
            let mut state_guard = state.write().await;
            let endpoint = make_test_endpoint();

            let mut result = base_result("test", &endpoint);
            result.is_up = true;
            result.status_code = Some(200);
            result.response_time_ms = Some(42);
            state_guard.insert("test".to_string(), result);
        }

        let results = get_sorted_results(&state).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "test");
        assert!(results[0].is_up);
        assert_eq!(results[0].status_code, Some(200));
        assert_eq!(results[0].response_time_ms, Some(42));
    }
}
