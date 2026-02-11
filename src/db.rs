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

#[cfg(test)]
mod tests {
    use super::*;

    // ============ endpoint_id_from_name Tests ============

    #[test]
    fn endpoint_id_from_name_is_deterministic() {
        let id1 = endpoint_id_from_name("my-endpoint");
        let id2 = endpoint_id_from_name("my-endpoint");
        assert_eq!(id1, id2);
    }

    #[test]
    fn endpoint_id_from_name_differs_for_different_names() {
        let id1 = endpoint_id_from_name("endpoint-a");
        let id2 = endpoint_id_from_name("endpoint-b");
        assert_ne!(id1, id2);
    }

    #[test]
    fn endpoint_id_from_name_produces_valid_ulid_format() {
        let id = endpoint_id_from_name("test-endpoint");
        // ULID is 26 characters, uppercase alphanumeric
        assert_eq!(id.len(), 26);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn endpoint_id_from_name_handles_special_characters() {
        // Should not panic and should produce valid ULIDs
        let id1 = endpoint_id_from_name("endpoint-with-dashes");
        let id2 = endpoint_id_from_name("endpoint_with_underscores");
        let id3 = endpoint_id_from_name("endpoint.with.dots");
        let id4 = endpoint_id_from_name("");

        assert_eq!(id1.len(), 26);
        assert_eq!(id2.len(), 26);
        assert_eq!(id3.len(), 26);
        assert_eq!(id4.len(), 26);
    }

    #[test]
    fn endpoint_id_from_name_is_case_sensitive() {
        let id_lower = endpoint_id_from_name("endpoint");
        let id_upper = endpoint_id_from_name("ENDPOINT");
        assert_ne!(id_lower, id_upper);
    }

    // ============ TimeRange Tests ============

    #[test]
    fn time_range_from_str_parses_valid_values() {
        assert_eq!(TimeRange::from_str("30m"), TimeRange::Minutes30);
        assert_eq!(TimeRange::from_str("1h"), TimeRange::Hour1);
        assert_eq!(TimeRange::from_str("3h"), TimeRange::Hours3);
        assert_eq!(TimeRange::from_str("8h"), TimeRange::Hours8);
        assert_eq!(TimeRange::from_str("24h"), TimeRange::Hours24);
        assert_eq!(TimeRange::from_str("7d"), TimeRange::Days7);
        assert_eq!(TimeRange::from_str("30d"), TimeRange::Days30);
    }

    #[test]
    fn time_range_from_str_defaults_to_hour1() {
        assert_eq!(TimeRange::from_str("invalid"), TimeRange::Hour1);
        assert_eq!(TimeRange::from_str(""), TimeRange::Hour1);
        assert_eq!(TimeRange::from_str("2h"), TimeRange::Hour1);
    }

    #[test]
    fn time_range_as_str_round_trips() {
        for range in TimeRange::all() {
            let s = range.as_str();
            let parsed = TimeRange::from_str(s);
            assert_eq!(*range, parsed, "Round trip failed for {range:?}");
        }
    }

    #[test]
    fn time_range_as_str_returns_expected_values() {
        assert_eq!(TimeRange::Minutes30.as_str(), "30m");
        assert_eq!(TimeRange::Hour1.as_str(), "1h");
        assert_eq!(TimeRange::Hours3.as_str(), "3h");
        assert_eq!(TimeRange::Hours8.as_str(), "8h");
        assert_eq!(TimeRange::Hours24.as_str(), "24h");
        assert_eq!(TimeRange::Days7.as_str(), "7d");
        assert_eq!(TimeRange::Days30.as_str(), "30d");
    }

    #[test]
    fn time_range_label_returns_human_readable() {
        assert_eq!(TimeRange::Minutes30.label(), "30 minutes");
        assert_eq!(TimeRange::Hour1.label(), "1 hour");
        assert_eq!(TimeRange::Hours3.label(), "3 hours");
        assert_eq!(TimeRange::Hours8.label(), "8 hours");
        assert_eq!(TimeRange::Hours24.label(), "24 hours");
        assert_eq!(TimeRange::Days7.label(), "7 days");
        assert_eq!(TimeRange::Days30.label(), "30 days");
    }

    #[test]
    fn time_range_as_duration_returns_correct_values() {
        assert_eq!(
            TimeRange::Minutes30.as_duration(),
            chrono::Duration::minutes(30)
        );
        assert_eq!(TimeRange::Hour1.as_duration(), chrono::Duration::hours(1));
        assert_eq!(TimeRange::Hours3.as_duration(), chrono::Duration::hours(3));
        assert_eq!(TimeRange::Hours8.as_duration(), chrono::Duration::hours(8));
        assert_eq!(
            TimeRange::Hours24.as_duration(),
            chrono::Duration::hours(24)
        );
        assert_eq!(TimeRange::Days7.as_duration(), chrono::Duration::days(7));
        assert_eq!(TimeRange::Days30.as_duration(), chrono::Duration::days(30));
    }

    #[test]
    fn time_range_all_returns_all_variants() {
        let all = TimeRange::all();
        assert_eq!(all.len(), 7);
        assert!(all.contains(&TimeRange::Minutes30));
        assert!(all.contains(&TimeRange::Hour1));
        assert!(all.contains(&TimeRange::Hours3));
        assert!(all.contains(&TimeRange::Hours8));
        assert!(all.contains(&TimeRange::Hours24));
        assert!(all.contains(&TimeRange::Days7));
        assert!(all.contains(&TimeRange::Days30));
    }

    #[test]
    fn time_range_default_is_hour1() {
        assert_eq!(TimeRange::default(), TimeRange::Hour1);
    }

    // ============ BucketStatus Tests ============

    #[test]
    fn bucket_status_css_class_returns_correct_classes() {
        assert_eq!(BucketStatus::Green.css_class(), "bg-green-500");
        assert_eq!(BucketStatus::Yellow.css_class(), "bg-yellow-500");
        assert_eq!(BucketStatus::Red.css_class(), "bg-red-500");
        assert_eq!(BucketStatus::Gray.css_class(), "bg-gray-300");
    }

    // ============ compute_bucket_statuses Tests ============

    #[test]
    fn compute_bucket_statuses_returns_all_gray_for_empty_events() {
        let events: Vec<UptimeEvent> = vec![];
        let buckets = compute_bucket_statuses(&events, TimeRange::Hour1);

        assert_eq!(buckets.len(), NUM_BUCKETS);
        assert!(buckets.iter().all(|b| *b == BucketStatus::Gray));
    }

    #[test]
    fn compute_bucket_statuses_returns_correct_bucket_count() {
        let events: Vec<UptimeEvent> = vec![];

        for range in TimeRange::all() {
            let buckets = compute_bucket_statuses(&events, *range);
            assert_eq!(
                buckets.len(),
                NUM_BUCKETS,
                "Bucket count mismatch for {range:?}"
            );
        }
    }

    #[test]
    fn compute_bucket_statuses_green_for_all_success() {
        let now = Utc::now();
        // Create events in the most recent bucket (all success)
        let events = vec![
            UptimeEvent {
                ts: now - chrono::Duration::minutes(1),
                success: true,
            },
            UptimeEvent {
                ts: now - chrono::Duration::minutes(2),
                success: true,
            },
        ];

        let buckets = compute_bucket_statuses(&events, TimeRange::Hour1);

        // The last bucket should be green
        assert_eq!(buckets[NUM_BUCKETS - 1], BucketStatus::Green);
    }

    #[test]
    fn compute_bucket_statuses_red_for_all_failures() {
        let now = Utc::now();
        // Create events in the most recent bucket (all failures)
        let events = vec![
            UptimeEvent {
                ts: now - chrono::Duration::minutes(1),
                success: false,
            },
            UptimeEvent {
                ts: now - chrono::Duration::minutes(2),
                success: false,
            },
        ];

        let buckets = compute_bucket_statuses(&events, TimeRange::Hour1);

        // The last bucket should be red
        assert_eq!(buckets[NUM_BUCKETS - 1], BucketStatus::Red);
    }

    #[test]
    fn compute_bucket_statuses_yellow_for_mixed_results() {
        let now = Utc::now();
        // For 1 hour with 30 buckets, each bucket is 2 minutes
        // Create two events very close together (within same bucket) with mixed results
        let events = vec![
            UptimeEvent {
                ts: now - chrono::Duration::seconds(10),
                success: true,
            },
            UptimeEvent {
                ts: now - chrono::Duration::seconds(20),
                success: false,
            },
        ];

        let buckets = compute_bucket_statuses(&events, TimeRange::Hour1);

        // The last bucket should be yellow (mixed success/failure in same bucket)
        assert_eq!(buckets[NUM_BUCKETS - 1], BucketStatus::Yellow);
    }

    #[test]
    fn compute_bucket_statuses_old_events_in_first_bucket() {
        let now = Utc::now();
        // Create events from ~59 minutes ago (should be in first bucket for 1h range)
        let events = vec![UptimeEvent {
            ts: now - chrono::Duration::minutes(59),
            success: true,
        }];

        let buckets = compute_bucket_statuses(&events, TimeRange::Hour1);

        // First bucket should be green, rest should be gray
        assert_eq!(buckets[0], BucketStatus::Green);
        // Later buckets should be gray (no events)
        assert!(buckets[NUM_BUCKETS - 1] == BucketStatus::Gray);
    }

    #[test]
    fn compute_bucket_statuses_events_outside_range_ignored() {
        let now = Utc::now();
        // Create events from 2 hours ago (outside 1h range)
        let events = vec![UptimeEvent {
            ts: now - chrono::Duration::hours(2),
            success: true,
        }];

        let buckets = compute_bucket_statuses(&events, TimeRange::Hour1);

        // All buckets should be gray since event is outside range
        assert!(buckets.iter().all(|b| *b == BucketStatus::Gray));
    }

    #[test]
    fn compute_bucket_statuses_single_event_determines_bucket() {
        let now = Utc::now();

        // Single success
        let success_event = vec![UptimeEvent {
            ts: now - chrono::Duration::seconds(30),
            success: true,
        }];
        let buckets = compute_bucket_statuses(&success_event, TimeRange::Hour1);
        assert_eq!(buckets[NUM_BUCKETS - 1], BucketStatus::Green);

        // Single failure
        let failure_event = vec![UptimeEvent {
            ts: now - chrono::Duration::seconds(30),
            success: false,
        }];
        let buckets = compute_bucket_statuses(&failure_event, TimeRange::Hour1);
        assert_eq!(buckets[NUM_BUCKETS - 1], BucketStatus::Red);
    }

    #[test]
    fn compute_bucket_statuses_multiple_buckets_with_events() {
        let now = Utc::now();

        // For 30m range, each bucket is 1 minute
        // Create events spread across multiple buckets
        let events = vec![
            // Recent bucket (success)
            UptimeEvent {
                ts: now - chrono::Duration::seconds(30),
                success: true,
            },
            // ~15 minutes ago (failure)
            UptimeEvent {
                ts: now - chrono::Duration::minutes(15),
                success: false,
            },
            // ~28 minutes ago (mixed)
            UptimeEvent {
                ts: now - chrono::Duration::minutes(28),
                success: true,
            },
            UptimeEvent {
                ts: now - chrono::Duration::minutes(28) + chrono::Duration::seconds(10),
                success: false,
            },
        ];

        let buckets = compute_bucket_statuses(&events, TimeRange::Minutes30);

        // Last bucket should be green
        assert_eq!(buckets[NUM_BUCKETS - 1], BucketStatus::Green);
        // We should have at least one non-gray bucket besides the last one
        let non_gray_count = buckets.iter().filter(|b| **b != BucketStatus::Gray).count();
        assert!(non_gray_count >= 2);
    }

    #[test]
    fn compute_bucket_statuses_handles_many_events() {
        let now = Utc::now();

        // Create 100 events spread over the hour
        let events: Vec<UptimeEvent> = (0..100)
            .map(|i| UptimeEvent {
                ts: now - chrono::Duration::seconds(i * 36), // Spread over 3600 seconds
                success: i % 2 == 0,                         // Alternate success/failure
            })
            .collect();

        let buckets = compute_bucket_statuses(&events, TimeRange::Hour1);

        assert_eq!(buckets.len(), NUM_BUCKETS);
        // Most buckets should have events (yellow due to mixed)
        let non_gray = buckets.iter().filter(|b| **b != BucketStatus::Gray).count();
        assert!(non_gray > 0);
    }
}
