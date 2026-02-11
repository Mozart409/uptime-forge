use std::time::Duration;

use chrono::{DateTime, Utc};
use color_eyre::eyre::{Context, Result};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use ulid::Ulid;

use crate::checker::CheckResult;

pub async fn connect_from_env() -> Result<Option<PgPool>> {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            tracing::info!("DATABASE_URL not set; database disabled");
            return Ok(None);
        }
        Err(err) => return Err(err).wrap_err("failed to read DATABASE_URL"),
    };

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .wrap_err("failed to connect to database")?;

    tracing::info!("database connection established");
    sqlx::migrate!()
        .run(&pool)
        .await
        .wrap_err("failed to run database migrations")?;

    tracing::info!("database migrated");

    Ok(Some(pool))
}

/// Generate a deterministic ULID from an endpoint name
/// This ensures the same endpoint always has the same ID
/// We use a hash-based approach to create a deterministic ULID from the name
pub fn endpoint_id_from_name(name: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let hash1 = hasher.finish();

    // Hash again with a different seed for the second 64 bits
    let mut hasher2 = DefaultHasher::new();
    "uptime-forge".hash(&mut hasher2);
    name.hash(&mut hasher2);
    let hash2 = hasher2.finish();

    // Combine the two hashes into a 128-bit value and create a ULID
    let combined = (u128::from(hash1) << 64) | u128::from(hash2);
    let ulid = Ulid::from(combined);

    ulid.to_string()
}

/// Insert a check result as an uptime event
pub async fn insert_uptime_event(pool: &PgPool, result: &CheckResult) -> Result<()> {
    let endpoint_id = endpoint_id_from_name(&result.name);
    let ts = Utc::now();
    let status_code = result.status_code.map(i32::from);
    let latency_ms = result
        .response_time_ms
        .map(|l| i32::try_from(l).unwrap_or(i32::MAX));
    let error_type = result
        .error_type
        .as_ref()
        .map(crate::checker::ErrorType::as_str);
    let error_message = result.error.as_deref();

    sqlx::query(
        r"
        INSERT INTO uptime_events (endpoint_id, ts, status_code, success, latency_ms, error_type, error_message)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ",
    )
    .bind(&endpoint_id)
    .bind(ts)
    .bind(status_code)
    .bind(result.is_up)
    .bind(latency_ms)
    .bind(error_type)
    .bind(error_message)
    .execute(pool)
    .await
    .wrap_err("failed to insert uptime event")?;

    Ok(())
}

/// Time range for querying uptime events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeRange {
    Minutes30,
    #[default]
    Hour1,
    Hours3,
    Hours8,
    Hours24,
    Days7,
    Days30,
}

impl TimeRange {
    pub fn from_str(s: &str) -> Self {
        match s {
            "30m" => TimeRange::Minutes30,
            "3h" => TimeRange::Hours3,
            "8h" => TimeRange::Hours8,
            "24h" => TimeRange::Hours24,
            "7d" => TimeRange::Days7,
            "30d" => TimeRange::Days30,
            _ => TimeRange::Hour1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TimeRange::Minutes30 => "30m",
            TimeRange::Hour1 => "1h",
            TimeRange::Hours3 => "3h",
            TimeRange::Hours8 => "8h",
            TimeRange::Hours24 => "24h",
            TimeRange::Days7 => "7d",
            TimeRange::Days30 => "30d",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TimeRange::Minutes30 => "30 minutes",
            TimeRange::Hour1 => "1 hour",
            TimeRange::Hours3 => "3 hours",
            TimeRange::Hours8 => "8 hours",
            TimeRange::Hours24 => "24 hours",
            TimeRange::Days7 => "7 days",
            TimeRange::Days30 => "30 days",
        }
    }

    /// Get the duration as `chrono::Duration`
    fn as_duration(self) -> chrono::Duration {
        match self {
            TimeRange::Minutes30 => chrono::Duration::minutes(30),
            TimeRange::Hour1 => chrono::Duration::hours(1),
            TimeRange::Hours3 => chrono::Duration::hours(3),
            TimeRange::Hours8 => chrono::Duration::hours(8),
            TimeRange::Hours24 => chrono::Duration::hours(24),
            TimeRange::Days7 => chrono::Duration::days(7),
            TimeRange::Days30 => chrono::Duration::days(30),
        }
    }

    /// Get all time range options
    pub fn all() -> &'static [TimeRange] {
        &[
            TimeRange::Minutes30,
            TimeRange::Hour1,
            TimeRange::Hours3,
            TimeRange::Hours8,
            TimeRange::Hours24,
            TimeRange::Days7,
            TimeRange::Days30,
        ]
    }
}

/// Status for a single time bucket in the status pills
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketStatus {
    /// All checks succeeded
    Green,
    /// Mix of success and failure
    Yellow,
    /// All checks failed
    Red,
    /// No data for this bucket
    Gray,
}

impl BucketStatus {
    pub fn css_class(self) -> &'static str {
        match self {
            BucketStatus::Green => "bg-green-500",
            BucketStatus::Yellow => "bg-yellow-500",
            BucketStatus::Red => "bg-red-500",
            BucketStatus::Gray => "bg-gray-300",
        }
    }
}

/// Event data for computing bucket statuses
#[derive(Debug, Clone)]
pub struct UptimeEvent {
    pub ts: DateTime<Utc>,
    pub success: bool,
}

/// Row type for uptime events query
#[derive(sqlx::FromRow)]
struct UptimeEventRow {
    ts: DateTime<Utc>,
    success: bool,
}

/// Get uptime events for an endpoint within a time range
pub async fn get_uptime_events(
    pool: &PgPool,
    endpoint_name: &str,
    range: TimeRange,
) -> Result<Vec<UptimeEvent>> {
    let endpoint_id = endpoint_id_from_name(endpoint_name);
    let since = Utc::now() - range.as_duration();

    let rows: Vec<UptimeEventRow> = sqlx::query_as(
        r"
        SELECT ts, success
        FROM uptime_events
        WHERE endpoint_id = $1 AND ts >= $2
        ORDER BY ts ASC
        ",
    )
    .bind(&endpoint_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .wrap_err("failed to fetch uptime events")?;

    Ok(rows
        .into_iter()
        .map(|r| UptimeEvent {
            ts: r.ts,
            success: r.success,
        })
        .collect())
}

/// Number of buckets to display in the status pills
pub const NUM_BUCKETS: usize = 30;

/// Compute bucket statuses for the status pills display
pub fn compute_bucket_statuses(events: &[UptimeEvent], range: TimeRange) -> Vec<BucketStatus> {
    let now = Utc::now();
    let total_duration = range.as_duration();
    let bucket_duration = total_duration / i32::try_from(NUM_BUCKETS).unwrap_or(30);

    let mut buckets = vec![BucketStatus::Gray; NUM_BUCKETS];

    for (i, bucket) in buckets.iter_mut().enumerate() {
        // Buckets go from oldest (index 0) to newest (index NUM_BUCKETS-1)
        let bucket_start = now - total_duration + bucket_duration * i32::try_from(i).unwrap_or(0);
        let bucket_end = bucket_start + bucket_duration;

        let bucket_events: Vec<_> = events
            .iter()
            .filter(|e| e.ts >= bucket_start && e.ts < bucket_end)
            .collect();

        if bucket_events.is_empty() {
            *bucket = BucketStatus::Gray;
        } else {
            let successes = bucket_events.iter().filter(|e| e.success).count();
            let total = bucket_events.len();

            *bucket = if successes == total {
                BucketStatus::Green
            } else if successes == 0 {
                BucketStatus::Red
            } else {
                BucketStatus::Yellow
            };
        }
    }

    buckets
}

/// Get bucket statuses for all endpoints
pub async fn get_all_endpoint_buckets(
    pool: &PgPool,
    endpoint_names: &[String],
    range: TimeRange,
) -> Result<std::collections::HashMap<String, Vec<BucketStatus>>> {
    let mut result = std::collections::HashMap::new();

    for name in endpoint_names {
        let events = get_uptime_events(pool, name, range).await?;
        let buckets = compute_bucket_statuses(&events, range);
        result.insert(name.clone(), buckets);
    }

    Ok(result)
}
