mod checker;
mod config;
mod db;
mod layout;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::Html,
    routing::get,
};
use color_eyre::eyre::{Context, Result};
use serde::Deserialize;
use sqlx::PgPool;
use tower::ServiceBuilder;
use tower_http::{
    ServiceBuilderExt,
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    request_id::MakeRequestUuid,
    services::ServeDir,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};
use tracing::Level;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::checker::{CheckResultsState, ReloadTrigger};
use crate::config::Config;
use crate::db::{BucketStatus, TimeRange};

/// Combined application state
#[derive(Clone)]
struct AppState {
    check_results: CheckResultsState,
    reload_trigger: ReloadTrigger,
    #[allow(dead_code)]
    db_pool: Option<PgPool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize error handling
    color_eyre::install()?;

    // Initialize tracing
    init_tracing();

    dotenvy::dotenv().ok();

    // Load configuration
    let config = Config::load("forge.toml")?;
    tracing::info!("loaded {} endpoints", config.endpoints.len());

    let db_pool = db::connect_from_env().await?;

    // Build middleware stack
    // Note: Layers wrap in reverse order - first added is outermost
    let middleware = ServiceBuilder::new()
        // Set a unique request id for each request
        .set_x_request_id(MakeRequestUuid)
        // Propagate request id to response
        .propagate_x_request_id()
        // Trace requests (outermost to capture full request lifecycle)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        // Catch panics and convert them to 500 responses
        .layer(CatchPanicLayer::new())
        // Compress responses
        .layer(CompressionLayer::new());

    // Create shared state for check results
    let check_results: CheckResultsState = Arc::default();

    // Perform initial check before starting server
    checker::initial_check(&config.endpoints, &check_results, db_pool.as_ref()).await;

    // Spawn background tasks (endpoint checkers + config reloader)
    let config_path = PathBuf::from("forge.toml");
    let reload_trigger = checker::spawn_background_tasks(
        config_path,
        config.clone(),
        check_results.clone(),
        db_pool.clone(),
    )
    .await;

    // Combined application state
    let app_state = AppState {
        check_results,
        reload_trigger,
        db_pool,
    };

    // Build router with shared state
    let app = Router::new()
        .route("/", get(index))
        .route("/status", get(status))
        .route("/reload", get(reload))
        .route("/health", get(health))
        .fallback_service(ServeDir::new("src/public"))
        .layer(middleware)
        .with_state(app_state);

    // Start server
    let listener = tokio::net::TcpListener::bind(config.server.addr)
        .await
        .wrap_err("failed to bind to address")?;

    tracing::info!("listening on {}", config.server.addr);

    axum::serve(listener, app).await.wrap_err("server error")?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .init();
}

/// Query parameters for status endpoints
#[derive(Debug, Deserialize)]
struct StatusQuery {
    #[serde(default)]
    range: Option<String>,
}

/// Get bucket statuses for all endpoints, or empty map if no database
async fn get_buckets(
    db_pool: Option<&PgPool>,
    endpoint_names: &[String],
    time_range: TimeRange,
) -> HashMap<String, Vec<BucketStatus>> {
    match db_pool {
        Some(pool) => match db::get_all_endpoint_buckets(pool, endpoint_names, time_range).await {
            Ok(buckets) => buckets,
            Err(e) => {
                tracing::warn!(error = %e, "failed to fetch bucket statuses");
                HashMap::new()
            }
        },
        None => HashMap::new(),
    }
}

async fn index(State(state): State<AppState>, Query(params): Query<StatusQuery>) -> Html<String> {
    let results = checker::get_sorted_results(&state.check_results).await;
    let time_range = params
        .range
        .as_deref()
        .map(TimeRange::from_str)
        .unwrap_or_default();

    let endpoint_names: Vec<String> = results.iter().map(|r| r.name.clone()).collect();
    let buckets = get_buckets(state.db_pool.as_ref(), &endpoint_names, time_range).await;

    Html(layout::dashboard(&results, &buckets, time_range).into_string())
}

/// Partial endpoint for htmx polling - returns only the status grid
async fn status(State(state): State<AppState>, Query(params): Query<StatusQuery>) -> Html<String> {
    let results = checker::get_sorted_results(&state.check_results).await;
    let time_range = params
        .range
        .as_deref()
        .map(TimeRange::from_str)
        .unwrap_or_default();

    let endpoint_names: Vec<String> = results.iter().map(|r| r.name.clone()).collect();
    let buckets = get_buckets(state.db_pool.as_ref(), &endpoint_names, time_range).await;

    Html(layout::status_grid_with_buckets(&results, &buckets, time_range).into_string())
}

/// Trigger config reload and re-check all endpoints
async fn reload(State(state): State<AppState>) -> StatusCode {
    if state.reload_trigger.send(()).await.is_ok() {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

async fn health() -> &'static str {
    "ok"
}
