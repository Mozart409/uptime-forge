use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use reqwest::Client;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use crate::config::{Config, Endpoint};

/// Shared state containing cached check results
pub type CheckResultsState = Arc<RwLock<HashMap<String, CheckResult>>>;

/// Shared state for active endpoint tasks (name -> cancellation token)
type ActiveTasks = Arc<RwLock<HashMap<String, CancellationToken>>>;

/// Channel sender for triggering config reload
pub type ReloadTrigger = mpsc::Sender<()>;

/// Result of checking an endpoint's availability
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: String,
    pub description: Option<String>,
    pub addr: String,
    pub is_up: bool,
    pub status_code: Option<u16>,
    pub response_time_ms: Option<u64>,
    pub error: Option<String>,
}

/// Check a single endpoint's availability
pub async fn check_endpoint(name: &str, endpoint: &Endpoint) -> CheckResult {
    let client = match Client::builder()
        .timeout(Duration::from_secs(endpoint.timeout))
        .danger_accept_invalid_certs(endpoint.skip_tls_verification)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return CheckResult {
                name: name.to_string(),
                description: endpoint.description.clone(),
                addr: endpoint.addr.clone(),
                is_up: false,
                status_code: None,
                response_time_ms: None,
                error: Some(format!("failed to build HTTP client: {e}")),
            };
        }
    };

    let start = std::time::Instant::now();

    match client.get(&endpoint.addr).send().await {
        Ok(response) => {
            let elapsed = start.elapsed().as_millis() as u64;
            let status = response.status().as_u16();
            let is_up = status == endpoint.expected_status;

            CheckResult {
                name: name.to_string(),
                description: endpoint.description.clone(),
                addr: endpoint.addr.clone(),
                is_up,
                status_code: Some(status),
                response_time_ms: Some(elapsed),
                error: if is_up {
                    None
                } else {
                    Some(format!(
                        "expected status {}, got {}",
                        endpoint.expected_status, status
                    ))
                },
            }
        }
        Err(e) => {
            let elapsed = start.elapsed().as_millis() as u64;
            CheckResult {
                name: name.to_string(),
                description: endpoint.description.clone(),
                addr: endpoint.addr.clone(),
                is_up: false,
                status_code: None,
                response_time_ms: Some(elapsed),
                error: Some(e.to_string()),
            }
        }
    }
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

            {
                let mut results = state.write().await;
                results.insert(name.clone(), result);
            }

            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = cancel_token.cancelled() => {
                    tracing::debug!(endpoint = %name, "endpoint checker cancelled");
                    break;
                }
            }
        }
    });
}

/// Perform initial check of all endpoints and populate state
pub async fn initial_check(endpoints: &HashMap<String, Endpoint>, state: &CheckResultsState) {
    tracing::info!("performing initial endpoint checks");

    let results = check_all_endpoints(endpoints).await;

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

        let mut results = state.write().await;
        for result in check_results {
            results.insert(result.name.clone(), result);
        }
    }

    // Update current endpoints
    *current_endpoints = new_endpoints.clone();
}

/// Start all endpoint checkers and return the active tasks tracker
async fn start_all_checkers(
    endpoints: &HashMap<String, Endpoint>,
    state: &CheckResultsState,
) -> ActiveTasks {
    let active_tasks: ActiveTasks = Default::default();

    for (name, endpoint) in endpoints {
        let cancel_token = CancellationToken::new();

        spawn_endpoint_checker(
            name.clone(),
            endpoint.clone(),
            Arc::clone(state),
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
) -> ReloadTrigger {
    let reload_interval = initial_config.server.reload_config_interval;

    // Start initial endpoint checkers
    let active_tasks = start_all_checkers(&initial_config.endpoints, &state).await;

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
                _ = tokio::time::sleep(interval), if auto_reload => {
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
                let mut results = state.write().await;
                for result in check_results {
                    results.insert(result.name.clone(), result);
                }
                continue;
            }

            tracing::info!("config changed, updating endpoints");

            // Apply the config update
            apply_config_update(&new_config.endpoints, &mut current, &active_tasks, &state).await;
        }
    });

    if reload_interval == 0 {
        tracing::info!("automatic config reloading disabled (manual reload still available)");
    }

    reload_tx
}
